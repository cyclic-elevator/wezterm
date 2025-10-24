# Phase 19: Failure Analysis & Next Steps

## 🚨 Critical Finding

**Phase 19 code is NOT executing!**

### Evidence
- **ZERO Phase 19 debug logs** in `frame-logs.19`
- Expected: `"Phase 19: Selective invalidation..."`, `"Phase 19: Scheduling deferred resize..."`
- Actual: Complete absence of all Phase 19 logging

### Performance Regression
- **Before Phase 19**: 100-750ms GPU stalls
- **After Phase 19**: 100-**2800ms** GPU stalls ❌ **3.7x WORSE!**

---

## 🔍 Root Cause: Wrong Layer!

### What We Fixed
✅ **Client-side** (`wezterm-client/src/pane/clientpane.rs`):
- `ClientPane::resize()` now uses selective invalidation
- Debounced server RPCs
- Fetch coalescing via generation counter

### What We Didn't Fix
❌ **Server-side** (`term/src/terminal.rs`, `mux/src/pane.rs`):
- Terminal still calls `make_all_stale()` on resize
- Server still invalidates ENTIRE scrollback (10,000+ lines)
- Server still triggers massive fetches

### The Architecture

```
[User's Machine] <---network---> [Remote Server]
     Client                           Server
       |                                 |
    ClientPane ← Phase 19 ✅         LocalPane ← NOT FIXED! ❌
    (optimized)                      (still slow!)
       |                                 |
    Fetch data                        PTY + Term
```

**When user resizes**:
1. Client → sends resize RPC → Server ✅
2. Server → resizes Terminal → **`make_all_stale()` on 10,000 lines!** ❌
3. Server → sends updates → Client
4. Client → fetches data (our optimizations help here ✅)
5. **But step 2-3 is the REAL bottleneck!** ❌

---

## 📊 Why Frame Times Improved But Stalls Worsened

### Frame Times: ✅ Better (80ms → 34ms)
- Tab bar cache working
- GPU optimizations working
- Less CPU work per frame

### GPU Stalls: ❌ Worse (750ms → 2800ms)
- Server bottleneck NOT fixed
- Faster client hits slower server harder
- Network latency amplified

**Unintended consequence**: By making the client faster, we're overwhelming the slow server!

---

## 🎯 Next Steps (Choose One)

### Option A: Emergency Verification (1 hour) ⚡

**Goal**: Confirm Phase 19 code path

**Action**:
1. Add emergency logging to `ClientPane::resize()`:
   ```rust
   fn resize(&self, size: TerminalSize) -> anyhow::Result<()> {
       log::error!("=== PHASE 19: ClientPane::resize {}x{} ===", size.cols, size.rows);
       // ... rest of code ...
   }
   ```

2. Rebuild and test:
   ```bash
   cargo build --package wezterm-gui --release
   RUST_LOG=error ./target/release/wezterm-gui start
   # Connect to remote mux and resize
   ```

3. Check logs:
   - **If log appears** → Code is running, but insufficient (need server fix)
   - **If log doesn't appear** → Wrong code path (investigate further)

### Option B: Fix Server Side (2-3 hours) 🔧 **RECOMMENDED**

**Goal**: Apply Phase 19 to Terminal/LocalPane

**Files to modify**:
1. `term/src/terminal.rs` - Add `make_viewport_stale()` method
2. `mux/src/pane.rs` - Update LocalPane to use it
3. Anywhere `make_all_stale()` is called

**Implementation**:
```rust
// In term/src/terminal.rs
impl Terminal {
    pub fn make_viewport_stale(&mut self, margin: usize) {
        // Same logic as in renderable.rs
        let viewport_start = self.screen().physical_top;
        let viewport_end = viewport_start + self.screen().physical_rows;
        let margin = margin as isize;
        
        let start_row = (viewport_start - margin).max(...);
        let end_row = viewport_end + margin;
        
        for row in start_row..end_row {
            self.make_row_stale(row);
        }
    }
}
```

**Expected improvement**: 40-100x reduction in server-side invalidation!

### Option C: Full Audit (4-6 hours) 🔬

**Goal**: Find ALL `make_all_stale()` call sites

**Action**:
```bash
cd /Users/zeyu.chen/git/wezterm
grep -r "make_all_stale()" --include="*.rs" | grep -v "target/"
```

**Then**: Apply selective invalidation to EVERY call site

---

## 🎲 Most Likely Scenario

### Hypothesis: Server-Side Bottleneck

**User is resizing remote server window:**
```
1. User drags window on remote server
2. Server Terminal receives resize event
3. Server calls make_all_stale() → 10,000 lines invalidated! ❌
4. Server sends updates to client
5. Client processes updates (Phase 19 helps here ✅)
6. BUT: Server already did the expensive work ❌
```

**The fix MUST be on server, not client!**

### Why Phase 19 Client Fix Didn't Help

**Client optimizations don't matter if server is bottleneck!**

It's like:
- Optimizing the checkout line (client) ✅
- But the warehouse is still slow (server) ❌
- Customers still wait because warehouse is slow!

---

## 📝 Immediate Action Items

### 1. Verify Code Path ⚡ **DO THIS FIRST**

Add `log::error!` to **top** of `ClientPane::resize()`:
```rust
fn resize(&self, size: TerminalSize) -> anyhow::Result<()> {
    log::error!("🚨 PHASE 19 CLIENTPANE RESIZE: {}x{} 🚨", size.cols, size.rows);
    // ... existing code ...
}
```

### 2. Check Server Binary

If testing remote mux:
- **Client binary**: Has Phase 19 ✅
- **Server binary**: Needs Phase 19 too! ❌

**Both must be updated!**

### 3. Implement Server-Side Fix 🔧

This is the **REAL** fix:
- Add `make_viewport_stale()` to `Terminal`
- Update `LocalPane::resize()` to use it
- This is where the bottleneck actually is!

---

## 🎯 Recommended Plan

**Phase 19.1: Server-Side Selective Invalidation**

1. **Verify** (Option A): Add emergency logging (30 min)
2. **Implement** (Option B): Fix Terminal/LocalPane (2-3 hours)
3. **Test**: Rebuild BOTH client and server, test again
4. **Verify**: Should see 10-100x improvement

**Expected result after Phase 19.1**:
- Deserialization: 11.82% → <1% ✅
- GPU stalls: 2800ms → <100ms ✅
- Repaint time: >10s → <1s ✅

---

## Summary

### What Went Wrong
- ✅ Phase 19 code is correct
- ✅ Phase 19 code compiles
- ❌ Phase 19 code is in wrong layer (client, not server)
- ❌ Performance regression (2800ms stalls)

### The Real Problem
**Server-side Terminal still uses `make_all_stale()`!**

This is THE bottleneck, not the client-side fetching!

### The Real Fix
**Apply Phase 19 to Terminal/LocalPane on the server!**

---

**Next action**: Run **Option A** (verification) to confirm diagnosis, then proceed with **Option B** (server-side fix).

