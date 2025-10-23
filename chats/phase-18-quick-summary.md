# Phase 18: Quick Fix - Summary

## ✅ Implementation Complete

**Option B+ implemented in 11 seconds build time.**

---

## Three Changes Made

### 1️⃣ Slower Resize (60fps → 30fps)
```rust
let throttle_duration = Duration::from_millis(33); // was 16ms
```
- **Impact**: Halves resize event frequency
- **File**: `window/src/os/wayland/window.rs`

### 2️⃣ Skip Tab Bar During Fast Resize
```rust
let fast_resize_in_progress = self.last_resize_time.elapsed() < Duration::from_millis(100);
let force_cache = fast_resize_in_progress && self.cached_tab_bar.is_some();
```
- **Impact**: Avoids expensive Lua callbacks during resize
- **Files**: `wezterm-gui/src/termwindow/mod.rs`, `resize.rs`

### 3️⃣ No Cursor Blinking During Resize
```rust
let blinking = ... && !fast_resize_in_progress;
```
- **Impact**: Eliminates animation overhead during resize
- **File**: `wezterm-gui/src/termwindow/render/mod.rs`

---

## Expected Results

**Baseline (Phase 17)**:
- 52 GPU stalls/2min
- 100-750ms stalls
- Sluggish resize

**Target (Phase 18)**:
- <35 GPU stalls/2min (30% reduction)
- <500ms stalls (30% reduction)
- 70% smoother resize

---

## Next: Test & Decide

1. **Test** on Linux/Wayland
2. **Collect**: `frame-logs.18`, `perf-report.18`
3. **Compare** against Phase 17
4. **Decide**:
   - ✅ Good enough? → DONE
   - ❌ Still sluggish? → Option A (2-3 weeks) or Option D (2-4 weeks)

---

**Risk**: ⭐ Very Low  
**Effort**: 1 day (complete)  
**Status**: ✅ Ready for testing

