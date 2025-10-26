# Comprehensive Code Changes Review
## Analysis of Changes Since Commit 64f2907c6

**Generated**: 2025-10-26
**Baseline Commit**: 64f2907c635b7bab407ac300b2261c77a85c1c8e (docs: changelog for PR #7283)
**Analyzed Commits**: 41 commits across 19 phases
**Lines Changed**: 4,218 lines of Rust code, 579,128 total insertions (includes logs/reports)

---

## Executive Summary

This analysis covers a comprehensive Wayland rendering optimization effort spanning **19 phases** over approximately 3-4 weeks. The changes primarily target **resize performance issues** on Wayland, with the project evolving from targeting Lua/callback overhead to GPU synchronization issues, and finally to remote mux connection bottlenecks.

**Key Outcomes**:
- ✅ **Lua overhead**: Reduced from 16% to ~3-5% CPU
- ✅ **Tab/window title caching**: <1ms cache hits vs 10-20ms misses
- ✅ **Selective invalidation**: 150 lines vs 10,000+ lines on resize
- ⚠️ **GPU stalls**: Identified but not fully resolved (100-700ms stalls remain)
- ✅ **Remote mux resize**: Fixed redundant detection, selective invalidation
- ❌ **Debounce**: Implemented but broken (spawns tasks without cancellation)
- ✅ **Wayland infrastructure**: Triple buffering, GPU fences, presentation time protocols added

**Overall Assessment**: **MIXED SUCCESS** - significant performance improvements in some areas, but several critical issues remain unresolved or partially broken.

---

## Table of Contents

1. [Change Categories by Type](#change-categories-by-type)
2. [Invasiveness Analysis](#invasiveness-analysis)
3. [Effectiveness Analysis](#effectiveness-analysis)
4. [Risk and Secondary Impact Analysis](#risk-and-secondary-impact-analysis)
5. [Maintainer Acceptance Likelihood](#maintainer-acceptance-likelihood)
6. [Cross-Project Impact Assessment](#cross-project-impact-assessment)
7. [Detailed Change Breakdown](#detailed-change-breakdown)
8. [Recommendations](#recommendations)

---

## Change Categories by Type

### Category A: Lua Callback Optimization (Phases 0-5)
**Files**: `wezterm-gui/src/{callback_cache.rs, tab_title_cache.rs, lua_ser_cache.rs, tabbar.rs, termwindow/mod.rs}`

**Changes**:
- Generation-based callback result caching (tab titles, window titles)
- Lua serialization caching
- Event throttling (16ms window)

**Status**: ✅ **Complete and Working**

---

### Category B: Wayland Rendering Infrastructure (Phases 8-17)
**Files**: `window/src/os/wayland/{window.rs, triplebuffer.rs, gpufence.rs, presentation.rs}`

**Changes**:
- Damage tracking integration (`wl_surface.damage_buffer`)
- Triple buffering implementation
- GPU fence synchronization (`eglCreateSyncKHR`)
- `wp_presentation_time` protocol support
- Event coalescing (16ms window)
- Adaptive frame rate (High/Medium/Low modes: 60/30/10 FPS)

**Status**: ✅ **Complete** but ⚠️ **GPU stalls not fully resolved**

---

### Category C: Remote Mux Optimization (Phases 18-19.4)
**Files**: `wezterm-client/src/{client.rs, pane/clientpane.rs, pane/renderable.rs}`, `mux/src/{lib.rs, tab.rs}`, `codec/src/lib.rs`, `wezterm-mux-server-impl/src/{dispatch.rs, sessionhandler.rs}`

**Changes**:
- Enhanced `TabResized` PDU with `size` and `topology_changed` fields
- Client-side redundant resize detection (early return for identical sizes)
- Selective viewport invalidation (150 lines vs 10,000+)
- "Debounced" resize RPC (100ms delay) - **BROKEN IMPLEMENTATION**
- TabResized → resync() optimization (skip resync for size-only changes)
- Fetch generation tracking for stale fetch discarding

**Status**: ⚠️ **Partially Working** - redundant detection and selective invalidation work, but debounce is broken and resync optimization incomplete

---

### Category D: GPU Resource Management (Phases 10-13)
**Files**: `wezterm-gui/src/{bufferpool.rs, renderstate.rs, termwindow/render/paint.rs}`

**Changes**:
- Vertex buffer pooling (reuse instead of allocate)
- Deferred texture atlas growth
- Frame budgeting (15ms target)
- GPU stall detection and logging

**Status**: ✅ **Complete** but ⚠️ **Effectiveness limited** (stalls persist)

---

## Invasiveness Analysis

### **Category: Low Invasiveness** (Easy to Accept)

#### 1. Lua Callback Caching (Phases 0-5)
**Invasiveness**: ⭐ **Very Low**

**Why**:
- Self-contained new modules (`callback_cache.rs`, `tab_title_cache.rs`)
- Wrapper pattern around existing callbacks
- Graceful fallback to non-cached path on failure
- No breaking changes to APIs
- No changes to external protocols

**Risk**: Minimal - cache bugs would just cause stale UI, easily noticed and fixed

---

#### 2. Vertex Buffer Pooling (Phase 12)
**Invasiveness**: ⭐ **Very Low**

**Why**:
- Self-contained new module (`bufferpool.rs`)
- Internal optimization, no API changes
- Uses existing allocation paths with pooling layer
- Can be disabled by clearing pool

**Risk**: Low - pool leaks would just waste memory, not cause crashes

---

#### 3. Frame Budgeting (Phase 15)
**Invasiveness**: ⭐ **Very Low**

**Why**:
- Monitoring only, doesn't affect rendering logic
- New fields added to structs, no behavioral changes
- Logs warnings, doesn't skip work
- Easy to remove if problematic

**Risk**: None - pure instrumentation

---

### **Category: Medium Invasiveness** (Requires Review)

#### 4. Selective Viewport Invalidation (Phase 19)
**Invasiveness**: ⭐⭐ **Medium**

**Why**:
- Modifies core rendering path (`make_viewport_stale` vs `make_all_stale`)
- Changes which lines are fetched from server
- Affects correctness if margin calculation is wrong
- Potential for rendering artifacts if viewport tracking is buggy

**Risk**: Medium - incorrect invalidation could cause stale content or missing updates

---

#### 5. Event Coalescing (Phase 15)
**Invasiveness**: ⭐⭐ **Medium**

**Why**:
- Changes event processing timing (16ms delay)
- Defers resize application
- Could affect perceived responsiveness
- Wayland-specific, doesn't affect other platforms

**Risk**: Medium - incorrect coalescing could drop important events or cause lag

---

#### 6. Damage Tracking (Phase 8)
**Invasiveness**: ⭐⭐ **Medium**

**Why**:
- New protocol usage (`wl_surface.damage_buffer`)
- Requires correct region calculation
- Affects compositor behavior
- Falls back to full damage if regions empty

**Risk**: Medium - incorrect damage regions could cause rendering artifacts on some compositors

---

### **Category: High Invasiveness** (Needs Careful Review)

#### 7. Enhanced TabResized Protocol (Phase 19.2)
**Invasiveness**: ⭐⭐⭐ **High**

**Why**:
- **Modifies wire protocol** (adds fields to `TabResized` PDU)
- **Breaking change** - requires server/client version match
- Changes notification semantics (size vs topology)
- Affects all remote mux connections
- Requires codec serialization changes

**Risk**: High - protocol mismatch could cause:
- Connection failures
- Deserialization errors
- Backward compatibility issues

**Mitigation**: Needs versioning or capability negotiation

---

#### 8. TabResized → resync() Optimization (Phase 19.2)
**Invasiveness**: ⭐⭐⭐ **High**

**Why**:
- Changes core client synchronization logic
- Skips expensive `list_panes()` RPC for size-only changes
- Requires correct `topology_changed` flag from server
- Incorrect flag could cause:
  - Stale pane list
  - Missing split/zoom updates
  - UI desynchronization

**Risk**: High - bugs could cause invisible panes, wrong splits, or desynced state

---

#### 9. Triple Buffering (Phase 16-17)
**Invasiveness**: ⭐⭐⭐ **High**

**Why**:
- Fundamental change to rendering architecture
- Creates multiple EGL surfaces
- Changes buffer management lifecycle
- Wayland-specific, significant code additions
- Complex state management (buffer rotation)

**Risk**: High - bugs could cause:
- Buffer leaks
- Rendering to wrong buffer
- GPU memory exhaustion
- Crashes on some drivers

---

#### 10. GPU Fence Synchronization (Phase 17)
**Invasiveness**: ⭐⭐⭐⭐ **Very High**

**Why**:
- **Platform-specific** (requires EGL fence extensions)
- Changes GPU synchronization model
- Adds blocking waits in critical paths
- **Timeout handling** could skip frames or cause hangs
- May not be available on all systems

**Risk**: Very High - bugs could cause:
- GPU timeouts and hangs
- Skipped frames
- Driver crashes on unsupported hardware
- Undefined behavior with incomplete fence support

**Critical Issue**: No fallback for systems without `EGL_KHR_fence_sync`

---

### **Category: Critical Invasiveness** (Major Architecture Change)

#### 11. wp_presentation_time Protocol (Phase 17)
**Invasiveness**: ⭐⭐⭐⭐ **Very High**

**Why**:
- **Adds new Wayland protocol dependency**
- **Compositor-specific** - not all compositors support it
- Changes frame timing model fundamentally
- Predicts vsync and schedules rendering accordingly
- **Requires protocol version negotiation**
- Adds new failure modes (protocol not available)

**Risk**: Very High - issues include:
- **Compositor compatibility** - some compositors don't support presentation-time
- **Fallback complexity** - must gracefully degrade
- Timing bugs could cause stuttering
- Incorrect vsync prediction could make things worse

**Critical Issue**: No guarantee protocol is available (optional Wayland extension)

---

## Effectiveness Analysis

### **Highly Effective** (Clear Improvements)

#### 1. Tab Title Caching (Phase 0) ⭐⭐⭐⭐⭐
**Effectiveness**: **Excellent**

**Evidence**:
- Tab bar rendering: **0.06% CPU** (from ~7%)
- Cache hit: **<1ms**, Cache miss: **10-20ms** (same as before)
- Profiling shows tab title functions negligible

**Impact**: **~100x improvement** for cached titles

**Confidence**: High - profiling data confirms effectiveness

---

#### 2. Redundant Resize Detection (Phase 19.2) ⭐⭐⭐⭐⭐
**Effectiveness**: **Excellent**

**Evidence**:
- Blocks **299 out of 300** redundant resize events (99%)
- Breaks resize feedback loop
- Logs show "🔴 RESIZE STORM" messages confirming blocks
- Server receives only **1 resize RPC** instead of 60+

**Impact**: **60x reduction** in resize RPCs (when working)

**Confidence**: High - logs and tests confirm

---

#### 3. Selective Viewport Invalidation (Phase 19) ⭐⭐⭐⭐⭐
**Effectiveness**: **Excellent**

**Evidence**:
- Invalidates **150 lines** vs **10,000+** before
- **67-100x reduction** in lines marked stale
- Performance no longer degrades with scrollback size
- Logs confirm viewport + 100 line margin

**Impact**: **Massive improvement** for large scrollback

**Confidence**: High - logs and analysis confirm

---

#### 4. Callback Caching (Phases 1-5) ⭐⭐⭐⭐
**Effectiveness**: **Very Good**

**Evidence**:
- Lua overhead: **16% → 3-5% CPU**
- Window title (cache hit): **<1ms**
- Status updates: Cached and efficient
- Profiling shows reduced FFI overhead

**Impact**: **67-80% reduction** in Lua overhead

**Confidence**: High - profiling confirms

---

### **Moderately Effective** (Some Improvements)

#### 5. Vertex Buffer Pooling (Phase 12) ⭐⭐⭐
**Effectiveness**: **Moderate**

**Evidence**:
- Reduces GPU allocations (reuses buffers)
- Stats show ~60-70% buffer reuse rate
- Helps but doesn't eliminate GPU stalls
- Logs show "Buffer pool: reused buffer" messages

**Impact**: **Moderate** - reduces allocation overhead, but stalls persist

**Confidence**: Medium - helps but not transformative

**Issue**: GPU stalls still occur (100-700ms), indicating allocation isn't the only bottleneck

---

#### 6. Event Coalescing (Phase 15) ⭐⭐⭐
**Effectiveness**: **Moderate**

**Evidence**:
- Logs show "Event coalescing: X resize events coalesced" messages
- Reduces render calls during rapid resize
- **BUT**: Adds 16ms latency to all events
- **Effectiveness masked** by redundant detection

**Impact**: **5-10x reduction** in render calls (when effective)

**Concern**: Perceived responsiveness reduction from delay

---

#### 7. Adaptive Frame Rate (Phase 15) ⭐⭐
**Effectiveness**: **Limited**

**Evidence**:
- Drops to 10 FPS when idle (power savings)
- **BUT**: Threshold too aggressive (100ms)
- Causes mode thrashing during normal use
- Phase 17 attempted fix (increased threshold)

**Impact**: **Good for power savings**, **Bad for responsiveness**

**Issue**: Needs tuning - current thresholds cause thrashing

---

### **Ineffective or Broken** (Doesn't Work as Intended)

#### 8. "Debounced" Resize RPC (Phase 19.3) ❌
**Effectiveness**: **BROKEN**

**Evidence**:
```rust
// Each resize spawns NEW independent task
promise::spawn::spawn(async move {
    Timer::after(Duration::from_millis(100)).await;
    client.client.resize(Resize { ... }).await  // NO CANCELLATION!
}).detach();
```

**Problem**:
- **No shared state** to track pending resize
- **No cancellation** of previous tasks
- **All tasks fire** their RPCs 100ms later
- **This is a DELAY, not a DEBOUNCE**

**Impact**: **NO EFFECT** - all RPCs still sent, just delayed 100ms

**Masked by**: Redundant detection catches most events, so only 1-2 tasks spawn in practice

**Root Cause**: Misunderstanding of async debounce pattern

**Confidence**: Very High - code review confirms broken implementation

---

#### 9. Triple Buffering (Phase 16-17) ⭐
**Effectiveness**: **Uncertain**

**Evidence**:
- Implementation present in `triplebuffer.rs`
- **BUT**: GPU stalls persist (100-700ms)
- Phase 16 logs still show frequent stalls
- Unclear if triple buffering is actually being used

**Issue**: **May not be correctly integrated** or drivers don't support it well

**Confidence**: Low - implementation present but stalls persist, suggesting incomplete integration or driver issues

---

#### 10. GPU Fence Synchronization (Phase 17) ⭐
**Effectiveness**: **Uncertain**

**Evidence**:
- Implementation present in `gpufence.rs`
- **BUT**: GPU stalls persist
- No clear "before/after" profiling data
- May have compatibility issues

**Issue**: **Effectiveness unproven** - needs profiling to confirm

**Confidence**: Low - implementation present but impact unclear

---

#### 11. wp_presentation_time (Phase 17) ⭐
**Effectiveness**: **Unknown**

**Evidence**:
- Implementation present in `presentation.rs`
- **BUT**: No test results showing improvement
- Not all compositors support it
- May be inactive on test system

**Issue**: **Effectiveness not demonstrated**

**Confidence**: Very Low - no evidence of impact

---

## Risk and Secondary Impact Analysis

### **Low Risk** (Safe Changes)

#### Lua Caching Infrastructure
**Risk**: ⭐ **Very Low**

**Primary Risk**:
- Stale cached values (fixed by cache invalidation on config reload)

**Secondary Benefits**:
- Reduced GC pressure (fewer Lua objects created)
- More predictable frame times (cache hits are consistent <1ms)
- Better battery life (less CPU churn)

**Secondary Risks**:
- Memory usage increased (caches stored in memory)
- Cache invalidation bugs could cause confusion (wrong title displayed)

**Mitigation**: Cache size limits, generation-based invalidation

---

#### Frame Budgeting & Instrumentation
**Risk**: ⭐ **Very Low**

**Primary Risk**: None (monitoring only)

**Secondary Benefits**:
- Better diagnostics for performance issues
- Early warning system for regressions
- Helps identify bottlenecks

**Secondary Risks**: None

---

### **Medium Risk** (Requires Testing)

#### Selective Viewport Invalidation
**Risk**: ⭐⭐ **Medium**

**Primary Risk**:
- **Incorrect margin calculation** → missed updates outside viewport
- **Viewport tracking bugs** → wrong lines invalidated
- **Scrolling artifacts** if invalidation doesn't track scroll

**Secondary Benefits**:
- Massive performance improvement for large scrollback
- Scales better with terminal size

**Secondary Risks**:
- **Subtle rendering bugs** hard to catch (user might not notice stale line 5000)
- **Edge cases**: split views, zoomed panes, rapid scrolling
- **Testing burden**: need comprehensive test cases

**Mitigation**: Conservative margin (100 lines), thorough testing

---

#### Event Coalescing
**Risk**: ⭐⭐ **Medium**

**Primary Risk**:
- **Perceived lag** (16ms delay on all resize events)
- **Dropped events** if coalescing logic buggy
- **Accumulated state** if final event represents multiple changes

**Secondary Benefits**:
- Smoother resize experience (fewer frame drops)
- Reduced GPU load

**Secondary Risks**:
- **User perception** - some users prefer immediate response over smoothness
- **Platform-specific** - only Wayland affected
- **Testing complexity** - timing-dependent behavior

**Mitigation**: Configurable threshold, careful testing

---

#### Enhanced TabResized Protocol
**Risk**: ⭐⭐⭐ **High**

**Primary Risk**:
- **Wire protocol change** - backward compatibility issues
- **Version mismatch** - client/server version requirements
- **Deserialization errors** - crashes if versions incompatible

**Secondary Benefits**:
- **Future-proof** - distinguishes size vs topology changes
- **Better semantics** - explicit about change type
- **Optimization potential** - enables targeted updates

**Secondary Risks**:
- **Migration complexity** - users with mixed versions
- **Testing burden** - must test all version combinations
- **Deployment coupling** - must upgrade server and client together
- **Rollback difficulty** - hard to revert protocol changes

**Critical Concern**: No versioning or capability negotiation visible in code

**Mitigation Needed**:
- Protocol versioning
- Backward compatibility mode
- Graceful degradation for old servers/clients

---

### **High Risk** (Major Concerns)

#### Triple Buffering + GPU Fences
**Risk**: ⭐⭐⭐⭐ **Very High**

**Primary Risk**:
- **Driver compatibility** - not all drivers support EGL fences
- **Buffer leaks** - if rotation logic buggy
- **GPU hangs** - if fence logic incorrect
- **Platform variance** - works on Intel, breaks on NVIDIA
- **Wayland compositor variance** - KWin works, Sway breaks

**Secondary Benefits**:
- **Should eliminate GPU stalls** (if working correctly)
- **Better GPU utilization**
- **Smoother rendering**

**Secondary Risks**:
- **Increased memory usage** (3 buffers instead of 1-2)
- **Increased GPU VRAM usage**
- **Complexity explosion** - hard to debug buffer lifetime issues
- **Fallback path complexity** - must handle unsupported systems
- **Testing nightmare** - must test on:
  - Multiple drivers (Intel, NVIDIA, AMD)
  - Multiple compositors (KWin, Sway, GNOME, Hyprland)
  - Multiple GPUs (integrated, discrete, multi-GPU)

**Critical Issue**: **No evidence these changes fixed GPU stalls in practice**

**Logs show**: GPU stalls persist in Phase 16-17 despite implementation

**Hypothesis**: Either:
1. Implementation not fully integrated
2. Driver support insufficient
3. Compositor doesn't cooperate
4. Root cause is something else entirely

**Mitigation Needed**:
- Comprehensive driver compatibility matrix
- Runtime detection of EGL fence support
- Graceful fallback to double buffering
- Extensive multi-platform testing

---

#### wp_presentation_time Protocol
**Risk**: ⭐⭐⭐⭐ **Very High**

**Primary Risk**:
- **Compositor support optional** - Weston, KWin support it; Sway doesn't
- **Protocol complexity** - timing prediction is hard
- **Timing bugs** - wrong vsync prediction makes things worse
- **Fallback requirement** - must work without presentation-time

**Secondary Benefits**:
- **Perfect vsync alignment** (when working)
- **Reduced latency** (predictive scheduling)
- **Better battery life** (GPU wakes at optimal time)

**Secondary Risks**:
- **Wayland ecosystem fragmentation** - different compositors behave differently
- **Testing complexity** - must test:
  - Compositors with presentation-time support
  - Compositors without (fallback path)
  - Different refresh rates (60Hz, 120Hz, 144Hz, variable)
- **Timing edge cases** - vsync changes, display hotplug, mode switches
- **Fallback code paths** - must maintain two timing systems

**Critical Concern**: **No evidence it's actually working or helping**

**Mitigation Needed**:
- Protocol availability detection at runtime
- Graceful fallback to frame callback timing
- Extensive compositor compatibility testing

---

## Maintainer Acceptance Likelihood

### **Very Likely to Accept** (85-100% chance)

#### Category: Self-Contained Optimizations

1. **Lua Callback Caching** (Phases 0-5) - **95%**
   - **Why**: Clear performance win, self-contained, no breaking changes
   - **Concerns**: None major
   - **Recommendation**: Clean up dead code warnings first

2. **Frame Budgeting & Instrumentation** (Phase 15) - **90%**
   - **Why**: Monitoring only, helps diagnose issues
   - **Concerns**: Log spam if thresholds wrong
   - **Recommendation**: Make log levels configurable

3. **Selective Viewport Invalidation** (Phase 19) - **85%**
   - **Why**: Massive performance win, well-understood optimization
   - **Concerns**: Correctness in edge cases
   - **Recommendation**: Add comprehensive tests, make margin configurable

---

### **Likely to Accept** (60-85% chance)

#### Category: Reasonable Improvements with Caveats

4. **Redundant Resize Detection** (Phase 19.2) - **80%**
   - **Why**: Obvious optimization, breaks feedback loop
   - **Concerns**: Log message level (ERROR is too aggressive)
   - **Recommendation**: Change log level to DEBUG or TRACE

5. **Vertex Buffer Pooling** (Phase 12) - **75%**
   - **Why**: Reasonable optimization, self-contained
   - **Concerns**: Memory overhead, complexity
   - **Recommendation**: Make pool size configurable, add metrics

6. **Damage Tracking** (Phase 8) - **70%**
   - **Why**: Compositor efficiency improvement
   - **Concerns**: Correctness of damage regions, compositor compatibility
   - **Recommendation**: Add validation, extensive testing

7. **Event Coalescing** (Phase 15) - **65%**
   - **Why**: Smoothness improvement
   - **Concerns**: Perceived lag (16ms), platform-specific
   - **Recommendation**: Make threshold configurable

---

### **Maybe** (35-60% chance)

#### Category: Useful But Needs Work

8. **Adaptive Frame Rate** (Phase 15) - **50%**
   - **Why**: Power savings are good
   - **Concerns**: Threshold too aggressive, mode thrashing
   - **Recommendation**: Make thresholds configurable, increase defaults significantly (2s → 10s)

9. **Enhanced TabResized Protocol** (Phase 19.2) - **45%**
   - **Why**: Good idea, better semantics
   - **Concerns**: **Wire protocol change**, backward compatibility
   - **Recommendation**: Add protocol versioning, implement backward compatibility

10. **TabResized → resync() Optimization** (Phase 19.2) - **40%**
    - **Why**: Performance improvement for remote mux
    - **Concerns**: **Correctness critical**, depends on Enhanced TabResized protocol
    - **Recommendation**: Extensive testing, ensure topology_changed flag is always correct

---

### **Unlikely to Accept** (15-35% chance)

#### Category: Needs Significant Rework

11. **"Debounced" Resize RPC** (Phase 19.3) - **30%**
    - **Why**: Good intent, but **implementation is broken**
    - **Concerns**: **Doesn't actually debounce**, just delays
    - **Recommendation**: **Rewrite with proper cancellation**:
      ```rust
      // Need shared state
      pending_resize: Arc<Mutex<Option<PendingResize>>>,

      // Cancel previous timer, schedule new one
      if let Some(prev) = pending.take() {
          prev.cancel();  // Actually cancel!
      }
      *pending = Some(PendingResize { size, timer_handle });
      ```

12. **Triple Buffering** (Phase 16-17) - **25%**
    - **Why**: Good optimization in theory
    - **Concerns**: **No evidence it works**, **complexity**, **driver compatibility**
    - **Recommendation**: Provide profiling data showing improvement, extensive driver testing

13. **GPU Fence Synchronization** (Phase 17) - **20%**
    - **Why**: Potentially useful
    - **Concerns**: **Driver compatibility critical**, **no evidence of improvement**
    - **Recommendation**: Runtime detection of EGL_KHR_fence_sync, graceful fallback, prove effectiveness with profiling

---

### **Very Unlikely to Accept** (0-15% chance)

#### Category: Questionable or Unproven

14. **wp_presentation_time Protocol** (Phase 17) - **15%**
    - **Why**: Advanced optimization
    - **Concerns**: **Compositor support limited**, **complexity**, **no evidence it helps**, **fallback required**
    - **Recommendation**: Only include if:
      - Proven to work on major compositors (KWin, Weston)
      - Graceful fallback implemented and tested
      - Clear performance benefit demonstrated

---

## Cross-Project Impact Assessment

### **Minimal Impact** (Isolated Changes)

#### Lua Caching Infrastructure
**Affected Areas**: GUI layer only

**Impact**:
- ✅ **CLI tools**: Unaffected
- ✅ **Remote mux server**: Unaffected
- ✅ **Non-Wayland platforms**: Unaffected

**Dependencies**: None

**Test Scope**: GUI rendering tests only

---

#### Vertex Buffer Pooling
**Affected Areas**: GUI rendering only

**Impact**:
- ✅ **CLI tools**: Unaffected
- ✅ **Remote mux**: Unaffected
- ✅ **Non-GPU platforms**: Gracefully skipped

**Dependencies**: OpenGL/EGL context

**Test Scope**: GPU rendering tests

---

### **Moderate Impact** (Platform-Specific)

#### Wayland-Specific Changes
**Affected Areas**: Wayland window backend only

**Impact**:
- ✅ **macOS**: Unaffected
- ✅ **Windows**: Unaffected
- ✅ **X11**: Unaffected
- ⚠️ **Wayland**: All changes apply

**Dependencies**: Wayland protocols, compositor behavior

**Test Scope**:
- Multiple compositors (KWin, GNOME, Sway, Hyprland)
- Multiple drivers (Intel, NVIDIA, AMD)

**Risk**: Regression risk isolated to Wayland

---

### **High Impact** (Cross-Cutting Changes)

#### Remote Mux Protocol Changes
**Affected Areas**: **Client, Server, and Protocol**

**Impact**:
- ⚠️ **CLI client**: Must understand new TabResized format
- ⚠️ **GUI client**: Must understand new TabResized format
- ⚠️ **Mux server**: Must send new TabResized format
- ⚠️ **Wire protocol**: **Serialization format changed**

**Dependencies**:
- `codec` crate (serialization)
- `mux` crate (notification system)
- `wezterm-client` crate (client handling)
- `wezterm-mux-server-impl` crate (server handling)

**Backward Compatibility**: **BROKEN**

**Migration Path**: **UNCLEAR**

**Test Scope**:
- Client-server version combinations
- Mixed version scenarios
- Upgrade/downgrade paths
- Fallback behavior

**Critical Issue**: **No versioning mechanism visible**

**Recommendation**: Add protocol versioning before merge:
```rust
pub struct TabResizedInfo {
    pub tab_id: TabId,
    pub size: Option<TerminalSize>,      // Optional for backward compat
    pub topology_changed: bool,           // Default to true if missing
    #[serde(default)]
    pub protocol_version: u32,            // Add versioning
}
```

---

#### Selective Viewport Invalidation
**Affected Areas**: **Client rendering path**

**Impact**:
- ⚠️ **All client panes**: Changed invalidation logic
- ⚠️ **Remote mux**: Fetch requests changed
- ⚠️ **Local panes**: Might have slightly different code path

**Dependencies**:
- `wezterm-client/src/pane/renderable.rs` (core rendering)
- `wezterm-client/src/pane/clientpane.rs` (pane interface)

**Risk**: **Rendering bugs could affect all panes, not just remote**

**Test Scope**:
- Local panes (ensure still work)
- Remote panes (primary target)
- Split views, zoomed panes
- Rapid scrolling
- Large scrollback (10k+ lines)

---

## Detailed Change Breakdown

### Phase 0-5: Lua Callback Optimization

**Goal**: Reduce Lua FFI overhead

**Files Modified**:
- `wezterm-gui/src/callback_cache.rs` (NEW, 371 lines)
- `wezterm-gui/src/tab_title_cache.rs` (NEW, 335 lines)
- `wezterm-gui/src/lua_ser_cache.rs` (NEW, 290 lines)
- `wezterm-gui/src/tabbar.rs` (modified, integrated caching)
- `wezterm-gui/src/termwindow/mod.rs` (modified, window title caching)
- `window/src/os/wayland/window.rs` (modified, event throttling)

**Key Changes**:
1. **Generation-based caching** - cache invalidation via generation counter
2. **WindowTitleKey** - caches `format-window-title` Lua callback results
3. **TabTitleKey** - caches tab title computations
4. **Lua serialization caching** - reduces Rust→Lua conversion overhead
5. **Event throttling** - 16ms window for resize events (Wayland)

**Effectiveness**: ✅ **Excellent** (Lua overhead 16% → 3-5%)

**Maintainer Acceptance**: **95%** (self-contained, proven win)

**Recommendation**: ✅ **Accept with minor cleanup** (remove unused code warnings)

---

### Phase 8: Damage Tracking

**Goal**: Reduce compositor workload

**Files Modified**:
- `window/src/os/wayland/window.rs` (modified, ~50 lines)

**Key Changes**:
1. `dirty_regions: RefCell<Vec<Rect>>` field added
2. `mark_dirty()` and `mark_all_dirty()` methods
3. `surface().damage_buffer(x, y, width, height)` calls in `do_paint()`

**Effectiveness**: ⭐⭐⭐ **Good** (compositor overhead reduced)

**Maintainer Acceptance**: **70%** (reasonable optimization)

**Concerns**: Damage region correctness

**Recommendation**: ✅ **Accept after validation** (test on multiple compositors)

---

### Phase 10-13: GPU Resource Management

**Goal**: Reduce GPU allocation overhead and detect stalls

**Files Modified**:
- `wezterm-gui/src/bufferpool.rs` (NEW, 147 lines)
- `wezterm-gui/src/renderstate.rs` (modified, integrated pooling)
- `wezterm-gui/src/termwindow/mod.rs` (modified, frame budgeting)
- `wezterm-gui/src/termwindow/render/paint.rs` (modified, budget checks)

**Key Changes**:
1. **Buffer pooling** - reuse vertex buffers instead of allocating
2. **Deferred texture growth** - move expensive ops off critical path
3. **Frame budgeting** - 15ms target, log warnings on exceed
4. **GPU stall detection** - log when waiting > 100ms

**Effectiveness**: ⭐⭐⭐ **Moderate** (helps but doesn't eliminate stalls)

**Maintainer Acceptance**: **75%** (reasonable optimizations)

**Concerns**: Complexity, effectiveness uncertain

**Recommendation**: ⚠️ **Accept pooling, defer fence/triple buffering** until proven effective

---

### Phase 15: Event Coalescing & Adaptive Frame Rate

**Goal**: Reduce event processing overhead and power usage

**Files Modified**:
- `window/src/os/wayland/window.rs` (modified, event coalescing)
- `wezterm-gui/src/termwindow/mod.rs` (modified, adaptive frame rate)
- `wezterm-gui/src/termwindow/render/paint.rs` (modified, frame rate update)

**Key Changes**:
1. **Event coalescing** - 16ms window, accumulate resize events
2. **FrameRateMode enum** - High (60fps), Medium (30fps), Low (10fps)
3. **Activity tracking** - keyboard, mouse, terminal output
4. **Adaptive mode selection** - based on idle time

**Effectiveness**:
- Event coalescing: ⭐⭐⭐ **Moderate** (reduces event flood)
- Adaptive FPS: ⭐⭐ **Limited** (threshold too aggressive)

**Maintainer Acceptance**: **65%** for coalescing, **50%** for adaptive FPS

**Concerns**:
- 16ms perceived lag
- Mode thrashing (100ms threshold too low)

**Recommendation**:
- ✅ **Accept coalescing** with configurable threshold
- ⚠️ **Revise adaptive FPS thresholds** (100ms → 2s, 2s → 10s)

---

### Phase 16-17: Wayland Best Practices

**Goal**: Implement proper Wayland synchronization

**Files Modified**:
- `window/src/os/wayland/triplebuffer.rs` (NEW, 408 lines)
- `window/src/os/wayland/gpufence.rs` (NEW, 267 lines)
- `window/src/os/wayland/presentation.rs` (NEW, 302 lines)
- `window/src/os/wayland/window.rs` (modified, integration)

**Key Changes**:
1. **Triple buffering** - 3 EGL surfaces, rotate on each frame
2. **GPU fences** - `eglCreateSyncKHR`, wait before submitting next frame
3. **wp_presentation_time** - vsync prediction and timing correction

**Effectiveness**: ⭐ **Uncertain** (GPU stalls persist in logs)

**Maintainer Acceptance**: **20-25%** (complexity without proven benefit)

**Concerns**:
- **Driver compatibility** (fences)
- **Compositor support** (presentation-time)
- **No evidence of improvement**
- **Fallback complexity**

**Recommendation**: ❌ **Reject or defer** until:
1. Profiling shows clear improvement
2. Driver compatibility matrix provided
3. Fallback paths thoroughly tested

---

### Phase 18-19: Remote Mux Optimization

**Goal**: Fix resize performance bottleneck in remote mux

**Files Modified**:
- `codec/src/lib.rs` (modified, TabResized struct)
- `mux/src/lib.rs` (modified, MuxNotification enum)
- `mux/src/tab.rs` (modified, resize notifications)
- `wezterm-client/src/client.rs` (modified, TabResized handling)
- `wezterm-client/src/pane/clientpane.rs` (modified, resize logic)
- `wezterm-client/src/pane/renderable.rs` (modified, fetch generation)
- `wezterm-mux-server-impl/src/dispatch.rs` (modified, notification dispatch)
- `wezterm-mux-server-impl/src/sessionhandler.rs` (modified, resize handling)

**Key Changes**:
1. **Enhanced TabResized PDU**:
   ```rust
   pub struct TabResized {
       pub tab_id: TabId,
       pub size: Option<TerminalSize>,      // NEW
       pub topology_changed: bool,          // NEW
   }
   ```

2. **Redundant resize detection** - early return if dimensions unchanged
3. **Selective invalidation** - `make_viewport_stale(100)` vs `make_all_stale()`
4. **"Debounced" resize** - spawn async task with 100ms delay (**BROKEN**)
5. **TabResized → resync optimization** - skip resync for size-only changes
6. **Fetch generation tracking** - discard stale fetches

**Effectiveness**:
- Redundant detection: ⭐⭐⭐⭐⭐ **Excellent** (99% of events blocked)
- Selective invalidation: ⭐⭐⭐⭐⭐ **Excellent** (150 vs 10,000 lines)
- Debounce: ❌ **Broken** (doesn't cancel previous tasks)
- resync optimization: ⭐⭐⭐⭐ **Very Good** (eliminates 100-200ms RPC)

**Maintainer Acceptance**:
- Redundant detection: **80%** (obvious win)
- Selective invalidation: **85%** (clear improvement)
- Enhanced protocol: **45%** (protocol change, needs versioning)
- Debounce: **30%** (broken implementation)
- resync optimization: **40%** (depends on enhanced protocol)

**Concerns**:
- **Wire protocol change** - backward compatibility unclear
- **Debounce implementation broken** - needs rewrite
- **Correctness critical** - topology_changed flag must be accurate

**Recommendation**:
- ✅ **Accept**: Redundant detection, selective invalidation
- ⚠️ **Accept with changes**: Enhanced protocol (add versioning)
- ❌ **Reject and rewrite**: Debounce implementation
- ⚠️ **Accept with testing**: resync optimization (after protocol fixed)

---

## Recommendations

### Tier 1: Accept Immediately ✅

These changes are self-contained, proven effective, and low risk:

1. **Lua Callback Caching** (Phases 0-5)
   - Action: Accept after cleanup (remove unused code warnings)
   - Testing: GUI rendering tests

2. **Redundant Resize Detection** (Phase 19.2)
   - Action: Accept after changing log level (ERROR → DEBUG)
   - Testing: Remote mux resize tests

3. **Selective Viewport Invalidation** (Phase 19)
   - Action: Accept after adding tests
   - Testing: Scrolling, large scrollback, split views

4. **Frame Budgeting** (Phase 15)
   - Action: Accept (monitoring only)
   - Testing: None needed (observability)

### Tier 2: Accept After Fixes ⚠️

These changes need rework before accepting:

5. **Event Coalescing** (Phase 15)
   - Issue: 16ms perceived lag
   - Fix: Make threshold configurable
   - Action: Accept after adding config option

6. **Adaptive Frame Rate** (Phase 15)
   - Issue: Threshold too aggressive (100ms)
   - Fix: Increase thresholds (100ms → 2s, 2s → 10s)
   - Action: Accept after threshold adjustment

7. **Enhanced TabResized Protocol** (Phase 19.2)
   - Issue: No versioning, backward compatibility unclear
   - Fix: Add protocol version field
   - Action: Accept after adding versioning:
     ```rust
     pub struct TabResized {
         pub tab_id: TabId,
         pub size: Option<TerminalSize>,
         pub topology_changed: bool,
         #[serde(default)]
         pub protocol_version: u32,
     }
     ```

8. **TabResized → resync() Optimization** (Phase 19.2)
   - Issue: Depends on Enhanced TabResized protocol
   - Fix: Ensure topology_changed flag is always correct
   - Action: Accept after protocol versioning added
   - Testing: Extensive remote mux testing (splits, zooms, size changes)

### Tier 3: Reject and Rewrite ❌

These changes are broken and need complete rewrite:

9. **"Debounced" Resize RPC** (Phase 19.3)
   - Issue: **Implementation is broken** - spawns tasks without cancellation
   - Fix: Rewrite with proper cancellation:
     ```rust
     // Need shared state
     pub struct ClientPane {
         pending_resize: Arc<Mutex<Option<PendingResize>>>,
     }

     struct PendingResize {
         size: TerminalSize,
         generation: usize,
     }

     impl ClientPane {
         fn resize(&self, size: TerminalSize) -> Result<()> {
             // Increment generation to invalidate pending tasks
             let generation = {
                 let mut pending = self.pending_resize.lock().unwrap();
                 let gen = pending.as_ref().map_or(0, |p| p.generation) + 1;
                 *pending = Some(PendingResize { size, generation: gen });
                 gen
             };

             // Spawn task with generation check
             let pending = Arc::clone(&self.pending_resize);
             promise::spawn::spawn(async move {
                 Timer::after(Duration::from_millis(100)).await;

                 // Check if this task is still current
                 let current_gen = pending.lock().unwrap()
                     .as_ref().map(|p| p.generation).unwrap_or(0);

                 if generation == current_gen {
                     // Send resize RPC
                     client.client.resize(Resize { ... }).await
                 } else {
                     // Task was superseded, skip
                     log::debug!("Debounced resize cancelled by newer generation");
                 }
             }).detach();

             Ok(())
         }
     }
     ```
   - Action: Rewrite, then submit as separate PR
   - Testing: Rapid resize test, verify only 1 RPC sent

### Tier 4: Defer Until Proven ⏸️

These changes lack evidence of effectiveness:

10. **Vertex Buffer Pooling** (Phase 12)
    - Issue: Unclear if it helps GPU stalls
    - Action: Defer or accept as "nice-to-have" optimization
    - Justification: Low risk, self-contained, might help

11. **Triple Buffering** (Phase 16-17)
    - Issue: GPU stalls persist despite implementation
    - Action: Defer until proven effective
    - Required: Before/after profiling showing improvement

12. **GPU Fence Synchronization** (Phase 17)
    - Issue: No evidence of improvement, driver compatibility unknown
    - Action: Defer until:
      - Driver compatibility matrix provided
      - Runtime detection implemented
      - Profiling shows improvement

13. **wp_presentation_time** (Phase 17)
    - Issue: Compositor support limited, effectiveness not demonstrated
    - Action: Defer until:
      - Proven to work on major compositors
      - Fallback implemented and tested
      - Clear performance benefit shown

14. **Damage Tracking** (Phase 8)
    - Issue: Correctness uncertain
    - Action: Accept after validation on multiple compositors
    - Testing: KWin, GNOME, Sway, Hyprland

---

## Summary Statistics

### Changes by Acceptance Tier

| Tier | Count | Lines Changed | Acceptance Rate |
|------|-------|---------------|-----------------|
| Tier 1: Accept Immediately | 4 | ~1,200 | 85-95% |
| Tier 2: Accept After Fixes | 5 | ~800 | 45-80% |
| Tier 3: Reject and Rewrite | 1 | ~100 | 30% |
| Tier 4: Defer Until Proven | 5 | ~2,100 | 15-75% |

### Changes by Category

| Category | Files | Lines | Status |
|----------|-------|-------|--------|
| Lua Optimization | 5 new, 3 modified | ~1,200 | ✅ Complete & Working |
| Wayland Infrastructure | 3 new, 1 modified | ~1,000 | ⚠️ Complete but Unproven |
| Remote Mux | 8 modified | ~500 | ⚠️ Partially Working |
| GPU Management | 1 new, 3 modified | ~500 | ⚠️ Limited Effectiveness |

### Overall Assessment

| Metric | Value |
|--------|-------|
| Total Commits | 41 |
| Total Lines Changed (Rust) | 4,218 |
| Phases Completed | 19 |
| Time Invested | ~3-4 weeks |
| **Success Rate (by line count)** | **~60%** (working changes) |
| **Success Rate (by impact)** | **~40%** (significant improvements) |

### Key Wins ✅

1. **Lua overhead reduced 67-80%** (16% → 3-5% CPU)
2. **Tab title rendering 100x faster** (<1ms vs 10-20ms)
3. **Remote mux resize 60-100x reduction** in fetched lines (150 vs 10,000)
4. **Resize feedback loop broken** (redundant detection blocks 99% of events)

### Key Concerns ⚠️

1. **Wire protocol changed** without versioning (backward compatibility broken)
2. **Debounce implementation broken** (doesn't cancel previous tasks)
3. **GPU stalls persist** (100-700ms) despite triple buffering & fences
4. **Wayland-specific changes untested** on multiple compositors
5. **No evidence** wp_presentation_time or GPU fences help

### Recommendations for Maintainers

#### Short Term (Accept Now)
- Lua caching infrastructure (proven win)
- Redundant resize detection (obvious optimization)
- Selective viewport invalidation (massive improvement)
- Frame budgeting (observability)

#### Medium Term (Fix Then Accept)
- Event coalescing (add config)
- Adaptive FPS (adjust thresholds)
- Enhanced TabResized protocol (add versioning)
- Damage tracking (validate on compositors)

#### Long Term (Needs Work)
- Debounce (rewrite completely)
- Triple buffering (prove effectiveness)
- GPU fences (test drivers, add fallback)
- wp_presentation_time (prove benefit, test compositors)

---

## Conclusion

This code review reveals a **mixed outcome**: significant progress in Lua optimization and remote mux efficiency, but **unproven or broken implementations** in GPU synchronization and async debounce.

**Recommended Actions**:
1. **Accept immediately**: Lua caching, redundant detection, selective invalidation (~60% of changes)
2. **Fix then accept**: Protocol versioning, adaptive FPS thresholds, event coalescing config (~25% of changes)
3. **Rewrite**: Debounce implementation (~5% of changes)
4. **Defer**: Triple buffering, GPU fences, wp_presentation_time until proven (~10% of changes)

**Overall Grade**: **B-** (70/100)
- Strong improvements in targeted areas
- Significant bugs in critical paths
- Excellent documentation and analysis
- Needs rework before mainline merge

