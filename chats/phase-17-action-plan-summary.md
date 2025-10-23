# Phase 17: Action Plan Summary - The Real Fix

## 🎯 The Problem (Finally Understood!)

**Phase 16 revealed**: Phase 15 targeted the wrong bottleneck

**Real bottleneck**: **GPU stalls** (100-700ms waits, 13% of time wasted)

**Root cause**: Missing Wayland best practices used by smooth apps

---

## ✅ What Smooth Apps Do (That WezTerm Doesn't)

From analysis of **Zed, VS Code, Chrome**:

| Feature | Smooth Apps | WezTerm | Impact |
|---------|-------------|---------|--------|
| `wl_surface.frame` | ✅ Yes | ✅ Yes | None (already done!) |
| **Triple buffering** | ✅ Yes | ❌ **No** | **CRITICAL** ⭐⭐⭐⭐⭐ |
| **GPU fences** | ✅ Yes | ❌ **No** | **CRITICAL** ⭐⭐⭐⭐⭐ |
| `wp_presentation_time` | ✅ Yes | ❌ No | High ⭐⭐⭐ |
| Damage tracking | ✅ Yes | ✅ Yes | Done |
| Idle suppression | ✅ Yes | ✅ Yes | Done |

**Missing**: Triple buffering + GPU fences = **GPU stalls!**

---

## 🔍 Why WezTerm Has GPU Stalls

### Current Flow (Broken)

```
Render frame → Wait for GPU (100-700ms!) → Submit → Wait for compositor
                     ↑
               BLOCKS HERE!
```

### Correct Flow (With Triple Buffering)

```
Render to buffer 1 → Render to buffer 2 → Render to buffer 3
                     (GPU works async)    (GPU works async)
No blocking! Smooth 60 FPS!
```

---

## 🚀 Phase 17: The Real Fix

### Phase 17.1: Triple Buffering ⭐⭐⭐⭐⭐ **CRITICAL**

**What**: Create 3 GPU buffers, rotate between them

**Why**: Eliminates GPU blocking stalls (100-700ms → <50ms!)

**Effort**: 2-3 days

**Expected**: **10x shorter stalls!**

---

### Phase 17.2: GPU Fences ⭐⭐⭐⭐⭐ **CRITICAL**

**What**: Use EGL sync fences to prevent GPU queue overflow

**Why**: Prevents over-submission causing stalls

**Effort**: 2-3 days

**Expected**: **2-3x fewer stalls!**

---

### Phase 17.3: `wp_presentation_time` ⭐⭐⭐

**What**: Get feedback on actual present times from compositor

**Why**: Precise vsync alignment, eliminate timing jank

**Effort**: 3-4 days

**Expected**: **Perfect frame timing!**

---

### Phase 17.4: Fix Adaptive FPS ⭐⭐ **QUICK WIN**

**What**: Change threshold from 100ms to 2 seconds

**Why**: Stop mode thrashing during resize

**Effort**: **10 minutes!**

**Expected**: **Restore Phase 14 performance immediately!**

```diff
- if idle_time < Duration::from_millis(100) {
+ if idle_time < Duration::from_secs(2) {
      FrameRateMode::High
  }
```

---

### Phase 17.5: Audit Damage Tracking ⭐⭐

**What**: Verify damage regions are correct

**Why**: Optimize compositor work

**Effort**: 1-2 days

**Expected**: **Minor optimization**

---

## 📊 Expected Results

### Frame Performance

**Phase 16** (current - disappointing):
```
avg=7.1ms, median=5.4ms, p95=12.9ms, p99=18.5ms
GPU stalls: 57 per 2.5min (100-700ms each)
```

**Phase 17** (predicted - transformative):
```
avg=5.0ms, median=4.0ms, p95=8.0ms, p99=12.0ms
GPU stalls: <10 per 2.5min (<50ms each)
```

**Improvements**:
- Average: **1.4x faster** ✅
- P95: **1.6x faster** ✅
- P99: **1.5x faster** ✅
- **Stall frequency**: **5x fewer** ✅
- **Stall duration**: **10x shorter** ✅

---

## 🎯 Why Phase 17 Will Succeed (vs Phase 15)

### Phase 15: Why It Failed ❌

- Targeted event processing (already fast)
- Based on assumptions, not evidence
- Ignored GPU synchronization

### Phase 17: Why It Will Succeed ✅

- Targets **actual bottleneck** (GPU stalls)
- Based on **proven techniques** (Zed, Chrome, VS Code)
- Uses **Wayland best practices**
- Has **quick win** (adaptive FPS fix)

---

## ⏱️ Timeline

**Week 1**: Quick wins + Double buffering  
**Week 2**: GPU fences  
**Week 3**: Presentation time  
**Week 4**: Polish and test  

**Total**: ~3-4 weeks for complete fix

---

## 🎓 Reference Implementations

Study these for implementation details:

1. **Zed**: `gpui/src/platform/linux/wayland/window.rs`
2. **Chromium**: `ui/ozone/platform/wayland/gpu/wayland_surface_gpu.cc`
3. **smithay-client-toolkit**: Wayland protocol examples

---

## ✅ Implementation Order (Recommended)

### Day 1: Immediate Win

```
1. Fix adaptive FPS threshold (10 minutes)
   → Restore Phase 14 performance immediately
```

### Week 1-2: Critical Fixes

```
2. Implement double/triple buffering (2-3 days)
   → Eliminate GPU blocking stalls
   
3. Implement GPU fences (2-3 days)
   → Prevent GPU queue overflow
```

### Week 3: Polish

```
4. Add wp_presentation_time (3-4 days)
   → Perfect vsync alignment
   
5. Audit damage tracking (1-2 days)
   → Verify correctness
```

---

## 🎉 Expected User Experience

### Before (Phase 16)

❌ Sluggish resize  
❌ Frequent 100-700ms pauses  
❌ Jank and stuttering  
❌ Feels broken  

### After (Phase 17)

✅ **Smooth 60 FPS resize**  
✅ **No visible pauses**  
✅ **Feels like Chrome/Zed**  
✅ **World-class Wayland support!**  

---

## 💪 Confidence Level

**Phase 15**: Low (targeted wrong bottleneck)  
**Phase 17**: **HIGH!** (targeting proven techniques)

**Why confident**:
1. ✅ We know the bottleneck (GPU stalls measured)
2. ✅ We know the solution (triple buffering + fences)
3. ✅ We have references (Zed, Chrome do this)
4. ✅ Quick win available (adaptive FPS fix)

---

## 📝 Summary

**Problem**: GPU stalls (100-700ms) due to missing buffering + fences  
**Solution**: Implement Wayland best practices (triple buffering + GPU fences)  
**Timeline**: 3-4 weeks  
**Expected**: **Finally smooth like Chrome and Zed!** 🚀

---

**Let's do this!** The real fix is within reach! 💪

