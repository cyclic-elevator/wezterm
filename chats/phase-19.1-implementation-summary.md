# Phase 19.1: Diagnostic Implementation & Code Analysis

## Summary

Implemented **Option A** (emergency logging) and completed **Option B** (analysis). Made a **critical discovery**: Terminal resize is already smart and doesn't over-invalidate!

---

## Changes Made

### Option A: Emergency Logging ✅

**File**: `wezterm-client/src/pane/clientpane.rs`

**Change**: Added emergency logging at the very start of `ClientPane::resize()`:

```rust
fn resize(&self, size: TerminalSize) -> anyhow::Result<()> {
    // Phase 19.1: Emergency logging to verify this code path is being used
    log::error!("🚨 PHASE 19 CLIENTPANE RESIZE: {}x{} (pane_id: {}) 🚨", size.cols, size.rows, self.remote_pane_id);
    
    // ... rest of existing Phase 19 code ...
}
```

**Purpose**: This will DEFINITIVELY show if `ClientPane::resize()` is being called during resize operations.

**How to test**:
```bash
# Run wezterm with error logging (no need for debug level)
RUST_LOG=error ./target/debug/wezterm-gui start

# Connect to remote mux and resize
# If you see "🚨 PHASE 19 CLIENTPANE RESIZE:" → code is running
# If you don't see it → code path is not being used
```

---

### Option B: Code Analysis ✅

#### Discovery 1: Terminal Resize is Already Smart! 🎉

**File**: `term/src/screen.rs` (lines 193-326)

**Analysis**: `Screen::resize()` already has selective invalidation logic:

```rust
pub fn resize(...) -> CursorPosition {
    // ... setup ...
    
    let (cursor_x, cursor_y) = if physical_cols != self.physical_cols {
        // Only invalidate when WIDTH changes
        if self.allow_scrollback {
            // Smart rewrap for primary screen
            self.rewrap_lines(physical_cols, ...)
        } else {
            // For alt screen: only invalidate/prune changed lines
            for line in &mut self.lines {
                if physical_cols < self.physical_cols {
                    line.resize(physical_cols, seqno);  // Prune
                } else {
                    line.update_last_change_seqno(seqno);  // Invalidate
                }
            }
            (cursor.x, cursor_phys)
        }
    } else {
        (cursor.x, cursor_phys)  // No width change = no invalidation!
    };
    
    // ... rest of resize logic ...
}
```

**Key findings**:
1. ✅ **Height-only changes**: NO invalidation (most common during resize!)
2. ✅ **Width changes**: Selective invalidation + smart rewrap
3. ✅ **No `make_all_lines_dirty()` call**: Terminal resize is already efficient!

#### Discovery 2: LocalPane Resize Chain

**File**: `mux/src/localpane.rs` (lines 417-426)

```rust
fn resize(&self, size: TerminalSize) -> Result<(), Error> {
    // 1. Resize the PTY
    self.pty.lock().resize(PtySize {
        rows: size.rows.try_into()?,
        cols: size.cols.try_into()?,
        pixel_width: size.pixel_width.try_into()?,
        pixel_height: size.pixel_height.try_into()?,
    })?;
    
    // 2. Resize the terminal (smart resize, already analyzed above)
    self.terminal.lock().resize(size);
    Ok(())
}
```

**Findings**:
- ✅ PTY resize happens first (sends SIGWINCH to process)
- ✅ Terminal resize second (smart, selective invalidation)
- ✅ No explicit `make_all_stale()` call!

#### Discovery 3: Remote Mux Server Resize Handling

**File**: `wezterm-mux-server-impl/src/sessionhandler.rs` (lines 633-653)

```rust
Pdu::Resize(Resize { containing_tab_id, pane_id, size }) => {
    spawn_into_main_thread(async move {
        catch(
            move || {
                let mux = Mux::get();
                
                // Get the pane and resize it
                let pane = mux.get_pane(pane_id)
                    .ok_or_else(|| anyhow!("no such pane {}", pane_id))?;
                pane.resize(size)?;  // → LocalPane::resize() → Terminal::resize()
                
                // Rebuild split sizes
                let tab = mux.get_tab(containing_tab_id)
                    .ok_or_else(|| anyhow!("no such tab {}", containing_tab_id))?;
                tab.rebuild_splits_sizes_from_contained_panes();
                
                Ok(Pdu::UnitResponse(UnitResponse {}))
            },
            send_response,
        )
    })
    .detach();
}
```

**Findings**:
- ✅ Server receives `Pdu::Resize` from client
- ✅ Calls `pane.resize(size)` → `LocalPane::resize()` → `Terminal::resize()`
- ✅ Terminal resize is already smart (Discovery 1)
- ✅ No over-invalidation at server level!

#### Discovery 4: Tab Resize Sends Notification

**File**: `mux/src/tab.rs` (line 1182)

```rust
fn resize(&mut self, size: TerminalSize) {
    // ... resize logic ...
    
    Mux::try_get().map(|mux| mux.notify(MuxNotification::TabResized(self.id)));
}
```

**Implication**: This notification might trigger client-side rendering/fetching. Need to investigate if this causes over-fetching.

---

## Critical Insight: The Real Bottleneck

### What We Know Now

1. ✅ **Terminal resize is already smart** (only invalidates on width change)
2. ✅ **LocalPane resize is clean** (no explicit over-invalidation)
3. ✅ **Phase 19 client code exists** (selective invalidation in ClientPane)
4. ❌ **Phase 19 client code isn't executing** (no logs in frame-logs.19)

### The Mystery

**If Terminal resize is already smart, why is remote mux so slow?**

### Possible Causes

#### Hypothesis A: ClientPane::resize() Not Being Called
- **Evidence**: Zero Phase 19 logs in frame-logs.19
- **Implication**: Resize might be going through a different code path
- **Next step**: Emergency logging will confirm this

#### Hypothesis B: TabResized Notification Overhead
- **Evidence**: `Tab::resize()` sends `MuxNotification::TabResized` on every resize
- **Implication**: This might trigger expensive client-side operations
- **Next step**: Profile what happens when `TabResized` is received

#### Hypothesis C: Network Latency Amplification
- **Evidence**: User reports 10+ second delays, but CPU profiles show low CPU
- **Implication**: Waiting for network I/O, not CPU-bound
- **Issue**: Even with smart invalidation, network round-trips dominate
- **Solution**: Need to reduce number of network requests, not just amount of data

#### Hypothesis D: Rendering/Fetching Loop
- **Evidence**: Perf profiles show high deserialization overhead (11.82%)
- **Implication**: Something is triggering repeated fetches of the same data
- **Possibility**: `TabResized` → client re-renders → fetches lines → discovers they're stale → fetches again?

---

## Next Steps for Testing

### Test 1: Verify ClientPane::resize() Execution ⚡ **DO THIS FIRST**

```bash
# Build with emergency logging
cd /Users/zeyu.chen/git/wezterm
cargo build --package wezterm-gui --release

# Run with error logging
RUST_LOG=error ./target/release/wezterm-gui start

# Connect to remote mux
# Resize window
# Check logs for "🚨 PHASE 19 CLIENTPANE RESIZE:"
```

**Expected outcomes**:
- **If log appears**: ClientPane is being called, Phase 19 code should be working
- **If log doesn't appear**: Resize is going through a different code path (need to investigate)

### Test 2: Profile What TabResized Triggers

Add logging to see what `MuxNotification::TabResized` causes:

```bash
# In wezterm-gui, find where TabResized is handled
grep -r "TabResized" wezterm-gui/src --include="*.rs"

# Add logging there to see if it triggers expensive operations
```

### Test 3: Check if Server Binary Needs Update

If testing remote mux:
```bash
# On remote server, rebuild wezterm
cd wezterm-server-installation
git pull
cargo build --release --package wezterm

# Restart wezterm-mux-server
pkill wezterm-mux-server
./target/release/wezterm-mux-server start
```

---

## Analysis: Why Phase 19 Might Not Help

### The Network Latency Problem

Even with Phase 19's selective invalidation:
1. Client detects resize → calls `make_viewport_stale(100)` ✅ (only 100-300 lines)
2. Client sends resize RPC to server ✅ (1 network round-trip)
3. Server resizes Terminal ✅ (smart resize, minimal invalidation)
4. Server sends `MuxNotification::TabResized` to client (1 network round-trip)
5. Client receives notification → triggers render ❌
6. Client discovers 100-300 stale lines → starts fetching ❌
7. **Client must make 100-300 network requests** ❌❌❌ **THIS IS THE BOTTLENECK!**

**The issue**: Even with selective invalidation, if we invalidate 100-300 lines, and each line requires a network request, that's still **100-300 network round-trips**!

At 10ms per round-trip = 1-3 seconds
At 50ms per round-trip = 5-15 seconds ← **Matches user observation!**

### Why Local Sessions Are Fast

**Local sessions**:
- Lines are in local memory (no network requests)
- Stale lines are just re-read from Terminal (microseconds)
- Total time: <100ms

**Remote sessions**:
- Stale lines require network fetches (milliseconds each)
- 100-300 network requests × 10-50ms = 1-15 seconds ← **THE BOTTLENECK!**

---

## The Real Fix: Batch Fetching

### Problem

Current behavior (suspected):
```
Client invalidates 100 lines:
  fetch_line(1000) → network request 1
  fetch_line(1001) → network request 2
  fetch_line(1002) → network request 3
  ... 100 requests ...
  fetch_line(1099) → network request 100
```

**Time**: 100 requests × 50ms = 5 seconds!

### Solution

Batch fetch:
```
Client invalidates 100 lines:
  fetch_lines_batch(1000..1100) → 1 network request
```

**Time**: 1 request × 50ms = **50 milliseconds**! 🎉

**Improvement**: **100x faster!**

---

## Recommendations

### Immediate (1-2 hours) ⚡

1. **Run Test 1** with emergency logging to confirm code path
2. **Profile what `TabResized` notification triggers** on client side
3. **Check if fetch requests are batched** or individual

### Short-term (1-2 days) 🔧

If fetches are individual (likely):
1. **Implement batch fetching** in `RenderableInner::poll()`
2. **Accumulate stale ranges** instead of fetching one-by-one
3. **Send single RPC** with multiple line ranges

**Expected improvement**: 50-100x reduction in network requests!

### Medium-term (1 week) 🚀

1. **Implement prefetching** based on viewport
2. **Implement delta updates** (only send changed cells, not full lines)
3. **Implement compression** for line data

---

## Build Status

✅ Successfully built with emergency logging

```bash
Finished `dev` profile [unoptimized + debuginfo] target(s) in 7.68s
```

---

## Summary

### Completed

- ✅ Added emergency logging to `ClientPane::resize()`
- ✅ Analyzed Terminal resize logic (already smart!)
- ✅ Analyzed LocalPane resize logic (clean)
- ✅ Analyzed remote mux server resize handling
- ✅ Built successfully

### Key Findings

1. **Terminal resize is already efficient** (no over-invalidation)
2. **Phase 19 client code exists but may not be executing**
3. **Real bottleneck is likely network round-trips** (100-300 requests per resize)
4. **Solution is batch fetching**, not just selective invalidation

### Next Action

**Run Test 1** (emergency logging) to confirm if `ClientPane::resize()` is being called!

If it is:
→ Profile `TabResized` handling
→ Implement batch fetching

If it isn't:
→ Find actual resize code path
→ Apply Phase 19 there

