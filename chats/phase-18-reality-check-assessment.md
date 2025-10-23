# Phase 18: Reality Check - Assessment of Phase 17 Results

## Date
2025-10-23

## Status
⚠️ **REASSESSMENT NEEDED** - Phase 17.4 deployed, but UI still sluggish

---

## Executive Summary

**Phase 17.4** (Adaptive FPS fix) was deployed and tested, but **the UI is still not smooth during resize**. The frame logs and perf report reveal that **our analysis was incomplete** - we focused on the right areas (GPU stalls, compositor sync) but **the Phase 17 frameworks haven't solved the problem yet because they're not fully wired up**.

**Key Finding**: The sluggishness persists because **Phase 17.1, 17.2, and 17.3 are frameworks only** - the critical EGL/Wayland wiring is still missing. Phase 17.4 (adaptive FPS) is working, but it alone cannot fix the GPU stall problem.

---

## Test Results Analysis

### Build Status ✅
- Build completed successfully in 2m 27s
- Only expected warnings (unused helper functions)
- No compilation errors

### Frame Logs Analysis

**Key Observations from `frame-logs.17`**:

1. **Adaptive FPS is working** (Phase 17.4) ✅:
   ```
   Line 10: Adaptive frame rate: High (60 fps) → Medium (30 fps) (idle: 134.371066ms)
   Line 13: Adaptive frame rate: Medium (30 fps) → High (60 fps) (idle: 253.211659ms)
   ```
   - Threshold change to 2 seconds is effective
   - No more rapid mode thrashing

2. **GPU stalls are STILL MASSIVE** ⚠️:
   ```
   Line 16: Frame callback completed after 142ms wait
   Line 17: Frame callback completed after 246ms wait
   Line 18: Frame callback completed after 285ms wait
   Line 19: Frame callback completed after 410ms wait (!)
   Line 20: Frame callback completed after 417ms wait (!)
   Line 22: Frame callback completed after 377ms wait
   Line 23: Frame callback completed after 463ms wait (!)
   Line 24: Frame callback completed after 544ms wait (!!)
   Line 25: Frame callback completed after 475ms wait (!)
   ...
   Line 32: Frame callback completed after 692ms wait (!!!)
   Line 33: Frame callback completed after 661ms wait (!!!)
   Line 36: Frame callback completed after 754ms wait (!!!!)
   ```

3. **Stall Statistics** (from lines 16-109):
   - **Total GPU stalls observed**: 52 in ~2 minutes
   - **Stall duration range**: 100ms - 754ms
   - **Average stall**: ~350ms
   - **Stalls >500ms**: 15 stalls (29%)
   - **Stalls >600ms**: 11 stalls (21%)
   
   **This is WORSE than Phase 16!** (Phase 16 had 57 stalls, but average ~250ms)

4. **Frame Times** (from periodic stats):
   ```
   Line 14: avg=13.5ms, median=6.2ms, p95=45.0ms, p99=45.0ms
   Line 21: avg=12.7ms, median=10.1ms, p95=38.6ms, p99=45.0ms
   Line 110: avg=6.9ms, median=5.3ms, p95=12.3ms, p99=14.8ms
   Line 111: avg=6.8ms, median=5.8ms, p95=12.0ms, p99=14.8ms
   ```
   - **Good news**: Frame rendering itself is fast (6-8ms avg at end)
   - **Bad news**: Punctuated by massive GPU stalls

5. **Event Coalescing is working** ✅:
   ```
   Line 15: Event coalescing: 1 resize events coalesced in last 5s (2x reduction)
   Line 60: Event coalescing: 3 resize events coalesced in last 5s (4x reduction)
   ```
   - Phase 15 event coalescing is functioning
   - But not addressing the main bottleneck

### Perf Report Analysis

**Top CPU consumers** (from `perf-report.17`):

1. **`__memmove_avx512_unaligned_erms`**: 3.33%
   - Memory copying overhead
   - Down from ~14% in earlier phases ✅
   - TabBarState caching is working!

2. **`clock_gettime`**: 1.44% + 1.00% = **2.44%**
   - Still significant but reduced
   - SpawnQueue::queue_func and pop_func
   - Not the primary bottleneck anymore

3. **Lua GC**: 1.61% (lua_newuserdatauv → luaC_step → singlestep → GCTM)
   - Lua garbage collection overhead
   - Not the primary bottleneck

4. **Memory allocation**: 1.85% (alloc::raw_vec::RawVecInner)
   - Normal overhead for Rust

5. **No compositor overhead in top samples**
   - Most time is spent waiting, not in user-space CPU

---

## What's Working vs What's Not

### ✅ What's Working (Phase 17.4 + Previous Phases)

1. **Adaptive FPS threshold fix** (Phase 17.4):
   - No more mode thrashing ✅
   - Stable frame rate mode ✅

2. **TabBarState caching** (Phase 5):
   - `__memmove` down from 14% to 3.3% ✅
   - Effective memory optimization ✅

3. **Event coalescing** (Phase 15):
   - Resize events being combined ✅
   - 2-4x reduction observed ✅

4. **Lua optimizations** (Phases 0-5):
   - Callbacks cached ✅
   - Serialization optimized ✅

### ⚠️ What's NOT Working

1. **GPU Stalls** (Phase 17.1 - Triple Buffering):
   - ❌ Still 100-750ms stalls
   - ❌ 52 stalls in 2 minutes
   - ❌ **Worse than Phase 16!**
   - **Root Cause**: Triple buffering framework exists but **NOT WIRED UP**

2. **GPU Fences** (Phase 17.2):
   - ❌ Not preventing GPU queue overflow
   - **Root Cause**: EGL fence creation **NOT IMPLEMENTED**
   - The wait logic exists but there are no fences to wait for!

3. **Presentation-time** (Phase 17.3):
   - ❌ No vsync alignment feedback
   - **Root Cause**: Wayland protocol **NOT BOUND**

---

## The Critical Problem: Phase 17 is Framework-Only

### What We Built in Phase 17

**Phase 17.1: Triple Buffering**:
- ✅ `TripleBufferManager` class (432 lines)
- ✅ State machine (Available → Rendering → Queued → Displayed)
- ✅ Buffer rotation logic
- ❌ **NOT WIRED**: No EGL configuration for 3 buffers
- ❌ **NOT WIRED**: No lifecycle hooks in do_paint()/finish_frame()

**Phase 17.2: GPU Fences**:
- ✅ `GpuFenceManager` class (268 lines)
- ✅ Wait logic in `do_paint()` (lines 1164-1183 in window.rs)
- ✅ Framework in `finish_frame()` (lines 402-430 in window.rs)
- ❌ **NOT WIRED**: No actual EGL sync fence creation
- ❌ **NOT WIRED**: No access to EGL context for fence creation

**Phase 17.3: Presentation-time**:
- ✅ `PresentationManager` class (304 lines)
- ✅ Vsync prediction algorithms
- ❌ **NOT WIRED**: No Wayland protocol binding
- ❌ **NOT WIRED**: No feedback collection from compositor

### Why GPU Stalls Persist

**The wait logic exists but has nothing to wait for**:

```rust
// In window/src/os/wayland/window.rs, line 1167
if let Some(gl_state) = self.gl_state.as_ref() {
    let mut fence_manager = self.gpu_fence_manager.borrow_mut();
    if fence_manager.is_fence_signaled() == Some(false) {
        // This check ALWAYS returns None because no fences are created!
        fence_manager.wait_for_fence(timeout);
    }
}
```

**The finish_frame override creates no fences**:

```rust
// In window/src/os/wayland/window.rs, line 413
let backend = gl_state.get_framebuffer_dimensions(); // Just to ensure context is valid

// TODO: Once we have proper EGL access, create the fence:
// let mut fence_manager = inner.gpu_fence_manager.borrow_mut();
// if let Err(e) = fence_manager.create_fence(egl_ptr, display) {
//     log::warn!("Failed to create GPU fence: {}", e);
// }
```

**This is why GPU stalls persist at 100-750ms!**

---

## Why Phase 17 Appeared Complete

### The Illusion of Completeness

1. ✅ **1,004 lines of new code** - Looks substantial
2. ✅ **Builds successfully** - No compilation errors
3. ✅ **Comprehensive documentation** - Every TODO documented
4. ✅ **Unit tests pass** - Framework logic is correct

**But**: All of this is **infrastructure without integration**.

### What We Missed

**The "final wiring" is not minor cleanup - it's THE ACTUAL FIX**:

1. **Triple Buffering wiring** = Configure EGL for 3 buffers
   - This is what eliminates blocking
   - Framework alone does nothing

2. **GPU Fence wiring** = Create actual EGL sync fences
   - This is what prevents queue overflow
   - Manager alone does nothing

3. **Presentation-time wiring** = Bind Wayland protocol
   - This is what provides timing feedback
   - Manager alone does nothing

**Analogy**: We built a car (framework) but didn't connect the engine (wiring). The car looks complete but doesn't move.

---

## Why GPU Stalls Got Worse

### Phase 16 → Phase 17 Comparison

**Phase 16**:
- 57 stalls in 2.5min
- 100-700ms duration
- Average ~250ms

**Phase 17**:
- 52 stalls in 2min
- 100-750ms duration
- Average ~350ms
- More >500ms stalls (29% vs 20%)

**Hypothesis**: Phase 15's adaptive FPS and frame budgeting may have added overhead without benefits since the GPU stalls dominate.

---

## The Hard Truth: We Need to Complete the Wiring

### What "Wiring" Actually Means

**Phase 17.1: Triple Buffering Wiring** (2-3 days):

1. **EGL Configuration** (in `window/src/egl.rs`):
   ```rust
   // Modify GlState::create_wayland() to configure triple buffering
   let config_attribs = [
       ffi::SURFACE_TYPE, ffi::WINDOW_BIT,
       ffi::RENDERABLE_TYPE, ffi::OPENGL_BIT,
       ffi::RED_SIZE, 8,
       ffi::GREEN_SIZE, 8,
       ffi::BLUE_SIZE, 8,
       ffi::ALPHA_SIZE, 8,
       ffi::DEPTH_SIZE, 24,
       ffi::RENDER_BUFFER, ffi::BACK_BUFFER,  // Use back buffer
       ffi::NONE,
   ];
   
   // After creating surface:
   egl.SwapInterval(display, 1);  // Sync to vsync but don't block
   ```

2. **Buffer Lifecycle** (in `window/src/os/wayland/window.rs`):
   ```rust
   // In do_paint():
   let mut buffer_mgr = self.triple_buffer_manager.borrow_mut();
   match buffer_mgr.acquire_buffer() {
       Some(buffer_id) => {
           log::trace!("Rendering to buffer {}", buffer_id);
           // Continue with paint
       }
       None => {
           self.invalidated = true;
           return Ok(());  // Drop frame if all buffers busy
       }
   }
   
   // In finish_frame():
   buffer_mgr.queue_current_buffer();
   
   // In next_frame_is_ready():
   for buffer_id in 0..3 {
       if buffer_mgr.buffer_info(buffer_id)?.state == BufferState::Displayed {
           buffer_mgr.release_buffer(buffer_id);
       }
   }
   ```

**Phase 17.2: GPU Fence Wiring** (2-3 days):

Need to access EGL context from glium:

```rust
// In window/src/os/wayland/window.rs finish_frame():
WaylandConnection::with_window_inner(self.0, |inner| {
    if let Some(gl_state) = inner.gl_state.as_ref() {
        // Need to extract EGL display and context from gl_state
        // This requires modifying glium or using unsafe FFI
        
        let egl_display = /* extract from gl_state */;
        let egl_funcs = /* get EGL function pointers */;
        
        let mut fence_manager = inner.gpu_fence_manager.borrow_mut();
        if let Err(e) = fence_manager.create_fence(egl_funcs, egl_display) {
            log::warn!("Failed to create GPU fence: {}", e);
        }
    }
    Ok(())
});
```

**Phase 17.3: Presentation-time Wiring** (2-3 days):

Bind Wayland protocol:

```rust
// In window/src/os/wayland/state.rs:
pub(super) struct WaylandState {
    // ... existing fields ...
    pub(super) presentation: Option<WpPresentation>,
}

// In window/src/os/wayland/connection.rs global handler:
if interface == "wp_presentation" {
    let presentation = registry.bind::<WpPresentation, _, _>(name, version.min(1), &qh, ());
    state.presentation = Some(presentation);
}

// Implement Dispatch<WpPresentationFeedback> handler
// Request feedback in do_paint()
// Record feedback and update PresentationManager
```

---

## Realistic Assessment

### Time Estimates (Revised)

**Originally estimated**: 5-8 days for "final wiring"

**Realistic estimate**: **2-3 weeks** because:

1. **Phase 17.1 (Triple Buffering)**: 3-5 days
   - Need to understand glium's buffer management
   - May need to modify glium crate
   - Testing across different drivers
   - Risk: High (driver-dependent)

2. **Phase 17.2 (GPU Fences)**: 3-5 days
   - Need unsafe FFI to access EGL from glium
   - May need to fork/modify glium
   - Driver compatibility issues
   - Risk: Medium-High

3. **Phase 17.3 (Presentation-time)**: 2-3 days
   - Wayland protocol binding (well-documented)
   - Handler implementation
   - Testing on multiple compositors
   - Risk: Low-Medium

**Total**: **8-13 days** of focused development work

### Complexity Reality Check

**These are NOT simple tasks**:

1. **Requires deep understanding** of:
   - EGL API and buffer management
   - glium internals
   - Wayland protocol implementation
   - GPU driver behavior

2. **Requires modifications to**:
   - Potentially glium crate (forking)
   - EGL FFI bindings
   - Wayland protocol handlers

3. **High risk of**:
   - Driver-specific bugs
   - Compositor-specific issues
   - Subtle timing/synchronization bugs

---

## Alternative: The Sledgehammer Approach

### Since We're Already Deep in the Weeds...

**Option A: Complete Phase 17 wiring** (2-3 weeks, high risk)
- Finish triple buffering
- Finish GPU fences
- Finish presentation-time
- Expected: Finally fix GPU stalls
- Risk: High complexity, may hit driver/compositor issues

**Option B: Simpler workarounds** (1-2 days, lower risk)
- Force lower resolution during resize (Qt does this)
- Skip frames more aggressively
- Disable expensive features during resize (tab bar, etc.)
- Expected: Acceptable but not smooth
- Risk: Low, but compromises UX

**Option C: Profile compositor itself** (2-3 days investigation)
- WaylandWindow might not be the bottleneck
- KWin might be slow at something specific
- We might be triggering expensive compositor operations
- Expected: Find compositor-specific optimization
- Risk: Medium, may not find anything actionable

---

## Recommendation

### Short Term (This Week)

**Option B+: Practical Workarounds**

1. **Reduce resize frequency even more** (1 hour):
   ```rust
   // In window/src/os/wayland/window.rs
   // Change resize throttle from 16ms to 33ms (30 FPS max)
   if self.resize_throttled && self.last_resize.elapsed() < Duration::from_millis(33) {
       ...
   }
   ```

2. **Skip tab bar updates during fast resize** (2 hours):
   ```rust
   // In wezterm-gui/src/termwindow/mod.rs
   // Don't update tab bar if resize event frequency > 30/sec
   if self.last_resize_time.elapsed() < Duration::from_millis(100) {
       // Use cached tab bar, don't recompute
   }
   ```

3. **Disable animations during resize** (1 hour):
   ```rust
   // Disable smooth scrolling, cursor blinking during resize
   ```

**Expected Result**: Sluggishness reduces but doesn't disappear (70% better?)

**Effort**: 1 day

**Risk**: Very low

---

### Medium Term (Next 2-3 Weeks)

**Option A: Complete Phase 17 Wiring**

Only if:
- User is willing to invest 2-3 weeks
- User can test on target hardware/compositor
- User accepts risk of driver-specific bugs

**Steps**:
1. Week 1: Triple buffering wiring + testing
2. Week 2: GPU fences wiring + testing
3. Week 3: Presentation-time wiring + polish

**Expected Result**: Smooth 60 FPS (if successful)

**Risk**: High (driver/compositor issues)

---

### Long Term (Alternative Approach)

**Option D: Switch rendering backend**

Zed is smooth because it uses `wgpu` which handles all this automatically:
- Triple buffering built-in
- GPU synchronization handled
- Presentation-time support

**Steps**:
1. Replace glium with wgpu (2-4 weeks major refactor)
2. Let wgpu handle buffer management
3. Much cleaner, modern approach

**Expected Result**: Smooth 60 FPS + future-proof

**Risk**: Medium (large refactor)

---

## Conclusion

### The Uncomfortable Truth

**Phase 17 built the right infrastructure but didn't connect it**. The frameworks are correct, well-designed, and comprehensive. But they're **not active**.

**The UI is still sluggish because**:
- ❌ Triple buffering is not configured (CPU still blocks on GPU)
- ❌ GPU fences are not created (queue overflow still happens)
- ❌ Presentation-time is not bound (no vsync feedback)

**Phase 17.4 (Adaptive FPS) is working**, but it can't fix GPU stalls - only manage frame rate when they occur.

### The Choice

**Quick Fix** (Option B+): 1 day, 70% better, low risk
- Reduce resize frequency
- Skip expensive operations during resize
- Disable animations
- Accept "good enough" on Wayland

**Complete Fix** (Option A): 2-3 weeks, smooth 60 FPS, high risk
- Wire up triple buffering
- Wire up GPU fences
- Wire up presentation-time
- Match Zed/Chrome smoothness (if successful)

**Clean Slate** (Option D): 2-4 weeks, smooth 60 FPS, medium risk
- Replace glium with wgpu
- Modern, maintained solution
- Future-proof

### My Recommendation

**Start with Option B+ (quick fix)** to get acceptable performance now, then **evaluate if Option A or D is worth the investment** based on user needs and priorities.

**The Phase 17 work is not wasted** - the frameworks are solid and can be completed later. But we need to be realistic about the effort required.

---

**Phase 17 Status**: ⚠️ **Infrastructure Complete, Wiring Needed (2-3 weeks)**  
**Phase 17.4 Status**: ✅ **Complete and Working**  
**GPU Stalls**: ❌ **Still 100-750ms, 52 per 2min**  
**UI Smoothness**: ❌ **Still sluggish during resize**

**Next Steps**: Choose between Quick Fix (1 day), Complete Wiring (2-3 weeks), or Backend Replacement (2-4 weeks)

