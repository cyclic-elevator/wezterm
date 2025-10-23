# Phase 18: Decision Tree - What's Next?

## Current Status
✅ **Phase 18 (Option B+) Complete**  
🧪 **Ready for Testing**

---

## Decision Tree

```
                    Test Phase 18
                         |
                         |
        ┌────────────────┴────────────────┐
        |                                 |
    Is it 70%+                        Is it 50-70%
    better?                             better?
        |                                 |
        |                                 |
    ┌───┴───┐                         ┌───┴───┐
    |       |                         |       |
   YES     NO                        YES     NO
    |       |                         |       |
    |       |                         |       |
    v       v                         v       v
  DONE   Option A                Accept    Option A
         or D?                    or Fix?   or D?
```

---

## Scenario 1: 70%+ Better ✅

**If resize is noticeably smoother, GPU stalls reduced by 70%+**

### Decision: DONE or CONTINUE?

**Option A: Accept and Move On**
- ✅ Good enough for daily use
- ✅ Wayland resize is acceptable
- ✅ Focus on other features

**Option B: Continue to Perfect (2-3 weeks)**
- Complete Phase 17 wiring
- Achieve smooth 60 FPS
- Match Zed/Chrome smoothness

**Option C: Clean Slate (2-4 weeks)**
- Replace glium with wgpu
- Modern, maintained solution
- Future-proof

**Recommendation**: **Accept and move on** unless smooth 60 FPS is critical.

---

## Scenario 2: 50-70% Better 🤔

**If resize is somewhat smoother, but still noticeable sluggishness**

### Decision: ACCEPT or CONTINUE?

**Accept**: Good enough?
- Significant improvement from baseline
- Low maintenance burden
- Can revisit later if needed

**Continue**: Still not smooth enough?
- Proceed to Option A or D
- Invest 2-4 weeks for smooth 60 FPS
- Higher risk, higher reward

**Recommendation**: **Accept** unless smooth resize is a top priority.

---

## Scenario 3: <50% Better ❌

**If resize is barely improved, still sluggish**

### Decision: OPTION A or OPTION D?

**Option A: Complete Phase 17 Wiring** (2-3 weeks)
- ✅ Leverages existing Phase 17 infrastructure
- ⚠️ High complexity (EGL/Wayland wiring)
- ⚠️ High risk (driver/compositor issues)
- ⚠️ May need to fork glium

**Best if**:
- Want to leverage Phase 17 work
- Willing to deal with EGL complexity
- Can test on multiple drivers/compositors

**Option D: Replace with wgpu** (2-4 weeks)
- ✅ Cleaner, modern solution
- ✅ Handles triple buffering automatically
- ✅ Future-proof
- ⚠️ Large refactor (replace glium throughout)
- ⚠️ May break existing rendering code

**Best if**:
- Want a long-term solution
- Prefer cleaner architecture
- Willing to invest in major refactor

**Recommendation**: **Option D (wgpu)** - cleaner path to smooth 60 FPS.

---

## Detailed Options Comparison

| Aspect | Option A (Phase 17 Wiring) | Option D (wgpu) |
|--------|---------------------------|-----------------|
| **Effort** | 2-3 weeks | 2-4 weeks |
| **Complexity** | Very High (EGL/Wayland FFI) | High (API migration) |
| **Risk** | High (driver-specific bugs) | Medium (well-tested) |
| **Result** | Smooth 60 FPS (if successful) | Smooth 60 FPS (likely) |
| **Maintenance** | High (EGL wiring to maintain) | Low (wgpu maintained) |
| **Future-proof** | No (glium deprecated) | Yes (wgpu active) |
| **Leverages Phase 17** | Yes (frameworks exist) | No (replace everything) |
| **May need fork** | Yes (glium for EGL access) | No (clean API) |

---

## Quick Assessment Guide

### GPU Stall Reduction

**Baseline (Phase 17)**: 52 stalls/2min, 100-750ms duration

| Improvement | Stalls/2min | Duration | Status | Action |
|-------------|-------------|----------|--------|--------|
| **70%+** | <15 | <225ms | 🟢 Excellent | Accept or continue |
| **50-70%** | 15-25 | 225-375ms | 🟡 Good | Accept or continue |
| **30-50%** | 25-35 | 375-525ms | 🟠 Moderate | Likely continue |
| **<30%** | >35 | >525ms | 🔴 Poor | Definitely continue |

### Frame Time Improvement

**Baseline (Phase 17)**: avg 8.5ms, P99 45ms

| Improvement | Avg | P99 | Status | Action |
|-------------|-----|-----|--------|--------|
| **70%+** | <6ms | <25ms | 🟢 Excellent | Accept or continue |
| **50-70%** | 6-7ms | 25-35ms | 🟡 Good | Accept or continue |
| **30-50%** | 7-8ms | 35-40ms | 🟠 Moderate | Likely continue |
| **<30%** | >8ms | >40ms | 🔴 Poor | Definitely continue |

### Subjective Feel

| Feel | Status | Action |
|------|--------|--------|
| **Smooth, barely noticeable lag** | 🟢 Excellent | Accept |
| **Occasional stutter, mostly smooth** | 🟡 Good | Accept or continue |
| **Noticeable lag, but better** | 🟠 Moderate | Continue |
| **Still very sluggish** | 🔴 Poor | Definitely continue |

---

## Testing Checklist

### Must Collect

- [ ] `frame-logs.18` (with GPU stall info)
- [ ] `perf-report.18` (CPU profile)
- [ ] Subjective feel assessment

### Compare Against Phase 17

- [ ] Number of GPU stalls
- [ ] Average/max stall duration
- [ ] Frame time statistics (avg, P95, P99)
- [ ] Tab bar cache hit rate during resize

### Calculate Improvement

```
Improvement % = (Baseline - Phase18) / Baseline * 100

Example:
Baseline: 52 stalls/2min
Phase 18: 18 stalls/2min
Improvement: (52 - 18) / 52 * 100 = 65%
```

---

## My Prediction

### Most Likely Outcome: 50-70% Better 🟡

**Why**:
- Resize throttling (30fps) will significantly reduce GPU load
- Tab bar caching will eliminate most Lua overhead during resize
- Cursor blinking removal is minor but helps
- But: Underlying GPU stall issue (missing triple buffering) remains

**What this means**:
- Resize will be noticeably smoother
- But still occasional stutters (especially on long resizes)
- Good enough for most users
- Power users may still notice lag

**Recommended Action**: **Accept** unless smooth 60 FPS is critical.

---

### Less Likely: 70%+ Better 🟢

**If this happens**:
- The GPU stalls were primarily caused by high event frequency
- Tab bar computation was the main bottleneck
- Rare, but possible!

**Recommended Action**: **Accept and celebrate** 🎉

---

### Possible: <50% Better 🔴

**If this happens**:
- The GPU stalls are due to driver/compositor issues
- WezTerm's rendering approach fundamentally incompatible with Wayland
- Need deep architectural changes

**Recommended Action**: **Option D (wgpu)** - cleaner than trying to fix glium/EGL.

---

## Final Recommendation

### My Personal Advice

1. **Test Phase 18 thoroughly**
   - Collect all metrics
   - Test for at least 5 minutes of resizing
   - Compare against Phase 17

2. **If 50%+ better**: **Accept and move on**
   - Significant improvement achieved
   - Low effort investment (1 day)
   - Can revisit later if needed

3. **If <50% better**: **Consider Option D (wgpu)**
   - Cleaner than Option A
   - More future-proof
   - Better maintained
   - Worth the 2-4 week investment

4. **Avoid Option A unless**:
   - You really want to complete Phase 17
   - You have deep EGL expertise
   - You can test on multiple drivers/compositors

---

## Summary

**Phase 18 (Option B+)**: ✅ Complete, ready for testing

**Next Steps**:
1. Deploy to Linux/Wayland
2. Test resize performance
3. Collect frame-logs.18 and perf-report.18
4. Compare against Phase 17 baseline
5. Use this decision tree to choose next steps

**Most Likely**: 50-70% better → Accept as "good enough" 🎯

**If not good enough**: Proceed with Option D (wgpu) 🚀

