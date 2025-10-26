# Phase 19.3: Fix Server 100% CPU Hang - Action Plan

**Date**: 2025-10-26
**Status**: 🔴 **CRITICAL FIX REQUIRED**

---

## Problem Summary

✅ **Client UI**: Now responsive (Phase 19.2 Priority 1 succeeded!)  
❌ **Server**: Stuck at 100% CPU for >60 seconds (broken debounce exposed!)

**Root cause**: The "debounce" implementation doesn't actually debounce - it spawns 2,933 independent async tasks that all fire 100ms later, flooding the server with resize RPCs.

---

## The Fix

### Priority 1: Implement True Debounce with Cancellation

**File**: `wezterm-client/src/pane/clientpane.rs`

**Current code (broken)**:
```rust
// Each resize spawns a NEW task - no cancellation!
promise::spawn::spawn(async move {
    Timer::after(Duration::from_millis(100)).await;
    client.resize(size).await  // All 2,933 fire!
}).detach();
```

**Fixed code (true debounce)**:
```rust
// Add to ClientPane struct:
pending_resize: Arc<Mutex<PendingResize>>,

struct PendingResize {
    size: Option<TerminalSize>,
    generation: usize,
}

// In resize() method:
{
    let mut pending = self.pending_resize.lock();
    let generation = pending.generation + 1;
    pending.generation = generation;
    pending.size = Some(size);
    
    let client = Arc::clone(&self.client);
    let pending_resize = Arc::clone(&self.pending_resize);
    let remote_pane_id = self.remote_pane_id;
    let remote_tab_id = self.remote_tab_id;
    
    promise::spawn::spawn(async move {
        Timer::after(Duration::from_millis(100)).await;
        
        // Check if superseded
        let (should_send, final_size) = {
            let pending = pending_resize.lock();
            if pending.generation == generation {
                (true, pending.size.unwrap())
            } else {
                (false, size)  // Cancelled
            }
        };
        
        if should_send {
            client.resize(Resize {
                containing_tab_id: remote_tab_id,
                pane_id: remote_pane_id,
                size: final_size,
            }).await.ok();
        }
    }).detach();
}
```

**Key changes**:
1. ✅ Shared `pending_resize` state
2. ✅ Generation counter for cancellation
3. ✅ Check before sending: only send if not superseded
4. ✅ Result: 2,933 tasks scheduled, 2,932 cancelled, 1 sent!

---

### Priority 2: Server-Side Protection (Belt-and-Suspenders)

**File**: `mux/src/tab.rs` (in `TabInner`)

```rust
// Add fields:
last_resize_size: Option<TerminalSize>,
last_resize_time: Instant,

// In resize() method (FIRST THING):
fn resize(&mut self, size: TerminalSize) {
    // Deduplicate
    if Some(size) == self.last_resize_size {
        log::debug!("Server: Skipping redundant resize {}x{}", size.cols, size.rows);
        return;
    }
    
    // Rate limit (max 10/sec)
    if self.last_resize_time.elapsed() < Duration::from_millis(100) {
        log::warn!("Server: Rate-limiting resize (too frequent)");
        return;
    }
    
    self.last_resize_size = Some(size);
    self.last_resize_time = Instant::now();
    
    // ... existing resize logic ...
}
```

---

## Expected Results

### Current (Phase 19.2)
```
2-second drag:
  - Client: Responsive ✅
  - Server: 2,933 RPCs received ❌
  - Server: 100% CPU for 60+ seconds ❌
```

### After Fix (Phase 19.3)
```
2-second drag:
  - Client: Responsive ✅
  - Server: 1 RPC received ✅
  - Server: Processes in 10ms ✅
```

**Improvement**: 2,932 fewer RPCs (99.97% reduction!)

---

## Testing

### Verify Client-Side Debounce

Add metrics:
```rust
static DEBOUNCE_SCHEDULED: AtomicUsize = AtomicUsize::new(0);
static DEBOUNCE_CANCELLED: AtomicUsize = AtomicUsize::new(0);
static DEBOUNCE_SENT: AtomicUsize = AtomicUsize::new(0);
```

**Expected logs after 2-second drag**:
```
METRIC:debounce_scheduled: 2933
METRIC:debounce_cancelled: 2932
METRIC:debounce_sent: 1
```

### Verify Server-Side

**Expected logs**:
```
Server: Skipping redundant resize 82x38 (many times if client still buggy)
Server: Rate-limiting resize (if burst detected)
```

### Monitor Server CPU

```bash
# On server, run top/htop during client resize
# Should see: <5% CPU (vs 100% before)
```

---

## Implementation Steps

1. ✅ **Understand problem** (complete - see analysis)
2. ⏳ **Implement client debounce fix** (2-3 hours)
3. ⏳ **Implement server protection** (1 hour)
4. ⏳ **Add metrics** (30 minutes)
5. ⏳ **Test with multi-pane window** (30 minutes)
6. ⏳ **Validate server CPU stays low** (10 minutes)

**Total effort**: ~4-5 hours

---

## Why This Matters

**Before Phase 19.2**:
- Client: Slow (300-500ms latency) ❌
- Server: Fine (5-10 RPCs) ✅

**Phase 19.2 (current)**:
- Client: Fast ✅
- Server: Broken (2,933 RPCs → hang) ❌

**Phase 19.3 (after fix)**:
- Client: Fast ✅
- Server: Fine (1 RPC) ✅

**= Complete solution!** 🎉

---

## Files to Modify

1. `wezterm-client/src/pane/clientpane.rs` - Fix debounce
2. `mux/src/tab.rs` - Add server protection
3. (Optional) Add metrics for validation

---

**Ready to implement!**

