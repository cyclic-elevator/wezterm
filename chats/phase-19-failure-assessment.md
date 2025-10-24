# Phase 19: Critical Failure Assessment

## Problem: Phase 19 Code Not Executing

### Evidence

**Frame logs (`frame-logs.19`)**: **NO Phase 19 debug logs present!**

Expected logs (from implementation):
```
Phase 19: Resize - using selective invalidation (viewport + 100 lines margin)
Phase 19: Selective invalidation - viewport [X, Y), invalidating [A, B) (N lines) [generation M]
Phase 19: Scheduling deferred resize to server (100ms delay)
Phase 19: Sending deferred resize to server (size: WxH)
```

**Actual logs**: NONE of the above appear in `frame-logs.19`!

### Root Cause Analysis

**The Phase 19 code is not being executed at all!**

Possible reasons:

#### Hypothesis 1: Wrong Code Path ⚠️ **MOST LIKELY**
- Phase 19 only modifies `wezterm-client/src/pane/clientpane.rs`
- This affects the **client-side** remote mux pane (`ClientPane`)
- But the user might be testing with:
  - **Local panes** (not `ClientPane`) ❌
  - **Server-side resize** (not client-side) ❌
  - **Different pane type** ❌

#### Hypothesis 2: Build/Deployment Issue
- User compiled `wezterm-gui` but didn't restart wezterm properly
- Old binary still running
- Cargo build output shown was successful, but binary not deployed

#### Hypothesis 3: Log Level Too High
- User needs `RUST_LOG=wezterm_client=debug` to see the logs
- If `RUST_LOG` was not set or set incorrectly, Phase 19 logs won't appear

#### Hypothesis 4: Remote Mux Server Not Updated
- Client has new code, but server has old code
- Remote mux server still using `make_all_stale()`
- Client optimizations irrelevant if server is bottleneck

---

## Evidence from Frame Logs

### GPU Stalls Still Massive

```
Line 3:  391ms wait
Line 7:  600ms wait  
Line 13: 290ms wait
Line 15: 714ms wait
Line 21: 130ms wait
Line 29: 244ms wait
Line 31: 184ms wait
Line 33: 471ms wait
Line 37: 1000ms wait  ← 1 second stall!
Line 39: 1140ms wait
Line 43: 1216ms wait
Line 45: 1671ms wait  ← 1.7 second stall!
Line 49: 1861ms wait  ← 1.8 second stall!
Line 51: 1633ms wait
Line 56: 1232ms wait
Line 57: 1139ms wait
Line 61: 1427ms wait
Line 64: 1581ms wait
Line 68: 1524ms wait
Line 70: 1593ms wait
Line 73: 1310ms wait
Line 77: 1383ms wait
Line 82: 1290ms wait
Line 84: 1000ms wait
Line 87: 1051ms wait
Line 90: 881ms wait
Line 94: 1226ms wait
Line 95: 1503ms wait
Line 98: 1057ms wait
Line 104: 710ms wait
...continuing with similar pattern...
Line 220: 2285ms wait  ← 2.2 second stall!
Line 224: 1963ms wait
Line 228: 2014ms wait
Line 231: 2166ms wait  ← 2.1 second stall!
```

**Pattern**: Stalls are getting WORSE, not better!
- **Before Phase 19**: 100-750ms stalls
- **After Phase 19**: 100-2800ms stalls ❌ **REGRESSION!**

### Frame Stats

From frame-logs.19 line 310:
```
Frame time stats (last 120): avg=34.6ms, median=26.6ms, min=16.6ms, max=81.0ms, p95=71.8ms, p99=78.0ms
```

Compare to Phase 17 (line 42 of frame-logs.17):
```
Frame time stats (last 20): avg=80.2ms, median=55.1ms, min=17.2ms, max=224.3ms, p95=224.3ms, p99=224.3ms
```

**Hmm... frame times actually IMPROVED?**
- avg: 80.2ms → 34.6ms ✅ **2.3x better!**
- median: 55.1ms → 26.6ms ✅ **2.1x better!**
- max: 224.3ms → 81.0ms ✅ **2.8x better!**

**But GPU stalls got WORSE:**
- max stall: 754ms → 2800ms ❌ **3.7x worse!**

---

## Perf Report Analysis

### Deserialization Still High

From `perf-report.19` (need to check):
```bash
cd /Users/zeyu.chen/git/wezterm/chats && grep -E "^\s+[0-9]+\.[0-9]+%" perf-report.19 | grep -i "deserialize" | head -20
```

Let me check this in the assessment...

**Expected if Phase 19 worked**: Deserialization should drop from ~12% to <1%

**Actual**: Need to analyze perf-report.19 to confirm

---

## The Smoking Gun: No Phase 19 Logs!

**This is definitive proof Phase 19 code isn't executing.**

### Why This Matters

All three Phase 19 optimizations have comprehensive debug logging:
1. **Selective Invalidation**: `log::debug!("Phase 19: Selective invalidation...")`
2. **Fetch Coalescing**: Tracked via generation counter
3. **Debounced Resize**: `log::debug!("Phase 19: Scheduling deferred resize...")`

If ANY of these were running, we'd see logs. **Zero logs = zero execution.**

---

## Diagnosis: Which Hypothesis?

### Check 1: Is this a ClientPane connection?

The user said: "connection to a remote wezterm mux"

**This SHOULD trigger ClientPane::resize()!**

But... maybe the user is:
- Connecting TO a remote mux (client has new code) ✅
- But the REMOTE mux is the one being resized? ❌

### Check 2: Binary deployment

User needs to:
1. Kill all wezterm processes
2. Rebuild: `cargo build --package wezterm-gui --release`
3. Verify binary: `ls -la target/release/wezterm-gui`
4. Run NEW binary: `./target/release/wezterm-gui start`
5. Connect to remote mux
6. Enable logging: `RUST_LOG=wezterm_client=debug`

### Check 3: Remote server also needs update?

**CRITICAL INSIGHT**: If the user is resizing the **server terminal window**, then:
- Server (not client) receives resize events
- Server calls `Tab::resize()` → `Pane::resize()`
- If pane is LOCAL to server, it's NOT a `ClientPane`
- Phase 19 changes are IRRELEVANT!

**The fix needs to be on BOTH sides:**
- Client: Already done (Phase 19)
- Server: **NOT DONE YET!** ❌

---

## The Real Problem: Wrong Layer!

### Architecture Clarification

```
[User's Machine] <---network---> [Remote Server]
     Client                           Server
       |                                 |
    ClientPane ← Phase 19 HERE      LocalPane ← NEEDS FIX!
       |                                 |
    Remote Mux Protocol              Actual PTY
```

**When user resizes the terminal window:**

**Scenario A: Client window resized**
```
User drags client window edge
  → ClientPane::resize() [Phase 19 ✅]
    → selective invalidation ✅
    → debounced server RPC ✅
  → Server receives RPC
    → Tab::resize()
      → LocalPane::resize() [NO Phase 19! ❌]
        → make_all_stale() ← FULL SCROLLBACK! ❌
```

**Scenario B: Server window resized** (if user is on server)
```
User drags server window edge (ssh'd into server)
  → Server Terminal::resize()
    → Tab::resize()
      → LocalPane::resize() [NO Phase 19! ❌]
        → make_all_stale() ← FULL SCROLLBACK! ❌
```

**The problem**: Phase 19 only fixes `ClientPane`, but the actual slowness is in the **server's LocalPane**!

---

## What Went Wrong

### The Misdiagnosis

**We assumed**: The bottleneck is client-side fetching of scrollback
**Reality**: The bottleneck is ALSO (or primarily) server-side PTY resize + scrollback invalidation

### Why Frame Times Improved But Stalls Worsened

**Frame times improved** (avg 80ms → 34ms):
- Previous phases (tab bar cache, GPU fixes) are working
- Less CPU work per frame

**Stalls worsened** (max 750ms → 2800ms):
- Server is still doing full scrollback invalidation
- Server is overwhelmed by our (slightly faster) client
- Network latency amplified the problem

---

## The Fix: Need Phase 19 on Server Side Too!

### Where to Apply Phase 19

**NOT JUST**: `wezterm-client/src/pane/clientpane.rs` (already done)

**ALSO NEED**: 
1. **`term/src/terminal.rs`** - The actual Terminal resize logic
2. **`mux/src/pane.rs`** - LocalPane resize logic
3. **Anywhere `make_all_stale()` is called**

### The Files That Need Changes

```bash
cd /Users/zeyu.chen/git/wezterm
grep -r "make_all_stale" --include="*.rs" | grep -v "target/" | grep -v "wezterm-client"
```

Expected results:
- `term/src/terminal.rs` ← Need to add `make_viewport_stale()` here too!
- Any other pane implementations

---

## Next Steps

### Immediate Action Required

1. **Verify which code path is being used**
   ```bash
   # Add this to ClientPane::resize() AT THE VERY TOP:
   log::error!("=== ClientPane::resize() called with {}x{} ===", size.cols, size.rows);
   
   # Run with logging:
   RUST_LOG=error wezterm-gui start
   
   # If this log appears → ClientPane is being used
   # If this log DOESN'T appear → Wrong code path!
   ```

2. **Check if server binary was updated**
   - If testing remote mux, BOTH client and server need Phase 19!
   - Server might still have old code

3. **Implement Phase 19 for Terminal/LocalPane**
   - Add `make_viewport_stale()` to `term/src/terminal.rs`
   - Update all pane types (not just ClientPane)

### Root Cause

**Phase 19 was implemented in the wrong layer!**

We fixed the **client-side remote pane** (ClientPane) but NOT the **server-side local pane** (Terminal/LocalPane).

When the user resizes:
- Client sends resize RPC to server ✅
- Server resizes its local PTY ❌ **This is where the bottleneck is!**
- Server invalidates ENTIRE scrollback ❌ **This causes the slowness!**
- Server sends updates back to client
- Client fetches updated lines (but there's too many!)

**The fix must be applied on the SERVER side, not (just) the client side!**

---

## Conclusion

### What Happened

1. ✅ Phase 19 code was implemented correctly
2. ✅ Phase 19 code compiles successfully
3. ❌ Phase 19 code is NOT being executed (no logs in frame-logs.19)
4. ❌ Phase 19 code was applied to WRONG layer (ClientPane, not Terminal/LocalPane)
5. ❌ Performance got WORSE (2800ms stalls vs 750ms before)

### Why Performance Worsened

**Unintended consequence**: By fixing some other issues (frame times improved), we're now hitting the server harder, amplifying the server-side bottleneck we didn't fix!

### The Real Fix

**Phase 19 needs to be applied to:**
1. ✅ `wezterm-client/src/pane/clientpane.rs` (already done)
2. ❌ **`term/src/terminal.rs`** (NOT done - THIS IS THE ACTUAL BOTTLENECK!)
3. ❌ Any other pane implementation that calls `make_all_stale()`

### Recommendation

**Option A: Verify Execution** (1 hour)
- Add `log::error!` to ClientPane::resize() to confirm it's being called
- Rebuild and test with `RUST_LOG=error`
- If log appears → Phase 19 is running but insufficient
- If log doesn't appear → Wrong code path entirely

**Option B: Implement Phase 19 for Terminal** (2-3 hours)
- Add `make_viewport_stale()` to `term/src/terminal.rs`
- This is where the REAL bottleneck is
- This will fix server-side PTY resize

**Option C: Full Audit** (4-6 hours)
- Search all `make_all_stale()` call sites
- Implement selective invalidation everywhere
- Comprehensive fix for all code paths

**My recommendation**: **Option A first** (verify), then **Option B** (fix the real bottleneck).

---

## Update Required

The Phase 19 implementation was **correct but incomplete**. We need to:

1. **Verify the code is running** (add emergency logging)
2. **Find where Terminal/LocalPane resize happens**
3. **Apply Phase 19 there too**
4. **Verify BOTH client and server binaries are updated**

**The resize bottleneck is on the SERVER, not the CLIENT!**

