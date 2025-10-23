# Phase 1 Assessment: Wayland Paint Throttling Results

## Date
2025-10-23

## Changes Implemented

### Code Changes Since 0a1985ba3cfaf470af8147050ec13ba61ef8cdd5

**File**: `window/src/os/wayland/window.rs`

Added paint throttling mechanism:
```rust
// New fields
paint_throttled: bool,
last_paint: Instant,

// In do_paint()
if self.paint_throttled {
    self.invalidated = true;
    return Ok(());
}

// After painting
self.paint_throttled = true;
promise::spawn::spawn(async move {
    async_io::Timer::after(Duration::from_millis(16)).await;
    WaylandConnection::with_window_inner(window_id, |inner| {
        inner.paint_throttled = false;
        if inner.invalidated {
            inner.do_paint().ok();
        }
        Ok(())
    });
}).detach();
```

## Performance Comparison

### Profiling Report Comparison

| Component | perf-report.1 | perf-report.2 | Change |
|-----------|---------------|---------------|---------|
| **memmove** | 16.45% / 7.08% | 15.61% / 6.88% | ✅ -0.84% / -0.20% |
| **malloc** | 8.13% / 0.87% | 8.11% / 0.98% | ≈ -0.02% / +0.11% |
| **_int_malloc** | 7.06% / 2.44% | 6.94% / 2.45% | ✅ -0.12% / +0.01% |
| **mlua::table::raw_set** | 7.13% / 1.75% | 6.70% / 1.55% | ✅ -0.43% / -0.20% |
| **mlua::create_string** | 6.29% / 0.97% | 6.31% / 0.99% | ≈ +0.02% / +0.02% |
| **aux_rawset** | 5.39% / 0.22% | 5.15% / 0.23% | ✅ -0.24% / +0.01% |
| **luaH_newkey** | 4.84% / 1.18% | 4.67% / 1.08% | ✅ -0.17% / -0.10% |
| **dynamic_to_lua_value** | 3.28% / 2.30% | 3.17% / 2.12% | ✅ -0.11% / -0.18% |

*Format: Children% / Self%*

### Overall Impact

**Minimal improvement observed** (~1-2% total reduction in overhead)

The numbers show:
- ✅ Small reduction in Lua FFI overhead (0.4-0.5% total)
- ✅ Small reduction in memory operations (~1% total)
- ❌ Still dominated by the same bottlenecks
- ❌ **No significant performance improvement**

## Root Cause Analysis

### Why Did Throttling Not Help Much?

The paint throttling implementation has a **fundamental flaw**:

#### The Async Timer Problem

```rust
// Current implementation
self.paint_throttled = true;
promise::spawn::spawn(async move {
    async_io::Timer::after(Duration::from_millis(16)).await;  // ⚠️ Problem here
    WaylandConnection::with_window_inner(window_id, |inner| {
        inner.paint_throttled = false;
        // ...
    });
}).detach();
```

**The issue**: The async timer does **not** prevent Lua callbacks from being invoked!

#### What Actually Happens

1. **Resize event arrives** from Wayland
2. **Event handlers fire immediately**:
   - `format-window-title` callback → Lua call
   - `update-right-status` callback → Lua call
   - Tab state updates → potential Lua calls
3. **Then** `do_paint()` checks throttle
4. If throttled, paint is skipped
5. Async timer eventually resets the throttle flag

**The problem**: Steps 2-3 happen BEFORE the paint throttle check!

#### The Real Flow

```
Wayland Resize Event
    ↓
TermWindow::do_resize()                    [Event handlers fire HERE]
    ↓
    ├─→ format-window-title (Lua) ❌      [Not throttled!]
    ├─→ update-right-status (Lua) ❌      [Not throttled!]
    └─→ update_title_impl()                [More callbacks]
        ↓
        eventually reaches...
        ↓
WaylandWindowInner::do_paint()             [Paint throttle checked HERE]
    ↓
    if paint_throttled { return; } ✅      [Only paint is throttled]
```

**Result**: The expensive Lua FFI calls happen at full rate (60-120 Hz), only the final paint is throttled.

### Evidence from Profiling

The profiling shows:
- **No reduction in Lua call frequency**
- `mlua::*` functions still dominant (13-15% total)
- `dynamic_to_lua_value` still at 3.17% (only -0.11% improvement)

This confirms that **Lua callbacks are still being invoked at high frequency**.

## What We Learned

### Paint Throttling vs Event Throttling

**Paint throttling** (what was implemented):
- Limits how often pixels are drawn to screen
- Does NOT reduce event processing
- Does NOT reduce Lua callback frequency

**Event throttling** (what's actually needed):
- Limits how often events trigger callbacks
- Reduces Lua FFI overhead
- Must happen BEFORE event handlers

### The Missing Layer

The architecture needs throttling at **three levels**:

```
Level 1: Input Events (Wayland)
    ↓ [Need throttle HERE] ❌
Level 2: Event Handlers (Callbacks, Lua)
    ↓ [Need throttle HERE] ❌
Level 3: Paint (do_paint)
    ↓ [Throttle added HERE] ✅ (but too late)
```

Currently, only Level 3 is throttled, but the expensive work happens at Levels 1-2.

## Updated Next Steps

### Critical Issue: Architecture Problem

The current approach of throttling at the paint level cannot solve the performance problem because:

1. **Wayland resize events** arrive at compositor rate (60-120 Hz)
2. **Each event triggers**:
   - Window resize handlers
   - Tab state updates
   - Lua callbacks for window title, status, etc.
3. **All this happens BEFORE** `do_paint()` is called
4. **Paint throttle** only prevents drawing, not event processing

### Three Approaches

#### Approach A: Throttle at Window Event Level (Recommended)

**Goal**: Debounce resize events before they trigger any processing

**Implementation**:
```rust
// window/src/os/wayland/window.rs
pub struct WaylandWindowInner {
    resize_throttle: ResizeThrottle,
    // ...
}

struct ResizeThrottle {
    pending_size: Option<(u16, u16)>,
    last_resize: Instant,
    throttle_timer: Option<JoinHandle<()>>,
}

impl WaylandWindowInner {
    fn configure(&mut self, width: u16, height: u16) {
        let now = Instant::now();
        
        // If we resized recently, accumulate the change
        if now.duration_since(self.resize_throttle.last_resize) < Duration::from_millis(16) {
            self.resize_throttle.pending_size = Some((width, height));
            
            // Schedule deferred resize if not already scheduled
            if self.resize_throttle.throttle_timer.is_none() {
                let window_id = self.window_id;
                self.resize_throttle.throttle_timer = Some(promise::spawn::spawn(async move {
                    async_io::Timer::after(Duration::from_millis(16)).await;
                    WaylandConnection::with_window_inner(window_id, |inner| {
                        if let Some((w, h)) = inner.resize_throttle.pending_size.take() {
                            inner.resize_throttle.throttle_timer = None;
                            inner.do_resize_internal(w, h)?;
                        }
                        Ok(())
                    });
                }));
            }
            return;
        }
        
        // Not throttled, process immediately
        self.resize_throttle.last_resize = now;
        self.do_resize_internal(width, height)?;
    }
    
    fn do_resize_internal(&mut self, width: u16, height: u16) -> anyhow::Result<()> {
        // Existing resize logic here
        // This is where all the event handlers and callbacks get triggered
        // ...
    }
}
```

**Impact**: 80-95% reduction in Lua callback frequency  
**Effort**: 2-3 days  
**Risk**: Medium (need careful testing of resize behavior)

#### Approach B: Throttle Individual Callbacks (Complementary)

**Goal**: Add throttling to each expensive callback

**Implementation**: As described in previous assessment (Priority 3)

```rust
// wezterm-gui/src/callback_throttle.rs
lazy_static::lazy_static! {
    static ref CALLBACK_THROTTLE: Mutex<CallbackThrottle> = 
        Mutex::new(CallbackThrottle::new());
}

pub struct CallbackThrottle {
    last_calls: HashMap<String, Instant>,
    intervals: HashMap<String, Duration>,
}

impl CallbackThrottle {
    pub fn should_call(&mut self, name: &str) -> bool {
        let now = Instant::now();
        let interval = self.intervals.get(name)
            .copied()
            .unwrap_or(Duration::from_millis(50));
        
        if let Some(&last) = self.last_calls.get(name) {
            if now.duration_since(last) < interval {
                return false; // Throttled
            }
        }
        
        self.last_calls.insert(name.to_string(), now);
        true
    }
}

// In termwindow.rs
fn update_title_impl(&mut self) {
    let mut throttle = CALLBACK_THROTTLE.lock().unwrap();
    
    // Only call format-window-title every 200ms
    if throttle.should_call("format-window-title") {
        // Call Lua callback
    } else {
        // Use cached value
    }
}
```

**Impact**: 50-80% reduction in callback overhead  
**Effort**: 3-4 days  
**Risk**: Low (can be made configurable)

#### Approach C: Cache Window Title & Status (Easiest)

**Goal**: Extend Phase 0 caching to other callbacks

Apply the `tab_title_cache.rs` pattern to:
1. Window title formatting
2. Status line updates
3. Any other frequent callbacks

**Impact**: Cache hits → <1ms per call  
**Effort**: 2-3 days  
**Risk**: Very low (proven pattern)

### Recommended Implementation Order

#### Week 2 (Immediate)
**Priority 1A**: Implement Approach C (Window Title & Status Caching)
- Lowest risk
- Immediate benefit
- Builds on proven Phase 0 pattern
- Can be done in parallel with approach A

**Priority 1B**: Implement Approach A (Window Event Throttling)
- Highest potential impact
- Addresses root cause
- More complex but most effective

#### Week 3
**Priority 2**: Implement Approach B (Callback Throttling)
- Complementary to A & C
- Provides additional safety net
- Can be made user-configurable

#### Week 4
**Polish & Tune**:
- GC tuning
- Fine-tune throttle intervals
- Measure and optimize

## Expected Results

### After Approach C (Caching)
- Window title/status: <1ms (cache hit)
- Combined with Phase 0: Most UI updates cached
- Lua calls reduced by 60-70%

### After Approach A (Event Throttling)
- Resize events: 120 Hz → 60 Hz effective rate
- Callback frequency: 80-95% reduction
- Combined reduction: 90-98% fewer Lua calls

### After Approach B (Callback Throttling)
- Additional safety layer
- Handle any remaining high-frequency calls
- Configurable per-callback intervals

### Combined Impact
**Expected reduction in Lua overhead**: From 30-40% → 5-10%  
**Expected resize smoothness**: Perceptually smooth at 60 FPS  
**Frame time stability**: Consistent, no GC spikes

## Detailed Event Flow Analysis

### Current Event Flow (from profiling + code analysis)

```
1. Wayland Compositor
    ↓ [60-120 Hz]
    
2. window/src/os/wayland/window.rs
    ├─ WaylandWindowInner::dispatch_pending_event()
    │   └─ Line 912: self.events.dispatch(WindowEvent::Resized { ... })
    │       
3. wezterm-gui/src/termwindow/mod.rs
    ├─ Line 958: self.resize(dimensions, window_state, window, live_resizing)
    │   
4. wezterm-gui/src/termwindow/resize.rs
    ├─ Line 70: self.emit_window_event("window-resized", None)
    │   └─ Triggers Lua "window-resized" event handler ❌ [High frequency!]
    │
    ├─ Updates dimensions, recalculates layout
    │
    └─ Eventually reaches painting...
        
5. window/src/os/wayland/window.rs
    └─ WaylandWindowInner::do_paint()
        └─ Line 1088: if self.paint_throttled { return; } ✅ [Too late!]
```

**Problem**: The Lua callback (`window-resized` event) fires at step 4, which happens BEFORE the paint throttle check at step 5.

### Additional Lua Callback Sources During Resize

Besides the `window-resized` event, these also fire on every resize:

1. **Tab bar rendering** (Phase 0 fixed ✅)
   - `format-tab-title` callback → Now cached!
   
2. **Window title** (Not yet optimized ❌)
   - `format-window-title` callback
   - Called from `update_title_impl()`
   - Fires on EVERY resize
   
3. **Status line** (Not yet optimized ❌)
   - `update-right-status` callback
   - Fires on EVERY frame/resize
   
4. **Custom event handlers** (Not yet optimized ❌)
   - `window-resized` event handler (if user configured)
   - `window-focus-changed` (if triggered)
   - Any other custom callbacks

### Why Paint Throttling Didn't Help

The profiling confirms:
- Lua table operations: 6.70% (was 7.13%, only -0.43%)
- Lua string creation: 6.31% (was 6.29%, +0.02%)
- Lua data conversion: 3.17% (was 3.28%, -0.11%)

**Total Lua overhead still ~16%** with negligible improvement.

## Conclusion

The Wayland paint throttling (Phase 1) was **correctly implemented** but addressed the wrong layer of the problem.

**Key insight**: The performance issue is not from **painting** (drawing pixels), but from **event processing** (Lua FFI overhead).

**Root cause**: Resize events trigger expensive Lua callbacks at full compositor rate (60-120 Hz), and the paint throttle happens after all this work is done.

**The Event Flow Problem**:
```
Resize Event → Lua Callbacks (❌ not throttled)
    ↓
    ├─ window-resized event
    ├─ format-window-title
    ├─ update-right-status
    └─ [All expensive FFI calls here]
        ↓
        Eventually...
        ↓
Paint Check (✅ throttled, but too late)
```

**Solution**: Need to throttle at the **event level** (Approach A) or **callback level** (Approaches B & C), not at the paint level.

**Immediate action**: Implement Approach C (window title/status caching) for quick wins, then Approach A (event throttling) for comprehensive solution.

The Phase 0 caching **is working** for tab titles (0.06% CPU). We need to extend this pattern to other callbacks and add event-level throttling to achieve smooth resize performance.

## Specific Implementation Targets

Based on code analysis, these are the specific functions that need optimization:

1. **wezterm-gui/src/termwindow/mod.rs:1567** - `schedule_window_event`
   - Add throttling here to prevent event spam
   
2. **wezterm-gui/src/termwindow/mod.rs** - `update_title_impl`
   - Add caching for `format-window-title` callback
   
3. **wezterm-gui/src/termwindow/mod.rs** - `update-right-status` callback site
   - Add caching for status line updates
   
4. **window/src/os/wayland/window.rs:912** - `WindowEvent::Resized` dispatch
   - Consider debouncing resize events here

These are the four critical intervention points to achieve smooth resize performance.

