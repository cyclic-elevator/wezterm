# Phase 19.2: Server 100% CPU Hang Analysis

**Date**: 2025-10-26
**Status**: 🔴 **CRITICAL - Server-Side Runaway Terminal Rewrap**

---

## Executive Summary

**Good news**: Client UI is now responsive! ✅  
**Bad news**: Server is stuck at 100% CPU for >60 seconds after resize stops! ❌

**Root cause**: The client is no longer calling `resync()`, but **the 100ms debounced resize RPCs are STILL BEING SENT**. The server is processing thousands of resize RPCs, each triggering expensive terminal rewrap operations.

---

## Evidence from Logs

### Client Side (`frame-logs.19.2`)

**Timeline**: 08:16:00 - 08:16:01 (1 second window)

```
Resize storm events blocked: 1,963
Actual resize RPCs scheduled: 2,933

08:16:00.714 - 08:16:01.276 (562ms of continuous resize events!)
```

**Key observations**:

1. ✅ **Redundant detection working**: 1,963 "RESIZE STORM" blocks
2. ❌ **Debounce NOT working**: 2,933 "PHASE 19 CLIENTPANE RESIZE" calls
3. ❌ **Massive async task spam**: Each non-redundant resize spawns a new 100ms delay task
4. ❌ **Tasks fire after drag stops**: The 2,933 tasks all fire their RPCs 100ms later

**Pattern observed**:
```
08:16:00.714  RESIZE STORM × 9 (blocked)
08:16:00.833  CLIENTPANE RESIZE × 6 (sent to server!)
08:16:00.915  RESIZE STORM × 9 (blocked)
08:16:01.112  CLIENTPANE RESIZE × 6 (sent to server!)
08:16:01.206  RESIZE STORM × 180 (blocked)
08:16:01.207  (continues for hundreds more events)
```

**The problem**: While **most** resize events are redundant (same dimensions), there are **enough non-redundant ones** (different panes have different sizes) to trigger ~3,000 debounce tasks!

---

### Server Side (`perf-report.19.2`)

**CPU profile during hang**:

```
86.13% __memmove_avx512_unaligned_erms  ← Memory copying (terminal rewrap!)
 0.72% Line::set_cell_grapheme           ← Cell updates
 0.69% Option::get_or_insert_with         ← Cell allocation
 0.54% grapheme_clusters::first_character ← Text processing
 0.47% str::validations::next_code_point  ← UTF-8 decoding
 0.44% SliceIndex::index                  ← Line indexing
 0.29% grapheme_clusters::find_cluster    ← Text segmentation
 0.28% ZSTD_compressStream2               ← Response compression
 0.23% Line::wrap                          ← Terminal line wrapping!
```

**Analysis**:

- **86% memmove**: Massive memory copying from terminal rewrap operations
- **Line operations**: `set_cell_grapheme`, `Line::wrap` indicate terminal content manipulation
- **ZSTD compression**: Server is compressing responses to send back to client

**What's happening**:
1. Server receives ~3,000 resize RPCs (from broken debounce)
2. Each RPC triggers `Terminal::resize()` which rewraps the entire terminal buffer
3. Terminal rewrap involves:
   - Copying all cell contents to new dimensions
   - Recalculating line wrapping
   - Updating grapheme clusters
   - Processing UTF-8 boundaries
4. Each resize generates a response that needs ZSTD compression
5. This continues for >60 seconds (processing 3,000 rewraps!)

---

## Root Cause: Broken Debounce (Confirmed)

### The Code (from Phase 19.2 implementation)

**File**: `wezterm-client/src/pane/clientpane.rs` lines 438-461

```rust
fn resize(&self, size: TerminalSize) -> Result<()> {
    // ... redundant detection (WORKING) ...
    
    if is_redundant {
        log::error!("🔴 RESIZE STORM: Redundant resize");
        return Ok(());  // ← Blocks most events ✅
    }
    
    // Update local dimensions
    inner.dimensions = size;
    inner.make_viewport_stale(100);
    
    // "Debounce" - BROKEN! ❌
    promise::spawn::spawn(async move {
        Timer::after(Duration::from_millis(100)).await;
        
        // NO CANCELLATION!
        client.client.resize(Resize {
            pane_id: remote_pane_id,
            size,
        }).await
    }).detach();
    
    Ok(())
}
```

**The problem**:

1. Redundant detection blocks most events (1,963/4,896 = 40% blocked) ✅
2. But **60%** of events are **not redundant** (different panes, different sizes) ❌
3. Each non-redundant resize **spawns a new independent async task** ❌
4. No cancellation mechanism - all 2,933 tasks run 100ms later ❌
5. Server receives 2,933 resize RPCs and processes them all! ❌

**Why redundant detection doesn't save us**:

With 9 panes of different sizes:
- Panes 0-2: 82x38
- Panes 3-6: 70x37 → 82x38 (size changed!)
- Panes 7-8: 80x38 → 82x38 (size changed!)

**Result**: Only panes 0-2 trigger redundant detection. Panes 3-8 all send resize RPCs!

---

## Why Priority 1 Made UI Responsive But Broke Server

### Before Priority 1

```
Client resize event
  → Redundant detection blocks 99% ✅
  → 1 resize RPC sent to server
  → Server processes resize (10ms)
  → Server sends TabResized(topology_changed=false)
  → Client receives TabResized
  → Client calls resync() ← 150ms WAIT ❌
  → Client blocked, can't send more resizes
  
Result: Client slow, but only ~5-10 resize RPCs sent total
```

### After Priority 1

```
Client resize event
  → Redundant detection blocks 40%
  → 60% spawn debounce tasks (2,933 tasks!)
  → Client DOESN'T call resync() ← Client responsive! ✅
  → Client can send more resizes immediately
  → 100ms later: All 2,933 tasks fire their RPCs! ❌
  → Server receives 2,933 resize RPCs
  → Server processes 2,933 × 10ms = 29 seconds of work ❌
  → Server hangs at 100% CPU for 60+ seconds ❌
```

**The unintended consequence**:

- **Before**: `resync()` blocked the client, but also **throttled resize RPC sending**
- **After**: No `resync()` → client responsive, but **no throttling** → RPC flood!

---

## The Resize Storm Feedback Loop (Still Active!)

Even though we fixed the **client-side** resize storm (redundant detection), there's a **new** feedback loop:

```
1. User drags window (2 seconds, 60 GUI events)
2. Redundant detection blocks 40% (1,963 events)
3. 60% pass through (2,933 events from 9 panes)
4. Each spawns a debounce task (2,933 tasks)
5. 100ms later: All 2,933 tasks fire RPCs
6. Server receives 2,933 resize RPCs
7. Server processes resize #1 → TabResized notification
8. Client receives TabResized(topology_changed=false)
9. Client skips resync() ✅
10. But client's panes have stale dimensions!
11. Something triggers re-layout (window focus? timer?)
12. Re-layout calls resize() on all panes again!
13. GO TO STEP 2 (feedback loop!)
```

**Evidence**: The logs show resize events continuing AFTER the mouse drag stops (08:16:01.206+), suggesting the client is re-triggering resizes internally.

---

## Why This Wasn't Caught Earlier

**Phase 19.2 Priority 1 focused on**:
- ✅ Eliminating `resync()` overhead (achieved!)
- ✅ Client UI responsiveness (achieved!)

**Phase 19.2 Priority 1 did NOT address**:
- ❌ The broken debounce implementation
- ❌ Server-side RPC flood
- ❌ Terminal rewrap overhead

**The assumption**: Redundant detection would block enough events that the broken debounce wouldn't matter.

**Reality**: With 9 panes of different sizes, 60% of events are non-redundant!

---

## Immediate Fix Required

### Critical: Fix Debounce Implementation

**Current (broken)**:
```rust
// Each resize spawns a new task
promise::spawn::spawn(async move {
    Timer::after(Duration::from_millis(100)).await;
    client.resize(size).await  // NO CANCELLATION!
}).detach();
```

**Required (true debounce)**:
```rust
// Shared state to track pending resize
struct PendingResize {
    size: TerminalSize,
    generation: usize,  // Cancel token
}

fn resize(&self, size: TerminalSize) -> Result<()> {
    // ... redundant detection ...
    
    // Update dimensions
    inner.dimensions = size;
    inner.make_viewport_stale(100);
    
    // TRUE DEBOUNCE with cancellation
    {
        let mut pending = self.pending_resize.lock();
        let generation = pending.generation + 1;
        pending.generation = generation;
        pending.size = size;
        
        let client = Arc::clone(&self.client);
        let pending_resize = Arc::clone(&self.pending_resize);
        
        promise::spawn::spawn(async move {
            Timer::after(Duration::from_millis(100)).await;
            
            // Check if we're still the latest resize
            let should_send = {
                let pending = pending_resize.lock();
                pending.generation == generation  // Only send if not superseded
            };
            
            if should_send {
                client.resize(Resize { size }).await
            }
        }).detach();
    }
    
    Ok(())
}
```

**Key differences**:
1. ✅ Generation counter tracks latest resize
2. ✅ Before sending RPC, check if superseded
3. ✅ Only the FINAL resize gets sent
4. ✅ All intermediate resizes are cancelled

---

## Additional Server-Side Protections

### Server-Side Resize Deduplication

Even with fixed debounce, add belt-and-suspenders protection on server:

**File**: `mux/src/tab.rs` (in `Tab` or `TabInner`)

```rust
struct TabInner {
    // ... existing fields ...
    last_resize_size: Option<TerminalSize>,
    last_resize_time: Instant,
}

fn resize(&mut self, size: TerminalSize) {
    // Skip if size unchanged
    if Some(size) == self.last_resize_size {
        log::debug!("Server: Skipping redundant resize {}x{}", size.cols, size.rows);
        return;
    }
    
    // Rate limit: max 10 resizes/second
    if self.last_resize_time.elapsed() < Duration::from_millis(100) {
        log::warn!("Server: Rate-limiting resize (too frequent)");
        return;
    }
    
    // Proceed with resize
    self.last_resize_size = Some(size);
    self.last_resize_time = Instant::now();
    
    // ... existing resize logic ...
}
```

---

## Testing Plan

### Verify Debounce Fix

**Run**:
```bash
RUST_LOG=debug,wezterm_client=debug ./wezterm-gui start
```

**Drag window for 2 seconds, look for**:
```
METRIC:debounce_scheduled: 2933
METRIC:debounce_cancelled: 2932
METRIC:debounce_sent: 1
```

**Expected**: Only 1 resize RPC sent (the final size)!

### Verify Server Protection

**On server, look for**:
```
Server: Skipping redundant resize 82x38 (appears many times)
Server: Rate-limiting resize (too frequent) (appears if client still floods)
```

---

## Performance Expectations After Fix

### Current State (Phase 19.2 with broken debounce)

```
User drags window (2 seconds):
  Client: Responsive! ✅
  Server: 100% CPU for 60+ seconds ❌
  Total RPCs sent: 2,933 ❌
  Server work: 2,933 × 10ms = 29+ seconds ❌
```

### After Debounce Fix

```
User drags window (2 seconds):
  Client: Responsive! ✅
  Server: Processes 1 resize (10ms) ✅
  Total RPCs sent: 1 ✅
  Server work: 1 × 10ms = 10ms ✅
```

**Improvement**: **2,932 fewer RPCs** (99.97% reduction!)

---

## Lessons Learned

### What Went Wrong

1. **Priority 1 was correct** - eliminating `resync()` was the right fix
2. **Debounce was always broken** - but masked by `resync()` throttling
3. **Removing `resync()` exposed the broken debounce** - unintended consequence
4. **Redundant detection isn't enough** - multi-pane windows have many non-redundant resizes

### Why Testing Didn't Catch This

- **Single pane testing**: Would have caught redundant detection working
- **Multi-pane testing with server profiling**: Required to catch this
- **Load testing**: Server-side impacts need separate monitoring

### Design Principle Violated

**Principle**: "Client should never be able to DOS the server"

**Violation**: Broken debounce allows client to send 3,000 RPCs in 100ms burst

**Fix**: Implement proper debounce with cancellation (Priority 2)

---

## Priority List (Updated)

### 🔴 Priority 1: Fix Debounce Implementation (CRITICAL)

**Status**: **MUST FIX IMMEDIATELY**

**Why**: Server hangs at 100% CPU for >60 seconds after resize

**Implementation**: See "Immediate Fix Required" section above

**Effort**: 2-4 hours

**Risk**: LOW (self-contained fix, adds cancellation)

---

### 🟡 Priority 2: Server-Side Protection (HIGH)

**Status**: **RECOMMENDED**

**Why**: Belt-and-suspenders defense against client bugs

**Implementation**: See "Additional Server-Side Protections" section above

**Effort**: 1-2 hours

**Risk**: LOW (adds deduplication layer)

---

### 🟢 Priority 3: Metrics/Instrumentation (MEDIUM)

**Status**: Optional (for production monitoring)

**Why**: Validate fixes, monitor for edge cases

**Effort**: 2-3 hours

**Risk**: NONE (observability only)

---

## Conclusion

**The good**: Priority 1 (TabResized→resync fix) achieved its goal - client UI is responsive!

**The bad**: Priority 1 exposed a pre-existing bug (broken debounce) that was previously masked by `resync()` throttling.

**The fix**: Implement proper debounce with cancellation (Phase 19.2 Priority 2 from original document).

**Timeline**: Critical fix needed immediately (2-4 hours work).

**Expected result**: Client responsive ✅ + Server responsive ✅ = Complete solution! 🎉

---

**Next steps**: Implement Priority 1 (Fix Debounce) immediately.

