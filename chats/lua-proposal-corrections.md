# Corrections and Clarifications to Lua Change Proposal

**Date**: 2025-10-22
**Status**: Critical Review - Corrections Required
**Relates to**: `chats/lua-change-proposal-1.md`

## Executive Summary

After re-inspecting the codebase, several critical issues were found in the original proposal. While the overall direction is sound, **Phase 0 (Async Conversion) as proposed is NOT feasible** without major architectural changes. This document provides corrections and a revised implementation strategy.

---

## ✅ What Was CORRECT in Original Proposal

### 1. Bytecode Caching (Phase 1) - VERIFIED

**Claim**: mlua 0.9.9 supports `Function::dump()` for bytecode caching.

**Verification**: ✅ **CONFIRMED**
```rust
// Found in ~/.cargo/registry/src/.../mlua-0.9.9/src/function.rs
pub fn dump(&self, strip: bool) -> Vec<u8> { ... }
```

**Assessment**: This feature is fully supported and the proposal is accurate.

### 2. Async Infrastructure Exists - VERIFIED

**Claim**: mlua has `create_async_function` and `call_async`.

**Verification**: ✅ **CONFIRMED**
```rust
// config/src/lua.rs:360
wezterm_mod.set("emit", lua.create_async_function(emit_event)?)?;

// config/src/lua.rs:816-835
pub async fn emit_async_callback<'lua, A>(...) -> mlua::Result<mlua::Value<'lua>> {
    // Uses call_async
    func.call_async(args).await
}
```

**Assessment**: Async infrastructure exists and is already used in several places.

### 3. Event Coalescing Exists - VERIFIED

**Claim**: Output parsing has coalescing with `mux_output_parser_coalesce_delay_ms`.

**Verification**: ✅ **CONFIRMED**
```rust
// config/src/config.rs:405-406
pub mux_output_parser_coalesce_delay_ms: u64,  // default: 3ms

// mux/src/lib.rs:146
let mut delay = Duration::from_millis(configuration().mux_output_parser_coalesce_delay_ms);
```

**Assessment**: Coalescing exists but is limited to mux output, not UI callbacks.

### 4. Paint Throttling Exists - PARTIAL

**Claim**: Paint throttling should be implemented.

**Verification**: ⚠️ **PARTIALLY EXISTS**
- ✅ Windows: `paint_throttled` in `window/src/os/windows/window.rs:128`
- ✅ macOS: `paint_throttled` in `window/src/os/macos/window.rs:1541`
- ✅ X11: `paint_throttled` in `window/src/os/x11/window.rs:105`
- ❌ **Wayland: NO paint throttling found**

**Assessment**: Proposal to add throttling is valid, especially for Wayland.

---

## ❌ What Was INCORRECT in Original Proposal

### Critical Issue 1: Async Conversion Approach is Infeasible

**Original Claim** (Phase 0):
> "Convert `format-tab-title` to async by using `with_lua_config_on_main_thread` and `emit_async_callback`"

**Reality Check**:

**File**: `wezterm-gui/src/tabbar.rs:133-144`
```rust
fn compute_tab_title(...) -> TitleText {  // ❌ SYNCHRONOUS function
    let title = call_format_tab_title(...);  // ❌ SYNCHRONOUS call
    // ...
}
```

**Call Chain** (`wezterm-gui/src/tabbar.rs:380-396`):
```rust
let tab_titles: Vec<TitleText> = tab_info
    .iter()
    .map(|tab| {  // ❌ SYNCHRONOUS map closure
        compute_tab_title(tab, ...)  // ❌ Must return TitleText immediately
    })
    .collect();
```

**Usage** (`wezterm-gui/src/termwindow/mod.rs:1991-2006`):
```rust
fn update_title_impl(&mut self) {  // ❌ SYNCHRONOUS function
    let new_tab_bar = TabBarState::new(...);  // ❌ SYNCHRONOUS constructor
    // Calls compute_tab_title internally
}
```

**Rendering** (`wezterm-gui/src/termwindow/render/tab_bar.rs:10`):
```rust
pub fn paint_tab_bar(&mut self, layers: &mut TripleLayerQuadAllocator) -> anyhow::Result<()> {
    // ❌ SYNCHRONOUS paint function
    // Uses self.tab_bar which was built synchronously
}
```

**Why This is a Problem**:
1. **Rendering is synchronous by design** - GPU/OpenGL requires synchronous frame completion
2. **Making it async requires refactoring**:
   - `TabBarState::new()` → async
   - `compute_tab_title()` → async
   - `update_title_impl()` → async
   - `paint_tab_bar()` → async (breaks rendering contract)
3. **This is NOT a small change** - it affects the entire rendering pipeline

**Correct Approach**:
- ❌ DON'T make render functions async
- ✅ DO use caching with pre-computation
- ✅ DO provide default/fallback values during computation
- ✅ DO spawn background tasks to warm cache (optional)

### Critical Issue 2: Proposed Code Examples Won't Compile

**Original Proposal** (Phase 0 example):
```rust
async fn call_format_tab_title_async(...) -> Option<TitleText> {
    config::with_lua_config_on_main_thread(|lua| async move {
        // ...
    }).await.ok()
}
```

**Problem**: The caller at line 387 is:
```rust
.map(|tab| {
    compute_tab_title(...)  // Can't call async function in sync closure!
})
```

**Can't use `.await` in synchronous context.**

### Critical Issue 3: Misunderstanding of call_async

**Original Claim**:
> "The problem is that critical callbacks still use the sync version"

**Reality**:
- The issue isn't just calling `call_async` instead of `call()`
- The issue is that **the entire call stack is synchronous**
- `call_async` yields within an async context, but we're NOT in an async context

**Example**:
```rust
// This works:
async fn some_async_function() {
    func.call_async(args).await;  // ✅ We're in async context
}

// This doesn't work:
fn some_sync_function() {
    func.call_async(args).await;  // ❌ ERROR: await in non-async function
}
```

---

## ✅ CORRECT Approach: Cache-First with Optional Background Pre-warming

### Strategy

**Keep render path synchronous, use smart caching:**

```rust
// Synchronous render path - no changes
fn compute_tab_title(...) -> TitleText {
    // 1. Check cache
    if let Some(cached) = TAB_CACHE.get(&key) {
        return cached;  // ✅ Instant cache hit
    }

    // 2. Return default immediately
    let default = generate_default_title(tab);

    // 3. Spawn background task to compute actual value
    spawn_tab_title_computation(key, tab.clone(), ...);

    // 4. Return default now (non-blocking)
    default
}

// Background task (runs off render thread)
fn spawn_tab_title_computation(key: CacheKey, tab: TabInformation, ...) {
    promise::spawn::spawn(async move {
        // This CAN be async because it's not on render path
        let result = with_lua_config_on_main_thread(|lua| async move {
            if let Some(lua) = lua {
                emit_async_callback(lua, ("format-tab-title", ...)).await
            }
        }).await;

        // Store in cache
        TAB_CACHE.insert(key, result);

        // Invalidate window to trigger repaint with new value
        invalidate_window();
    }).detach();
}
```

**Benefits**:
- ✅ Render path stays synchronous
- ✅ No blocking on Lua
- ✅ Lua execution happens in background
- ✅ First render shows defaults (fast)
- ✅ Next render shows computed values (cached)
- ✅ Smooth progressive enhancement

---

## Revised Implementation Plan

### Phase 0: Sync Caching with Defaults (Week 1-2) **[REVISED]**

**Goal**: Add synchronous cache that returns defaults on miss.

**Implementation**:

```rust
// New file: wezterm-gui/src/tab_title_cache.rs

use std::collections::HashMap;
use std::sync::Mutex;

lazy_static! {
    static ref TAB_TITLE_CACHE: Mutex<HashMap<TabCacheKey, TitleText>> =
        Mutex::new(HashMap::new());
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct TabCacheKey {
    tab_id: TabId,
    title: String,
    is_active: bool,
    hover: bool,
}

pub fn get_tab_title_cached(
    tab: &TabInformation,
    tab_info: &[TabInformation],
    pane_info: &[PaneInformation],
    config: &ConfigHandle,
    hover: bool,
) -> TitleText {
    let key = TabCacheKey {
        tab_id: tab.tab_id,
        title: tab.tab_title.clone(),
        is_active: tab.is_active,
        hover,
    };

    // Check cache
    {
        let cache = TAB_TITLE_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(&key) {
            return cached.clone();
        }
    }

    // Cache miss - try Lua with timeout
    let result = try_format_tab_title_with_timeout(
        tab,
        tab_info,
        pane_info,
        config,
        hover,
        Duration::from_millis(50),  // 50ms max
    );

    match result {
        Some(title) => {
            // Cache it
            TAB_TITLE_CACHE.lock().unwrap().insert(key, title.clone());
            title
        }
        None => {
            // Timeout or error - use default
            generate_default_title(tab)
        }
    }
}

fn try_format_tab_title_with_timeout(
    // ... args ...
    timeout: Duration,
) -> Option<TitleText> {
    // Use a oneshot channel with timeout
    let (sender, receiver) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let result = call_format_tab_title(...);  // Existing sync function
        sender.send(result).ok();
    });

    // Wait with timeout
    receiver.recv_timeout(timeout).ok().flatten()
}
```

**Benefits**:
- ✅ Works with existing synchronous code
- ✅ No blocking beyond timeout
- ✅ Progressive: default → cached
- ✅ Backward compatible

**Effort**: 1-2 weeks

### Phase 0b: Background Pre-warming (Week 2-3) **[OPTIONAL]**

**Once caching works, optionally add background pre-computation:**

```rust
pub fn prewarm_tab_titles(
    tabs: Vec<TabInformation>,
    panes: Vec<PaneInformation>,
    config: ConfigHandle,
) {
    promise::spawn::spawn(async move {
        for tab in tabs {
            for hover in [false, true] {
                let key = TabCacheKey { /* ... */ };

                // Skip if already cached
                if TAB_TITLE_CACHE.lock().unwrap().contains_key(&key) {
                    continue;
                }

                // Compute in background using async
                if let Ok(title) = with_lua_config_on_main_thread(|lua| async move {
                    // ... async Lua call ...
                }).await {
                    TAB_TITLE_CACHE.lock().unwrap().insert(key, title);
                }

                // Small delay between computations
                Timer::after(Duration::from_millis(16)).await;
            }
        }
    }).detach();
}
```

**Call from**: `update_title_impl` after updating tab info.

**Benefits**:
- ✅ Cache is warm when user hovers
- ✅ Truly non-blocking
- ✅ Uses async infrastructure correctly

**Effort**: 3-5 days

### Phase 1: Bytecode Caching (Week 3-4) **[UNCHANGED]**

✅ Original proposal is correct - proceed as planned.

### Phase 2: Event Throttling (Week 4-5) **[ENHANCED]**

**Add throttling for these callbacks:**
- `format-tab-title`: NO throttling (needs instant hover feedback)
- `format-window-title`: 500ms throttle
- `update-right-status`: 200ms throttle
- `update-status`: 200ms throttle

**Also add throttling for tabbar rebuilding:**
```rust
impl TermWindow {
    fn should_rebuild_tab_bar(&mut self) -> bool {
        // Only rebuild if significant change
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_tab_bar_rebuild);

        // Always rebuild on these events:
        if self.tabs_changed || self.active_tab_changed {
            self.last_tab_bar_rebuild = now;
            return true;
        }

        // Throttle hover updates to 60 FPS (16ms)
        if elapsed < Duration::from_millis(16) {
            return false;
        }

        self.last_tab_bar_rebuild = now;
        true
    }
}
```

### Phase 3: GC Tuning (Week 5) **[UNCHANGED]**

✅ Original proposal is correct - proceed as planned.

### Phase 4: Enhanced Caching (Week 6-8) **[SIMPLIFIED]**

Instead of complex AsyncLuaCache, use the simpler approach from Phase 0.

**Just extend the caching to other callbacks:**
- Window title cache
- Status line cache
- Command palette cache

**Same pattern**: sync cache + timeout + background pre-warm.

---

## Updated Effort Estimates

| Phase | Task | Revised Estimate | Original Estimate | Change |
|-------|------|------------------|-------------------|--------|
| 0 | Sync caching + timeout | 1-2 weeks | 1-2 weeks (but different approach) | Approach corrected |
| 0b | Background pre-warming | 3-5 days | N/A | Optional |
| 1 | Bytecode caching | 3-4 days | 3-4 days | No change |
| 2 | Event throttling | 5-7 days | 3-5 days | Enhanced scope |
| 3 | GC tuning | 2-3 days | 2-3 days | No change |
| 4 | Extended caching | 1-2 weeks | 3-4 weeks | Simplified |
| **Total (Phase 0-4)** | | **5-7 weeks** | **7-9 weeks** | **Faster!** |

**Reason for improvement**: Simpler approach, no async refactoring needed.

---

## Expected Performance (Corrected)

### Phase 0 (Sync Cache + Timeout):
- **First hover**: 50ms max (timeout) → shows default
- **Second hover**: <1ms (cache hit) → shows computed
- **User experience**: Slight delay first time, then smooth

### Phase 0 + 0b (Background Pre-warm):
- **First hover**: <1ms (already cached from pre-warm)
- **User experience**: Always smooth

### Phase 0-4 (All optimizations):
- **Tabbar**: <1ms always (cached)
- **Startup**: 20-30% faster (bytecode)
- **Frame times**: Stable (throttling + GC)

---

## Key Corrections Summary

| Original Claim | Correction | Impact |
|----------------|------------|--------|
| "Convert callbacks to async" | ❌ Can't - render path is sync | **CRITICAL** |
| "Use emit_async_callback in render" | ❌ Breaks synchronous contract | **CRITICAL** |
| "Phase 0 is async conversion" | ✅ Phase 0 is smart caching | **MAJOR** |
| "11 weeks total effort" | ✅ 5-7 weeks with simpler approach | **POSITIVE** |
| "Bytecode caching feasible" | ✅ Confirmed correct | **VALIDATED** |
| "Async infrastructure exists" | ✅ Exists but can't use in render | **CLARIFIED** |

---

## Recommendations

### MUST DO (Critical Corrections):
1. ❌ **ABANDON** the async conversion approach in Phase 0
2. ✅ **ADOPT** the sync cache + timeout approach instead
3. ✅ **OPTIONALLY ADD** background pre-warming (Phase 0b)
4. ✅ **PROCEED** with Phases 1-3 as originally proposed
5. ✅ **SIMPLIFY** Phase 4 to use same caching pattern

### Testing Strategy:
1. **Phase 0**: Verify cache hit rates >80%, timeout protection works
2. **Phase 0b**: Verify background tasks don't impact foreground
3. **Phase 1**: Benchmark startup time improvement
4. **Phase 2**: Measure frame time stability
5. **Phase 3**: Profile GC pause times
6. **Phase 4**: Verify all callbacks are cached

### Risk Assessment (Updated):
- **Phase 0 (sync cache)**: LOW risk - simple, testable
- **Phase 0b (pre-warm)**: LOW risk - background only
- **Phase 1 (bytecode)**: LOW risk - validated approach
- **Phase 2 (throttle)**: LOW risk - configurable
- **Phase 3 (GC)**: LOW risk - tunable
- **Phase 4 (extended cache)**: LOW risk - proven pattern

**Overall**: Much lower risk than original async approach.

---

## Conclusion

The original proposal had the right goals but proposed an infeasible implementation for Phase 0. The corrected approach:

1. ✅ **Achieves the same goals** (non-blocking, fast)
2. ✅ **Uses simpler techniques** (caching + timeout vs async refactor)
3. ✅ **Lower risk** (no architectural changes)
4. ✅ **Faster implementation** (5-7 weeks vs 7-9 weeks)
5. ✅ **Backward compatible** (no API changes)

**Key Insight**: You don't need to make everything async. Smart caching with timeouts and background pre-warming achieves the same result without the complexity.

**Next Steps**:
1. Update `lua-change-proposal-1.md` with these corrections
2. Implement Phase 0 (sync cache + timeout)
3. Evaluate Phase 0b (pre-warming) based on Phase 0 results
4. Proceed with Phases 1-4 as clarified

---

**Document Version**: 1.0
**Type**: Critical Corrections
**Confidence**: HIGH (based on thorough code inspection)
**Action Required**: Update original proposal before implementation
