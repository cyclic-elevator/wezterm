# Phase 16: Disappointing Results - Root Cause Analysis

## Date
2025-10-23

## Status
⚠️ **DISAPPOINTING** - Phase 15 improvements were minor, not transformative

---

## Executive Summary

After implementing Phase 15 (Event Coalescing & Adaptive Frame Rate), the actual performance improvement was **minimal and disappointing**. While the new features are working correctly, they did not eliminate the core problem: **massive GPU stalls** remain the dominant bottleneck.

**Key Finding**: **Event coalescing only reduced 2-8 events per 5s window, not the expected 10x reduction.**

---

## Results Analysis

### Frame Time Performance

**From `frame-logs.14`**:

**Final steady-state** (line 165):
```
avg=7.1ms, median=5.4ms, min=3.0ms, max=21.0ms
p95=12.9ms, p99=18.5ms, variance=17.9ms
```

**Comparison with Phase 14**:
```
Phase 14: avg=6.5ms, median=5.0ms, p95=13.3ms, p99=14.0ms, variance=11.4ms
Phase 15: avg=7.1ms, median=5.4ms, p95=12.9ms, p99=18.5ms, variance=17.9ms
```

**Result**: **WORSE!** 😞
- Average: 6.5ms → 7.1ms (1.09x **slower**)
- Median: 5.0ms → 5.4ms (1.08x **slower**)
- P95: 13.3ms → 12.9ms (1.03x better - insignificant)
- P99: 14.0ms → 18.5ms (1.32x **WORSE**)
- Variance: 11.4ms → 17.9ms (1.57x **WORSE**)

**Verdict**: **Phase 15 made things slightly worse!** ⚠️

---

## Why Did Phase 15 Fail?

### 1. Event Coalescing: Minimal Impact

**From logs** (line 18, 41, 49, 70, 77):
```
Event coalescing: 1 resize events coalesced (2x reduction)
Event coalescing: 2 resize events coalesced (3x reduction)
Event coalescing: 7 resize events coalesced (8x reduction)
```

**Expected**: 10x reduction per window  
**Actual**: 2-8x reduction, but **infrequently**

**Analysis**:
- **Only coalescing 1-7 events per 5-second window**
- Expected to coalesce **dozens** of events during rapid resize
- **The throttle mechanism was already doing its job** in previous phases
- **No 10x improvement because events weren't that frequent**

**Conclusion**: **Event coalescing solved a problem that didn't exist.**

---

### 2. GPU Stalls: Still Massive

**From `frame-logs.14`**:

**Count**: ~57 GPU stalls logged (lines 5, 7, 10, 20-21, etc.)

**Durations**:
- **100-200ms**: 22 stalls (39%)
- **200-400ms**: 19 stalls (33%)
- **400-600ms**: 10 stalls (18%)
- **600-800ms**: 6 stalls (11%)

**Comparison with Phase 14**:
```
Phase 14: ~80 stalls, most 100-400ms, max 954ms
Phase 15: ~57 stalls, most 100-400ms, max 783ms
```

**Improvement**: **1.4x fewer stalls** (80 → 57)

**But**: Stalls are still **massive** and **dominate** the experience!

**Analysis**:
- GPU stalls account for ~50-70% of time during resize
- CPU rendering is fast (5-7ms median)
- **The compositor/GPU is the bottleneck**, not event processing

---

### 3. Adaptive Frame Rate: Counterproductive

**From logs**:
```
Line 8:  Adaptive frame rate: High (60 fps) → Medium (30 fps) (idle: 59.362µs)
Line 13: Adaptive frame rate: Medium (30 fps) → High (60 fps) (idle: 27.522927ms)
Line 14: Adaptive frame rate: High (60 fps) → Medium (30 fps) (idle: 335.215286ms)
```

**Observation**: **Frequent mode switching** during resize!

**Analysis**:
- Switching happens **every few hundred milliseconds**
- Causes frame time inconsistency
- **Adds overhead** instead of reducing it
- The 100ms threshold is **too aggressive** for interactive resize

**Conclusion**: **Adaptive FPS is thrashing**, not helping.

---

### 4. Frame Budgeting: Just Diagnostics

**From logs** (line 26, 40, 59, 71):
```
Frame budget exceeded 8 times in last 5s (budget: 15ms, avg frame: 20.569179ms)
Frame budget exceeded 1 times in last 5s (budget: 15ms, avg frame: 22.339375ms)
```

**Observation**: Budget exceeded occasionally

**Analysis**:
- Only monitoring, not preventing
- Frames go over budget, but we don't skip work
- **No enforcement** = **No benefit**

**Conclusion**: **Frame budgeting is just noise.**

---

## Root Cause Analysis

### The Real Bottleneck: GPU Stalls

**From `frame-logs.14`**:

**Test duration**: ~2.5 minutes (150 seconds)  
**Total stalls**: 57 stalls  
**Total stall time**: ~15-20 seconds (57 stalls × avg 300ms)

**Calculation**:
- **13% of total time** is spent waiting for GPU
- **CPU rendering** is **fast** (median 5.4ms)
- **Event processing** is **fast** (coalescing helps minimally)

**The problem**: **GPU/compositor synchronization is broken!**

---

### Why GPU Stalls Persist

From previous analysis and current data:

1. **Buffer pooling (Phase 12)**: Implemented but **not reducing stalls**
   - May not be used everywhere
   - May not cover all GPU operations

2. **Deferred texture growth (Phase 12)**: Working correctly
   - Only 1 occurrence (line 2-4)
   - Not a significant factor

3. **Damage tracking (Phase 8)**: Implemented but **compositor still slow**
   - Damage regions sent to compositor
   - Compositor still takes 100-700ms to respond
   - **Compositor overhead**, not WezTerm issue

4. **Wayland frame callbacks**: The smoking gun
   - WezTerm waits for compositor's frame callback
   - Compositor takes 100-700ms to signal "ready"
   - **This is the bottleneck!**

---

## Comparison with Phase 14

### Phase 14 vs Phase 15

| Metric | Phase 14 | Phase 15 | Change |
|--------|----------|----------|--------|
| **Avg frame** | 6.5ms | 7.1ms | **1.09x slower** ⚠️ |
| **Median** | 5.0ms | 5.4ms | **1.08x slower** ⚠️ |
| **P95** | 13.3ms | 12.9ms | 1.03x better ✓ |
| **P99** | 14.0ms | 18.5ms | **1.32x worse** ⚠️ |
| **Variance** | 11.4ms | 17.9ms | **1.57x worse** ⚠️ |
| **GPU stalls** | 80 | 57 | 1.4x fewer ✓ |
| **Stall duration** | 100-954ms | 100-783ms | Slightly better ✓ |

**Verdict**: **Minimal improvement, some regressions**

---

## What Went Wrong?

### Wrong Diagnosis

**Assumption**: Event flooding causes slow resize  
**Reality**: GPU stalls cause slow resize

**Phase 15 targeted**: Event processing  
**Should have targeted**: GPU/compositor synchronization

### Wrong Solutions

1. **Event coalescing**: Solved a non-problem
   - Events were already throttled (Phase 1)
   - Only 1-7 events per 5s coalesced
   - **No 10x improvement materialized**

2. **Adaptive FPS**: Made things worse
   - Too aggressive threshold (100ms)
   - Causes mode thrashing during resize
   - **Adds overhead, not benefit**

3. **Frame budgeting**: Only monitoring
   - No enforcement
   - No work skipping
   - **Just diagnostics**

---

## Why Phase 15 Predictions Failed

### Predicted Results (from Phase 15 proposal)

```
Phase 14: avg=6.5ms, p95=13.3ms
Phase 15: avg=4.5ms, p95=6.5ms
Expected: 1.4x faster, 2.0x faster p95
```

### Actual Results

```
Phase 14: avg=6.5ms, p95=13.3ms
Phase 15: avg=7.1ms, p95=12.9ms
Actual: 1.09x SLOWER, 1.03x faster p95
```

### Why the Mismatch?

1. **Overestimated event flooding**
   - Assumed 10 events needed coalescing
   - Actual: Only 1-7 events per 5s window

2. **Underestimated GPU stalls**
   - Thought buffer pooling (Phase 12) solved it
   - Actual: GPU stalls still dominate (13% of time)

3. **Didn't measure compositor overhead**
   - Assumed compositor was fast
   - Actual: Compositor takes 100-700ms per frame callback

---

## The Real Problem: Wayland Frame Callback Model

### How It Currently Works

```
WezTerm renders frame → Request frame callback → Wait for compositor
                                                      ↓
                                        Compositor takes 100-700ms
                                                      ↓
                                        Frame callback arrives
                                                      ↓
                                        WezTerm renders next frame
```

**Problem**: **Synchronous wait** for compositor is **unbounded**!

---

## What Actually Needs to be Fixed

### Priority 1: Compositor Synchronization ⭐⭐⭐⭐⭐

**Problem**: Waiting 100-700ms for frame callbacks

**Solutions**:

1. **Timeout on frame callbacks**
   - Don't wait forever
   - After 50ms, assume callback is lost
   - Continue rendering

2. **Async frame callbacks**
   - Don't block on frame callback
   - Render next frame immediately
   - Compositor catches up when ready

3. **Triple buffering**
   - Keep 3 frames in flight
   - Compositor can lag behind
   - WezTerm doesn't wait

4. **Switch to wl_subsurface + commit without sync**
   - Bypass frame callback entirely
   - Let compositor vsync naturally
   - No explicit synchronization

**Expected Impact**: **Eliminate 13% of time wasted on stalls!**

---

### Priority 2: Buffer Pooling Audit ⭐⭐⭐⭐

**Problem**: Buffer pooling (Phase 12) not reducing stalls enough

**Solutions**:

1. **Audit all vertex buffer allocations**
   - Find which ones don't use pool
   - Add pool usage everywhere

2. **Pool index buffers** (not just vertex buffers)
   - May be significant allocations
   - Easy win

3. **Add detailed metrics**
   - Track pool hit/miss rate per call site
   - Identify gaps

**Expected Impact**: **2-3x reduction in remaining stalls**

---

### Priority 3: Disable Adaptive FPS (for now) ⭐⭐

**Problem**: Thrashing between modes during resize

**Solution**: **Remove adaptive FPS** or increase threshold

```rust
// Change threshold from 100ms to 2 seconds
let new_mode = if idle_time < Duration::from_secs(2) {
    FrameRateMode::High    // Stay in high mode during all interactive use
} else if idle_time < Duration::from_secs(10) {
    FrameRateMode::Medium
} else {
    FrameRateMode::Low     // Only drop when truly idle
};
```

**Expected Impact**: **Eliminate mode thrashing overhead**

---

### Priority 4: Investigate Compositor Behavior ⭐⭐⭐

**Problem**: Compositor (KWin?) takes 100-700ms per frame

**Investigation needed**:

1. **Which compositor?** (KDE Plasma/KWin, GNOME/Mutter, Sway?)
2. **Is this a KWin bug?** (Other apps smooth?)
3. **Compositor settings?** (Vsync, compositing mode)
4. **Driver issues?** (GPU driver version)

**Questions for user**:
- What compositor are you using?
- Do other apps have smooth resize?
- What GPU and driver version?

---

## Revised Understanding: The Bottleneck Hierarchy

### Phase 11-15 Journey: What We Fixed

1. ✅ **Lua overhead** (Phase 0-5): Fixed with caching
2. ✅ **Tab bar computation** (Phase 5): Fixed with TabBarState caching
3. ✅ **Duplicate work** (Phase 4): Fixed with single-pass computation
4. ✅ **Memory copying** (Phase 5): Fixed with state caching
5. ⚠️ **GPU stalls** (Phase 10-12): **Partially** fixed with buffer pooling

### What Remains (The Real Bottleneck)

1. ❌ **Compositor synchronization**: 100-700ms waits (13% of time!)
2. ❌ **GPU stalls**: Still ~57 stalls per 2.5 minutes
3. ❌ **Buffer pooling gaps**: Not used everywhere

---

## Why This Is Disappointing

### Expected (from Phase 15 proposal)

```
"10x fewer renders during resize!"
"6x lower power when idle!"
"1.4x faster frames!"
```

### Actual

```
2-8x event coalescing (but only 1-7 events per 5s!)
No power savings measured
1.09x SLOWER frames (due to adaptive FPS thrashing)
```

### The Gap

**Expected**: Transformative improvement  
**Actual**: Marginal gains, some regressions

**Why**: **Targeted the wrong bottleneck!**

---

## Lessons Learned

### 1. Measure Before Optimizing

**What we did**: Assumed event flooding based on theory  
**What we should have done**: Measured actual event frequency

**Result**: Built a solution for a non-problem

### 2. Don't Trust Previous Optimizations

**What we did**: Assumed Phase 12 (buffer pooling) fixed GPU stalls  
**What we should have done**: Measured GPU stall impact after Phase 12

**Result**: Ignored the real bottleneck

### 3. Game Engine Patterns ≠ Terminal Emulator Needs

**What we did**: Applied game engine patterns (event coalescing, adaptive FPS)  
**Reality**: Terminal emulators have different constraints

**Result**: Solutions that work in games don't work here

### 4. Wayland ≠ Other Platforms

**What we did**: General solutions  
**Reality**: Wayland's frame callback model is unique

**Result**: Need Wayland-specific solutions

---

## Recommendations

### Immediate Actions

1. **Disable adaptive FPS** or increase threshold to 2s
   - Eliminate mode thrashing
   - Restore Phase 14 performance

2. **Profile compositor behavior**
   - Which compositor is user using?
   - Is this a compositor bug?
   - Can we work around it?

3. **Audit buffer pooling**
   - Find gaps in coverage
   - Add metrics per call site

### Strategic Direction

**Stop**: Trying to optimize event processing (it's fast enough)  
**Start**: Fixing compositor synchronization (it's the real bottleneck)

**Focus**: Wayland frame callback model, not generic optimizations

---

## Next Steps (Phase 17 Proposal)

### Phase 17.1: Disable/Fix Adaptive FPS

**Goal**: Eliminate mode thrashing regression  
**Effort**: 1 hour  
**Impact**: Restore Phase 14 performance

### Phase 17.2: Compositor Investigation

**Goal**: Understand compositor behavior  
**Effort**: 1 day  
**Impact**: Identify root cause

### Phase 17.3: Async Frame Callbacks

**Goal**: Don't wait for compositor  
**Effort**: 3-5 days  
**Impact**: **Eliminate 100-700ms stalls!**

### Phase 17.4: Buffer Pooling Audit

**Goal**: Complete buffer pool coverage  
**Effort**: 2-3 days  
**Impact**: Reduce remaining GPU stalls

---

## Performance Target Revision

### Previous Target (Phase 15)

```
avg=4.5ms, p95=6.5ms, p99=8.0ms
```

**Status**: **Not achieved** ❌

### Revised Target (Phase 17)

```
avg=5.0ms, p95=10.0ms, p99=15.0ms
Max stalls: <100ms (vs current 100-700ms)
```

**Focus**: **Eliminate long stalls**, not optimize fast paths

---

## Conclusion

### Phase 15 Status: ⚠️ **FAILED TO DELIVER**

**Implemented**: Event coalescing, adaptive FPS, frame budgeting  
**Expected**: 1.4x faster, 10x fewer renders, 6x power savings  
**Actual**: 1.09x **slower**, minimal event reduction, mode thrashing

**Root Cause**: **Targeted the wrong bottleneck**

### The Real Problem: Compositor Synchronization

- **13% of time** spent waiting for compositor
- **100-700ms stalls** dominate the experience
- **Event processing is fast**, GPU/compositor is slow

### Path Forward: Phase 17

1. Fix adaptive FPS regression (immediate)
2. Investigate compositor behavior (understand)
3. Async frame callbacks (eliminate waits)
4. Buffer pooling audit (reduce stalls)

**Expected**: **Actually fix the problem this time!** 🎯

---

## Acknowledgment

**Phase 15 was a learning experience.**

Sometimes optimization efforts don't pan out. The key is to:
1. Recognize when we're on the wrong track
2. Measure actual impact, not assumed impact
3. Pivot to address the real bottleneck

**Let's fix this in Phase 17!** 💪

