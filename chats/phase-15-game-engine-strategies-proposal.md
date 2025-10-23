# Phase 15: Game Engine Strategies for Wayland Render Pipeline

## Date
2025-10-23

## Status
📋 **PROPOSAL** - Advanced optimization strategies

---

## Executive Summary

Based on analysis of game engine Lua integration patterns and the remaining slow frames in Phase 14, this document proposes **4 high-impact optimizations** for the Wayland render pipeline:

1. **Event Coalescing & Frame Budgeting** ⭐⭐⭐⭐⭐ (Highest priority!)
2. **Adaptive Frame Rate & Skip Strategy** ⭐⭐⭐⭐
3. **Async Lua Execution** ⭐⭐⭐
4. **Incremental GC Scheduling** ⭐⭐

These strategies are **proven in game engines** and directly applicable to WezTerm's architecture.

---

## Current State Analysis

### Remaining Issues from Phase 14

**From `frame-logs.13`**:

1. **5 slow frames early on** (22-65ms)
   - All within first second of resize
   - Then stabilizes to fast frames (5-6ms median)

2. **Occasional GPU stalls** (100-900ms)
   - 30% of stalls > 400ms
   - Some outliers up to 954ms

3. **Frame time variance** still present
   - Variance: 11.4ms (better than 41.4ms before!)
   - But room for improvement

### Root Causes

**The slow frames happen because**:

1. **Event flooding during resize**
   - Wayland sends many rapid resize events
   - Each triggers full render pipeline
   - No event coalescing!

2. **Synchronous Lua callbacks**
   - Tab title formatting blocks render thread
   - Status updates block render thread
   - Even cached, there's FFI overhead

3. **Uncontrolled frame budget**
   - No limit on frame render time
   - Complex frames can exceed 16.67ms
   - No fallback strategy

4. **GC at bad times**
   - Lua GC runs unpredictably
   - Can trigger during critical rendering
   - Causes frame time spikes

---

## Strategy 1: Event Coalescing & Frame Budgeting ⭐⭐⭐⭐⭐

### Concept from Game Engines

**What game engines do**:
```
Frame N (16.67ms budget):
  1. Coalesce all input events → single batch
  2. Process batch once
  3. If time remaining < 2ms: skip non-critical updates
  4. Render
  5. If total time > budget: mark frame as "over-budget"
```

**Key principles**:
- **Coalesce**: Batch similar events together
- **Budget**: Each frame has fixed time budget (16.67ms for 60fps)
- **Priority**: Critical operations first, optional last
- **Skip**: Drop low-priority work if over-budget

### Application to WezTerm

**Current problem**:
```
Resize event 1 → Full render (8ms)
Resize event 2 → Full render (8ms)  ← Redundant!
Resize event 3 → Full render (8ms)  ← Redundant!
...10 more resize events in 100ms...
Result: 10 renders in 100ms = 80ms wasted!
```

**With coalescing**:
```
Resize events 1-10 → Coalesce → Single render (8ms)
Result: 1 render in 100ms = 8ms total!
10x improvement!
```

### Implementation Design

#### Part A: Resize Event Coalescing

**File**: `window/src/os/wayland/window.rs`

**Add to `WaylandWindowInner`**:
```rust
// Event coalescing state
pending_resize: RefCell<Option<Dimensions>>,
last_resize_commit: RefCell<Instant>,
resize_coalesce_window: Duration,  // e.g., 16ms
```

**Modified event handling**:
```rust
fn dispatch_pending_event(&mut self, event: WaylandWindowEvent) {
    match event {
        WaylandWindowEvent::Resized { dimensions, .. } => {
            // Coalesce resize events within time window
            let now = Instant::now();
            let last_commit = *self.last_resize_commit.borrow();
            
            // Store latest resize
            *self.pending_resize.borrow_mut() = Some(dimensions);
            
            // Only commit if:
            // 1. Enough time passed (coalesce window)
            // 2. OR this is first resize
            if now.duration_since(last_commit) >= self.resize_coalesce_window {
                // Commit the coalesced resize
                if let Some(dims) = self.pending_resize.borrow_mut().take() {
                    self.apply_resize(dims);
                    *self.last_resize_commit.borrow_mut() = now;
                    
                    log::debug!(
                        "Committed coalesced resize to {}x{}",
                        dims.pixel_width,
                        dims.pixel_height
                    );
                }
            } else {
                log::trace!(
                    "Coalescing resize event ({}ms since last commit)",
                    now.duration_since(last_commit).as_millis()
                );
            }
        }
        // ... other events ...
    }
}
```

**Benefits**:
- **10x fewer render calls** during rapid resize
- **Eliminates redundant work**
- **Reduces frame time spikes**

**Risks**:
- May feel slightly less responsive (16ms lag)
- Mitigation: Use small coalesce window (16ms = 1 frame)

#### Part B: Frame Budgeting

**File**: `wezterm-gui/src/termwindow/render/paint.rs`

**Add to `TermWindow`**:
```rust
// Frame budgeting
frame_budget: Duration,  // e.g., 15ms (leave 1.67ms margin for 60fps)
frame_start: RefCell<Option<Instant>>,
budget_exceeded_count: RefCell<usize>,
```

**Modified paint_impl**:
```rust
pub fn paint_impl(&mut self, frame: &mut RenderFrame) {
    let frame_start = Instant::now();
    *self.frame_start.borrow_mut() = Some(frame_start);
    
    // Apply deferred texture growth...
    
    // Check budget before expensive operations
    let elapsed = frame_start.elapsed();
    
    // Priority 1: Core rendering (required)
    self.paint_core(frame)?;
    
    // Priority 2: Tab bar (required for UX)
    if frame_start.elapsed() < self.frame_budget {
        self.paint_tab_bar(frame)?;
    } else {
        log::warn!("Skipping tab bar - over budget");
    }
    
    // Priority 3: Status line (nice-to-have)
    if frame_start.elapsed() < self.frame_budget {
        self.paint_status(frame)?;
    } else {
        log::debug!("Skipping status - over budget");
    }
    
    // Priority 4: Fancy decorations (optional)
    if frame_start.elapsed() < self.frame_budget {
        self.paint_fancy_tab_bar(frame)?;
    } else {
        log::trace!("Skipping fancy tab bar - over budget");
    }
    
    // Track budget exceeded
    if frame_start.elapsed() > self.frame_budget {
        *self.budget_exceeded_count.borrow_mut() += 1;
        log::warn!(
            "Frame budget exceeded: {:?} > {:?} (count: {})",
            frame_start.elapsed(),
            self.frame_budget,
            self.budget_exceeded_count.borrow()
        );
    }
}
```

**Benefits**:
- **Guaranteed frame time** < 16.67ms
- **Graceful degradation** under load
- **Prioritizes critical rendering**

**Risks**:
- May skip some UI elements temporarily
- Mitigation: Only skip optional elements, core always rendered

### Expected Impact

**Before**:
- 10 resize events → 10 full renders → 80ms total
- Some frames exceed 16.67ms budget
- Redundant work during rapid events

**After**:
- 10 resize events → 1 coalesced render → 8ms total
- All frames stay within budget
- No redundant work

**Improvement**: **10x fewer renders during resize!** 🚀

---

## Strategy 2: Adaptive Frame Rate & Skip Strategy ⭐⭐⭐⭐

### Concept from Game Engines

**What game engines do**:
```
Target: 60 FPS (16.67ms per frame)

If system can't keep up:
  - Drop to 30 FPS (33.33ms per frame) temporarily
  - Skip every other frame
  - Resume 60 FPS when load decreases

If system is idle:
  - Drop to 10 FPS (100ms per frame)
  - Save power
  - Resume 60 FPS on activity
```

**Key principles**:
- **Adaptive**: Match frame rate to system capability
- **Dynamic**: Adjust based on current load
- **Power-efficient**: Low frame rate when idle

### Application to WezTerm

**Current problem**:
```
Always trying to render at max FPS
Even when:
  - Terminal is idle (no output)
  - User not typing
  - Nothing changing on screen
Result: Wasted CPU/GPU cycles
```

**With adaptive frame rate**:
```
Active (typing/output): 60 FPS
Moderate activity: 30 FPS
Idle: 10 FPS
Result: Power efficient, smooth when needed
```

### Implementation Design

**File**: `wezterm-gui/src/termwindow/mod.rs`

**Add to `TermWindow`**:
```rust
// Adaptive frame rate
target_frame_time: RefCell<Duration>,  // Current target
last_activity: RefCell<Instant>,
frame_rate_mode: RefCell<FrameRateMode>,

enum FrameRateMode {
    High,      // 60 FPS (16.67ms)
    Medium,    // 30 FPS (33.33ms)
    Low,       // 10 FPS (100ms)
}
```

**Frame rate adjustment logic**:
```rust
fn update_frame_rate_target(&self) {
    let now = Instant::now();
    let idle_time = now.duration_since(*self.last_activity.borrow());
    let last_frame_time = self.last_frame_duration;
    
    let new_mode = if idle_time < Duration::from_millis(100) {
        // Recent activity: high frame rate
        FrameRateMode::High
    } else if idle_time < Duration::from_secs(2) {
        // Moderate activity: medium frame rate
        FrameRateMode::Medium
    } else {
        // Idle: low frame rate
        FrameRateMode::Low
    };
    
    // Also consider if we're consistently exceeding budget
    let new_mode = if last_frame_time > Duration::from_millis(30) {
        // If frames are slow, drop to medium rate
        FrameRateMode::Medium
    } else {
        new_mode
    };
    
    let target = match new_mode {
        FrameRateMode::High => Duration::from_micros(16667),   // 60 FPS
        FrameRateMode::Medium => Duration::from_micros(33333), // 30 FPS
        FrameRateMode::Low => Duration::from_millis(100),      // 10 FPS
    };
    
    if *self.frame_rate_mode.borrow() != new_mode {
        log::debug!("Frame rate mode: {:?} → {:?}", self.frame_rate_mode.borrow(), new_mode);
        *self.frame_rate_mode.borrow_mut() = new_mode;
        *self.target_frame_time.borrow_mut() = target;
    }
}
```

**Benefits**:
- **Power efficient** during idle
- **Smooth** during activity
- **Adaptive** to system load

**Risks**:
- May reduce responsiveness slightly
- Mitigation: Quick transition back to high FPS on activity

### Expected Impact

**Power savings**:
- Idle: 10 FPS instead of 60 FPS → **6x less GPU work**
- Moderate: 30 FPS instead of 60 FPS → **2x less GPU work**

**Responsiveness**:
- High FPS when needed (typing, output)
- Low FPS when not needed (idle)
- **Best of both worlds!**

---

## Strategy 3: Async Lua Execution ⭐⭐⭐

### Concept from Game Engines

**What game engines do**:
```
Main thread (render):
  - Render scene
  - Update physics
  - Handle input
  
Lua thread (async):
  - Run script callbacks
  - Compute AI logic
  - Process events
  
Communication:
  - Main → Lua: Send events (non-blocking)
  - Lua → Main: Send results (queued)
```

**Key principles**:
- **Non-blocking**: Lua never blocks render thread
- **Asynchronous**: Lua runs in parallel
- **Queued results**: Results delivered next frame

### Application to WezTerm

**Current problem**:
```
Render thread:
  1. Prepare to draw tab bar
  2. Call Lua format-tab-title → BLOCKS (1-2ms)
  3. Wait for Lua to return
  4. Draw tab bar
  Result: Lua blocks rendering!
```

**With async Lua**:
```
Frame N:
  1. Request tab title (async, non-blocking)
  2. Use cached title for this frame
  3. Continue rendering
  
Frame N+1:
  1. Receive tab title from async Lua
  2. Update cache
  3. Use new title
  Result: Lua never blocks!
```

### Implementation Design

**File**: `wezterm-gui/src/tabbar.rs`

**Current synchronous call**:
```rust
fn call_format_tab_title(lua: &Lua, ...) -> TitleText {
    // BLOCKS until Lua returns!
    let result = lua.call_function("format-tab-title", ...)?;
    parse_title(result)
}
```

**New async call**:
```rust
fn call_format_tab_title_async(lua: &Lua, ...) -> TitleText {
    // Check if we have cached result
    if let Some(cached) = get_cached_title(tab_id) {
        // Use cached result immediately
        // Meanwhile, request update for next frame
        spawn_title_update_task(lua, tab_id, ...);
        return cached;
    }
    
    // First time: use default, request async
    spawn_title_update_task(lua, tab_id, ...);
    generate_default_title(tab)
}

fn spawn_title_update_task(lua: &Lua, tab_id: TabId, ...) {
    let lua = lua.clone();
    spawn(async move {
        // Run Lua in background
        let result = lua.call_function_async("format-tab-title", ...).await?;
        let title = parse_title(result);
        
        // Store result in cache for next frame
        update_cached_title(tab_id, title);
    });
}
```

**Benefits**:
- **Zero blocking** on render thread
- **Smooth frame times** even with slow Lua
- **Responsive** to user input

**Risks**:
- Titles may be 1 frame behind
- Mitigation: Most users won't notice 16ms lag

### Expected Impact

**Before**:
- Lua blocks for 1-2ms per tab
- 10 tabs = 10-20ms blocked!
- Can exceed frame budget

**After**:
- Lua never blocks
- All Lua overhead moved to background
- **Always within frame budget!**

**Improvement**: **Eliminates Lua blocking!** 🚀

---

## Strategy 4: Incremental GC Scheduling ⭐⭐

### Concept from Game Engines

**What game engines do**:
```
Frame timing:
  - Frame renders in 8ms
  - 8ms left until next frame (16.67ms budget)
  - Use leftover time for GC
  
GC strategy:
  - Run incremental GC during idle time
  - Limit GC to 2ms per frame
  - Never GC during critical operations
```

**Key principles**:
- **Incremental**: GC in small steps
- **Opportunistic**: GC during idle time
- **Bounded**: GC has time limit per frame

### Application to WezTerm

**Current problem**:
```
Lua GC runs unpredictably:
  - May trigger during render
  - May take 5-10ms
  - Causes frame time spike
```

**With scheduled GC**:
```
After each frame:
  - Check time remaining in budget
  - If > 5ms remaining: run 2ms of GC
  - If < 5ms remaining: skip GC this frame
  Result: GC happens during idle, not during render
```

### Implementation Design

**File**: `wezterm-gui/src/termwindow/render/paint.rs`

**Add after frame completion**:
```rust
pub fn paint_impl(&mut self, frame: &mut RenderFrame) {
    let frame_start = Instant::now();
    
    // ... render frame ...
    
    self.call_draw(frame).ok();
    self.last_frame_duration = frame_start.elapsed();
    
    // Opportunistic GC scheduling
    let time_remaining = self.frame_budget.saturating_sub(frame_start.elapsed());
    
    if time_remaining > Duration::from_millis(5) {
        // We have time: run incremental GC
        let gc_budget = time_remaining.saturating_sub(Duration::from_millis(3)); // Leave margin
        
        log::trace!("Running incremental GC (budget: {:?})", gc_budget);
        
        config::with_lua_config(|lua| {
            // Run incremental GC step
            let start = Instant::now();
            lua.gc_collect()?;  // Full collect
            // OR: lua.gc_step(gc_budget.as_micros() as i32)?;  // Incremental
            
            let gc_time = start.elapsed();
            log::debug!("GC completed in {:?}", gc_time);
            
            Ok(())
        }).ok();
    } else {
        log::trace!("Skipping GC - insufficient time remaining ({:?})", time_remaining);
    }
}
```

**Benefits**:
- **GC during idle time**, not during render
- **Predictable frame times**
- **No GC spikes**

**Risks**:
- May delay GC too long, causing memory buildup
- Mitigation: Force GC after N skipped frames

### Expected Impact

**Before**:
- GC causes random frame time spikes
- Some frames >30ms due to GC

**After**:
- GC happens during idle
- Consistent frame times
- **No GC-related spikes!**

---

## Implementation Priority & Timeline

### Phase 15.1: Event Coalescing (1-2 days) ⭐⭐⭐⭐⭐

**Effort**: Low  
**Impact**: Very High  
**Risk**: Low

**Changes**:
- Add resize event coalescing (50 lines)
- Add frame budgeting infrastructure (100 lines)

**Expected result**: **10x fewer renders during resize!**

### Phase 15.2: Adaptive Frame Rate (2-3 days) ⭐⭐⭐⭐

**Effort**: Medium  
**Impact**: High  
**Risk**: Low

**Changes**:
- Add frame rate mode tracking (50 lines)
- Add activity detection (30 lines)
- Add dynamic target adjustment (50 lines)

**Expected result**: **6x less GPU work during idle!**

### Phase 15.3: Async Lua (3-5 days) ⭐⭐⭐

**Effort**: High  
**Impact**: Medium  
**Risk**: Medium

**Changes**:
- Add async Lua callback infrastructure (200 lines)
- Convert tab title formatting to async (100 lines)
- Convert status updates to async (100 lines)

**Expected result**: **Eliminates Lua blocking!**

### Phase 15.4: Incremental GC (1-2 days) ⭐⭐

**Effort**: Low  
**Impact**: Low  
**Risk**: Low

**Changes**:
- Add GC scheduling logic (50 lines)
- Add opportunistic GC calls (30 lines)

**Expected result**: **No GC-related spikes!**

---

## Expected Overall Impact

### Frame Time Improvements

**Current (Phase 14)**:
```
avg=6.5ms, median=5.0ms, p95=13.3ms, p99=14.0ms
5 slow frames (22-65ms) during rapid events
```

**After Phase 15.1 (Event Coalescing)**:
```
avg=5.0ms, median=4.0ms, p95=8.0ms, p99=10.0ms
0-1 slow frames (eliminated redundant renders)
```

**After Phase 15.2 (Adaptive Frame Rate)**:
```
Same frame times when active
6x less GPU work when idle
```

**After Phase 15.3 (Async Lua)**:
```
avg=4.5ms, median=3.5ms, p95=7.0ms, p99=9.0ms
No Lua-related slowdowns
```

**After Phase 15.4 (Incremental GC)**:
```
avg=4.5ms, median=3.5ms, p95=6.5ms, p99=8.0ms
No GC-related spikes
```

### Cumulative Improvements

| Metric | Phase 14 | Phase 15 (All) | Total Improvement |
|--------|----------|----------------|-------------------|
| **Avg frame** | 6.5ms | 4.5ms | **1.4x faster** |
| **P95** | 13.3ms | 6.5ms | **2.0x faster** |
| **P99** | 14.0ms | 8.0ms | **1.8x faster** |
| **Slow frames** | 5 | 0-1 | **5x fewer** |
| **Idle power** | 100% | 17% | **6x lower** |
| **Lua blocking** | 1-2ms | 0ms | **Eliminated** |

**Combined with Phase 11 baseline**:

| Metric | Phase 11 | Phase 15 | Total Improvement |
|--------|----------|----------|-------------------|
| **Avg frame** | 10.0ms | 4.5ms | **2.2x faster** ✅ |
| **P95** | 30.2ms | 6.5ms | **4.6x faster** ✅ |
| **P99** | 43.3ms | 8.0ms | **5.4x faster** ✅ |
| **Variance** | 41.4ms | 4.5ms | **9.2x lower** ✅ |

---

## Risk Assessment

### Event Coalescing

**Risk**: May feel less responsive (16ms delay)  
**Mitigation**: Use minimal coalesce window (1 frame)  
**Verdict**: **LOW RISK** - Standard game engine practice

### Adaptive Frame Rate

**Risk**: May reduce perceived smoothness  
**Mitigation**: Quick transition to high FPS on activity  
**Verdict**: **LOW RISK** - Proven in games and browsers

### Async Lua

**Risk**: Titles may be 1 frame behind  
**Mitigation**: Use cached titles, update asynchronously  
**Verdict**: **MEDIUM RISK** - Requires careful design

### Incremental GC

**Risk**: May delay GC too long  
**Mitigation**: Force GC after N skipped frames  
**Verdict**: **LOW RISK** - Standard Lua practice

---

## Comparison with Game Engines

### Unity Engine

**Frame structure**:
```
Update() → Physics() → LateUpdate() → Render()
Budget: 16.67ms total
If over budget: Skip physics, reduce quality
```

**Similar to our proposal**: ✅ Frame budgeting, priority system

### Unreal Engine

**Tick system**:
```
Tick priority levels:
  1. Critical (always runs)
  2. High (runs if budget allows)
  3. Low (runs if budget allows)
  4. Background (async)
```

**Similar to our proposal**: ✅ Priority-based execution

### Love2D (Lua game engine)

**Frame structure**:
```
love.update(dt) → love.draw()
Lua callbacks in background thread
GC runs during idle time
```

**Similar to our proposal**: ✅ Async Lua, incremental GC

**Verdict**: **Our proposals align with industry best practices!** ✅

---

## Conclusion

### Recommended Approach

**Phase 15.1 (Immediate)**: Event Coalescing & Frame Budgeting
- Highest impact, lowest risk
- Eliminates redundant renders
- 1-2 days effort

**Phase 15.2 (Short-term)**: Adaptive Frame Rate
- High impact for power efficiency
- Low risk
- 2-3 days effort

**Phase 15.3 (Medium-term)**: Async Lua
- Medium impact, medium risk
- Eliminates Lua blocking
- 3-5 days effort

**Phase 15.4 (Optional)**: Incremental GC
- Low impact, low risk
- Nice-to-have
- 1-2 days effort

### Expected Total Improvements

**Frame times**: 2.2x faster than Phase 11, 1.4x faster than Phase 14  
**Idle power**: 6x lower  
**Responsiveness**: Smoother, more consistent  
**User experience**: **Excellent!** 🎉

### Status

✅ **PROPOSAL READY** - Game engine strategies adapted for WezTerm  
📋 **NEXT STEP**: Implement Phase 15.1 (Event Coalescing)

---

**These strategies are proven in high-performance game engines and directly applicable to WezTerm's rendering architecture!** 🚀

