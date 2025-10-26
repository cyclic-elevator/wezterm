# Phase 19.2: Assessment Summary

**Date**: 2025-10-26  
**Status**: ✅ **Client Success** / ❌ **Server Regression**

---

## What Happened

### Phase 19.2 Implementation (Complete)

✅ **Priority 1: Replace TabResized→resync** - IMPLEMENTED  
❌ Priority 2: Fix Debounce - NOT IMPLEMENTED (assumed not needed)  
❌ Priority 3: Server Coalescing - NOT IMPLEMENTED (assumed not needed)

**Rationale at the time**: "Redundant detection blocks 99% of events, so broken debounce doesn't matter"

---

## Test Results

### ✅ Success: Client UI Responsiveness

**Before Phase 19.2**:
- Resize latency: 300-500ms (slow, laggy)
- Cause: `resync()` RPC blocking after every TabResized

**After Phase 19.2**:
- Resize latency: ~50-100ms (fast, responsive!) ✅
- Cause: Skipping `resync()` for size-only TabResized

**User feedback**: "Client UI became a lot more responsive!" ✅

---

### ❌ Regression: Server 100% CPU Hang

**Symptom**: Server stuck at 100% CPU for >60 seconds after resize stops

**Evidence**:
```
frame-logs.19.2:
  - 1,963 "RESIZE STORM" blocks (redundant detection working)
  - 2,933 "CLIENTPANE RESIZE" calls (debounce spawning tasks)
  - Events continue for 562ms AFTER mouse drag stops

perf-report.19.2:
  - 86.13% __memmove (terminal rewrap operations)
  - 0.72% Line::set_cell_grapheme (cell updates)
  - 0.23% Line::wrap (line wrapping)
  - Server processing ~3,000 resize RPCs in sequence
```

**Root cause**: The "debounce" implementation doesn't actually debounce:
- Each non-redundant resize spawns a NEW independent async task
- No cancellation mechanism
- All 2,933 tasks fire their RPCs 100ms later
- Server receives 2,933 RPCs and processes them all (100% CPU hang)

---

## Why Phase 19.2 Priority 1 Exposed This Bug

### Before Priority 1

```
Client resize → resync() blocks for 150ms
  ↓
Client can't send more resizes (throttled by blocking)
  ↓
Result: Only 5-10 resize RPCs sent total
  ↓
Server: Fine ✅
```

### After Priority 1

```
Client resize → skips resync() (responsive!)
  ↓
Client can send more resizes immediately (no throttling)
  ↓
Broken debounce spawns 2,933 independent tasks
  ↓
100ms later: All 2,933 tasks fire their RPCs
  ↓
Server: 100% CPU hang for 60+ seconds ❌
```

**The unintended consequence**: Removing `resync()` made the client responsive, but also **removed the accidental throttling** that was preventing the broken debounce from flooding the server.

---

## Why Redundant Detection Didn't Save Us

**Assumption**: "99% of resize events are redundant (same dimensions)"

**Reality with multi-pane windows**:
- 9 panes with different sizes (82x38, 70x37, 80x38)
- When window resizes, panes change to different sizes
- Only ~40% are redundant (same pane, same size)
- **60% are non-redundant** (different panes or different sizes)
- Result: 2,933 non-redundant resize events → 2,933 RPC tasks!

---

## Next Steps

### 🔴 Priority 1: Fix Debounce (CRITICAL)

**Status**: **MUST IMPLEMENT NOW**

**Why**: Server hangs at 100% CPU (production blocker)

**Implementation**: Add shared state + generation counter for cancellation

**Expected result**: 2,933 tasks → 2,932 cancelled → 1 RPC sent ✅

**Details**: See `phase-19.3-action-plan.md`

---

### 🟡 Priority 2: Server Protection (HIGH)

**Status**: **RECOMMENDED**

**Why**: Defense-in-depth against client bugs

**Implementation**: Server-side resize deduplication + rate limiting

**Expected result**: Even if client bugs, server stays responsive ✅

**Details**: See `phase-19.3-action-plan.md`

---

## Performance Trajectory

```
Phase 19.0: Selective Invalidation
  - Lines invalidated: 10,000 → 150 ✅
  - Client: Still slow (resync overhead)
  
Phase 19.2: Redundant Detection
  - Events blocked: 0 → 299/300 ✅
  - Client: Still slow (resync overhead)

Phase 19.2 Priority 1: Remove resync()
  - Client latency: 300-500ms → 50-100ms ✅
  - Server: Exposed broken debounce ❌
  
Phase 19.3: Fix Debounce (NEXT)
  - Client: 50-100ms (stays fast) ✅
  - Server: 1 RPC, 10ms work (fixed!) ✅
  - = COMPLETE SOLUTION 🎉
```

---

## Lessons Learned

### What Went Right

1. ✅ Phase 19.2 Priority 1 was **correct** - eliminating `resync()` was the right fix
2. ✅ Client UI is now **responsive** - users will notice immediate improvement
3. ✅ The analysis and implementation were **sound** - just incomplete

### What Went Wrong

1. ❌ Underestimated importance of **Priority 2** (fix debounce)
2. ❌ Assumed redundant detection would **compensate** for broken debounce
3. ❌ Didn't test with **multi-pane windows** + **server profiling**

### Key Insight

**Removing throttling mechanism** (even accidental ones like blocking `resync()`) **exposes downstream bottlenecks**.

**Principle**: When removing a bottleneck, **always check if it was masking other bugs**.

---

## Timeline

**Phase 19.2 implementation**: 2 hours  
**Testing & discovery**: 30 minutes  
**Root cause analysis**: 1 hour  
**Phase 19.3 plan**: 30 minutes  
**Phase 19.3 implementation**: 4-5 hours (estimated)

**Total**: ~8-9 hours from start to complete fix

---

## Conclusion

### The Good News 🎉

- Client UI is **responsive** (Phase 19.2 succeeded!)
- Root cause **identified** (broken debounce)
- Fix is **straightforward** (add cancellation)
- Expected result: **Both client AND server fast** ✅

### The Bad News ⚠️

- Server currently **unusable** (100% CPU for 60+ seconds)
- Requires **immediate fix** (production blocker)
- Need to implement **Phase 19.3** ASAP

### The Path Forward 🚀

1. **Implement Phase 19.3** (fix debounce + server protection)
2. **Test thoroughly** (multi-pane + server profiling)
3. **Monitor in production** (metrics + alerting)
4. **Document learnings** (for future reference)

**ETA to complete solution**: 4-5 hours

---

**Detailed analysis**: `phase-19.2-server-hang-analysis.md`  
**Action plan**: `phase-19.3-action-plan.md`  
**Ready to implement**: YES ✅

