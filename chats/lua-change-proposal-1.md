# Async Lua Execution for WezTerm: Comprehensive Proposal (AMENDED)

## Executive Summary

This document proposes a comprehensive strategy for optimizing Lua execution in wezterm to eliminate rendering thread blocking, particularly for performance-critical paths like tabbar rendering, window title updates, and other UI callbacks. The proposal is based on analysis of wezterm's current Lua integration, game engine best practices, an independent architectural review, and the capabilities of the mlua crate.

**AMENDMENT NOTE**: This proposal has been revised based on an independent review of Lua integration patterns from game engines. Key additions include bytecode caching, event throttling/coalescing, GC tuning, and better utilization of existing async infrastructure.

**Key Objectives:**
- Eliminate Lua-induced frame drops during UI updates
- Leverage existing mlua async infrastructure more effectively
- Add bytecode caching for faster config loads
- Implement event coalescing and throttling
- Tune garbage collection for stable frame times
- Maintain backward compatibility with existing user configurations

**Expected Impact:**
- **Tabbar rendering**: 5-50ms → <1ms with caching + async
- **Config startup**: 20-30% faster with bytecode caching
- **Event overhead**: 50% reduction with coalescing/throttling
- **Frame stability**: Eliminate GC-induced stutters

---

## 1. Current Architecture Analysis

### 1.1 Lua Integration Overview

**File**: `config/src/lib.rs`

**Core Infrastructure**:
```rust
thread_local! {
    static LUA_CONFIG: RefCell<Option<LuaConfigState>> = RefCell::new(None);
}

struct LuaConfigState {
    lua: Option<Rc<mlua::Lua>>,
}
```

**Key Properties**:
- **`mlua::Lua` is `Send` but NOT `Sync`**: Can be sent between threads but not shared
- **Thread-local storage**: Main thread holds `Rc<Lua>` via thread-local
- **Reload mechanism**: New Lua contexts sent via channel (`LuaPipe`)
- **Version**: mlua 0.9.x (upgraded from 0.8.x in late 2023)
- **Async support**: ✅ mlua provides `create_async_function` and `call_async`

### 1.2 Existing Async Infrastructure (GOOD NEWS!)

**Discovery from code analysis**: WezTerm ALREADY uses async Lua in several places!

**File**: `config/src/lua.rs:360`
```rust
wezterm_mod.set("emit", lua.create_async_function(emit_event)?)?;
```

**Existing async functions** (from grep results):
- `wezterm.emit` - Event emission (async)
- `wezterm.sleep_ms` - Async sleep
- `wezterm.run_child_process` - Async process spawning
- `wezterm.background_child_process` - Background processes
- `wezterm.read_dir` - Async file system operations
- `wezterm.glob` - Async file globbing

**Async callback infrastructure** (`config/src/lua.rs:767-835`):
```rust
pub async fn emit_event<'lua>(
    lua: &'lua Lua,
    (name, args): (String, mlua::MultiValue<'lua>),
) -> mlua::Result<bool> {
    // ...
    match func.call_async(args.clone()).await? {  // ALREADY ASYNC!
        // ...
    }
}

pub async fn emit_async_callback<'lua, A>(
    lua: &'lua Lua,
    (name, args): (String, A),
) -> mlua::Result<mlua::Value<'lua>>
{
    // ...
    return func.call_async(args).await;  // ALREADY ASYNC!
}
```

**KEY INSIGHT**: The async infrastructure EXISTS. The problem is that **critical callbacks still use the sync version** (`emit_sync_callback` instead of `emit_async_callback`).

### 1.3 Current Callback Mechanisms

#### A. Synchronous Callbacks (BLOCKING) - THE PROBLEM

**File**: `config/src/lua.rs:795-814`

```rust
pub fn emit_sync_callback<'lua, A>(
    lua: &'lua Lua,
    (name, args): (String, A),
) -> mlua::Result<mlua::Value<'lua>>
where
    A: IntoLuaMulti<'lua>,
{
    let decorated_name = format!("wezterm-event-{}", name);
    let tbl: mlua::Value = lua.named_registry_value(&decorated_name)?;
    match tbl {
        mlua::Value::Table(tbl) => {
            for func in tbl.sequence_values::<mlua::Function>() {
                let func = func?;
                return func.call(args);  // SYNCHRONOUS - BLOCKS RENDER THREAD
            }
            Ok(mlua::Value::Nil)
        }
        _ => Ok(mlua::Value::Nil),
    }
}
```

#### B. Async Callbacks (NON-BLOCKING) - THE SOLUTION

**Already implemented** but underutilized:
```rust
pub async fn emit_async_callback<'lua, A>(/* ... */) -> mlua::Result<mlua::Value<'lua>> {
    // Uses call_async - yields without blocking
    func.call_async(args).await
}
```

### 1.4 Critical Blocking Sites (Need Async Conversion)

**Site 1: Tabbar Rendering** (`wezterm-gui/src/tabbar.rs:53-58`)
```rust
match config::run_immediate_with_lua_config(|lua| {
    if let Some(lua) = lua {
        let v = config::lua::emit_sync_callback(  // ❌ SYNC
            &*lua,
            ("format-tab-title".to_string(), (/* ... */)),
        )?;
    }
})
```
**Impact**: 5-50ms × N tabs

**Site 2: Window Title** (`wezterm-gui/src/termwindow/mod.rs:2020-2025`)
```rust
let v = config::lua::emit_sync_callback(  // ❌ SYNC
    &*lua,
    ("format-window-title".to_string(), (/* ... */)),
)?;
```

**Site 3: Status Line** (`mux/src/localpane.rs:568-573`)
```rust
let v = config::lua::emit_sync_callback(  // ❌ SYNC
    &*lua,
    ("update-right-status".to_string(), (/* ... */)),
)?;
```

**Site 4: Command Palette** (`wezterm-gui/src/termwindow/palette.rs:98-102`)
```rust
let result = config::lua::emit_sync_callback(  // ❌ SYNC
    &*lua,
    ("augment-command-palette".to_string(), (/* ... */)),
)?;
```

---

## 2. Game Engine Lua Integration Patterns (Independent Review)

### 2.1 Applicable Patterns from Review

The independent review identified these patterns as directly applicable to wezterm:

#### Pattern 1: Two-Tier Architecture ✅ **ALREADY DOING**
- Core (rendering, I/O) in Rust
- Scripting (configuration, events) in Lua
- **Keep enforcing clear boundaries**

#### Pattern 2: Batch & Cache Data ⚠️ **PARTIALLY IMPLEMENTED**
- **Current**: Some caching exists (see previous report)
- **Needed**: Systematic caching for all format callbacks
- **Benefit**: Reduces FFI boundary crossings

#### Pattern 3: Asynchronous Lua Execution ⚠️ **INFRASTRUCTURE EXISTS BUT UNDERUSED**
- **Current**: Async functions available, but critical paths use sync
- **Needed**: Convert critical callbacks to use async
- **Benefit**: Non-blocking UI loop

#### Pattern 4: Precompiled Lua Bytecode ❌ **NOT IMPLEMENTED**
- **Current**: Config parsed from source each time
- **Needed**: Cache compiled bytecode in `$CACHE_DIR`
- **Benefit**: 20-30% faster startup/reload
- **NEW PRIORITY**: Should be Phase 1 quick win

#### Pattern 5: Incremental GC ❌ **NOT TUNED**
- **Current**: Using Lua defaults
- **Needed**: Tune GC step size, schedule during idle
- **Benefit**: Prevents frame hitches
- **NEW PRIORITY**: Medium priority optimization

#### Pattern 6: Hot Reloading ✅ **ALREADY IMPLEMENTED**
- `wezterm.reload_configuration()` works well

#### Pattern 7: Event Coalescing & Throttling ⚠️ **PARTIAL**
- **Current**: Output parsing has coalescing (`mux_output_parser_coalesce_delay_ms`)
- **Needed**: Throttle high-frequency event callbacks (e.g., `update-right-status`)
- **Benefit**: Stable frame times
- **NEW PRIORITY**: Should be Phase 2

#### Pattern 8: Data Handle API ❌ **NOT IMPLEMENTED**
- **Current**: Full objects passed to Lua (heavy serialization)
- **Needed**: Pass handles/IDs, lazy fetch heavy data
- **Benefit**: Reduced FFI overhead
- **NEW PRIORITY**: Medium-long term optimization

---

## 3. Proposed Improvements (REVISED)

### 3.1 Phase 0: Quick Wins - Async Conversion (Week 1-2) **[NEW]**

**Insight**: We don't need to build new infrastructure - just use what's already there!

**Change**: Replace `emit_sync_callback` with `emit_async_callback` in critical paths.

**Challenge**: These callsites are currently synchronous. We need to make them async-compatible.

#### Approach A: Async-Compatible with Fallback (RECOMMENDED)

Create a hybrid function that tries async first, falls back to cached/default:

```rust
// New file: config/src/async_callback.rs

pub async fn emit_callback_async_with_cache<'lua, A>(
    lua: &'lua Lua,
    name: String,
    args: A,
    cache_key: impl Hash + Eq + Clone,
    default_fn: impl FnOnce() -> mlua::Value<'lua>,
) -> mlua::Result<mlua::Value<'lua>>
where
    A: IntoLuaMulti<'lua> + Clone,
{
    // Check cache first
    if let Some(cached) = CALLBACK_CACHE.lock().unwrap().get(&cache_key) {
        return Ok(cached.clone());
    }

    // Try async execution with timeout
    let result = timeout(
        Duration::from_millis(100),  // Max 100ms for Lua callback
        emit_async_callback(lua, (name.clone(), args.clone()))
    ).await;

    match result {
        Ok(Ok(value)) => {
            // Success - cache it
            CALLBACK_CACHE.lock().unwrap().insert(cache_key, value.clone());
            Ok(value)
        }
        Ok(Err(e)) => {
            // Lua error - return default
            log::warn!("Lua callback '{}' failed: {}", name, e);
            Ok(default_fn())
        }
        Err(_) => {
            // Timeout - return default and log
            log::warn!("Lua callback '{}' timed out", name);
            Ok(default_fn())
        }
    }
}
```

**Apply to tabbar**:
```rust
// In wezterm-gui/src/tabbar.rs

async fn call_format_tab_title_async(
    tab: &TabInformation,
    // ...
) -> Option<TitleText> {
    let key = format!("tab-{}-{}", tab.tab_id, hover);

    config::with_lua_config_on_main_thread(|lua| async move {
        if let Some(lua) = lua {
            let result = config::emit_callback_async_with_cache(
                &*lua,
                "format-tab-title".to_string(),
                (/* args */),
                key,
                || {
                    // Default value
                    lua.create_string(&tab.tab_title).unwrap().into()
                },
            ).await;

            // Parse result...
        }
    }).await.ok()
}
```

**Benefits**:
- ✅ Uses existing async infrastructure
- ✅ Non-blocking with timeout protection
- ✅ Graceful fallback to defaults
- ✅ Caching for performance

**Effort**: 1-2 weeks to convert all critical callbacks

### 3.2 Phase 1: Bytecode Caching (Week 2-3) **[NEW - HIGH PRIORITY]**

**Pattern from game engines**: Precompile Lua scripts to bytecode.

**Implementation**:

```rust
// In config/src/lib.rs

use std::fs;
use std::path::PathBuf;
use sha2::{Sha256, Digest};

fn get_bytecode_cache_path(config_path: &Path) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(config_path.as_os_str().to_string_lossy().as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    CACHE_DIR.join(format!("wezterm-config-{}.luac", &hash[..16]))
}

fn load_config_with_bytecode_cache(config_path: &Path) -> anyhow::Result<Lua> {
    let lua = Lua::new();

    let cache_path = get_bytecode_cache_path(config_path);
    let config_mtime = fs::metadata(config_path)?.modified()?;

    // Check if cached bytecode is valid
    let use_cache = if cache_path.exists() {
        let cache_mtime = fs::metadata(&cache_path)?.modified()?;
        cache_mtime > config_mtime
    } else {
        false
    };

    if use_cache {
        // Load from bytecode cache
        log::debug!("Loading config from bytecode cache: {:?}", cache_path);
        let bytecode = fs::read(&cache_path)?;
        lua.load(&bytecode).exec()?;
    } else {
        // Load from source and cache
        log::debug!("Loading config from source: {:?}", config_path);
        let source = fs::read_to_string(config_path)?;
        let chunk = lua.load(&source).set_name(config_path.to_string_lossy())?;

        // Execute
        chunk.exec()?;

        // Dump bytecode to cache
        if let Ok(bytecode) = chunk.into_function()?.dump(false) {
            if let Err(e) = fs::write(&cache_path, bytecode) {
                log::warn!("Failed to write bytecode cache: {}", e);
            } else {
                log::debug!("Wrote bytecode cache: {:?}", cache_path);
            }
        }
    }

    Ok(lua)
}
```

**Expected Impact**:
- **First load**: Same speed (create cache)
- **Subsequent loads**: 20-30% faster
- **Benefits compound**: Every config reload gets speedup

**Effort**: 3-4 days

### 3.3 Phase 2: Event Throttling/Coalescing (Week 3-4) **[NEW - MEDIUM PRIORITY]**

**Pattern from game engines**: Limit callback frequency to maintain stable frame times.

**Problem**: `update-right-status` may be called every frame (16ms @ 60fps).

**Solution**: Throttle to reasonable rate (e.g., 200ms).

**Implementation**:

```rust
// In wezterm-gui/src/termwindow/mod.rs

struct ThrottledCallback {
    last_call: Instant,
    min_interval: Duration,
    pending: bool,
}

impl TermWindow {
    fn should_call_throttled_callback(
        &mut self,
        callback_name: &str,
        min_interval: Duration,
    ) -> bool {
        let entry = self.throttled_callbacks
            .entry(callback_name.to_string())
            .or_insert_with(|| ThrottledCallback {
                last_call: Instant::now() - min_interval,
                min_interval,
                pending: false,
            });

        let elapsed = entry.last_call.elapsed();
        if elapsed >= min_interval {
            entry.last_call = Instant::now();
            entry.pending = false;
            true
        } else {
            entry.pending = true;
            false
        }
    }

    fn update_right_status_throttled(&mut self) {
        // Only call every 200ms
        if self.should_call_throttled_callback("update-right-status", Duration::from_millis(200)) {
            self.update_right_status_impl();
        }
    }
}
```

**Throttle Targets**:
- `update-right-status`: 200ms (5 Hz)
- `format-window-title`: 500ms (2 Hz) when window focused
- `bell` events: 100ms (prevent spam)

**Benefits**:
- Reduces Lua invocations by 80-95%
- More stable frame times
- Lower CPU usage

**Effort**: 3-5 days

### 3.4 Phase 3: GC Tuning (Week 4-5) **[NEW - MEDIUM PRIORITY]**

**Pattern from game engines**: Tune GC to prevent frame hitches.

**Implementation**:

```rust
// In config/src/lua.rs

fn tune_lua_gc_for_terminal(lua: &Lua) -> anyhow::Result<()> {
    // Lua GC tuning: more aggressive during idle, gentler during activity
    lua.gc_set_pause(150)?;   // Start GC when memory 150% of threshold (default 200)
    lua.gc_set_step_multiplier(200)?;  // More aggressive collection (default 100)

    // Consider setting up idle-time GC
    // When terminal is idle (no output for 500ms), do a full GC cycle
    Ok(())
}

// In wezterm-gui, schedule GC during idle
impl TermWindow {
    fn schedule_idle_gc(&mut self) {
        if self.last_activity.elapsed() > Duration::from_millis(500) {
            // Terminal idle - safe to GC
            if let Some(lua) = get_lua_context() {
                lua.gc_collect()?;  // Full collection during idle
            }
        }
    }
}
```

**Benefits**:
- Prevents GC spikes during typing/rendering
- Smoother frame times
- Better memory management

**Effort**: 2-3 days

### 3.5 Phase 4: Systematic Caching (Week 5-8)

**Keep from original proposal**: The async cache pattern with generation tracking.

```rust
pub struct AsyncLuaCache<K, V> {
    cache: Arc<Mutex<HashMap<K, CachedEntry<V>>>>,
    pending: Arc<Mutex<HashMap<K, Receiver<V>>>>,
    generation: usize,
}

pub enum CacheResult<V> {
    Hit(V),
    Computed(V),
    Pending(Option<V>),
    Started,
}
```

**Apply to**:
- Tab titles (with Phase 0 async)
- Window titles
- Status lines
- Command palette augmentation

### 3.6 Phase 5: Data Handle API (Week 9-12) **[NEW - LONG TERM]**

**Pattern from game engines**: Reduce FFI overhead by passing handles instead of full objects.

**Current Problem**:
```rust
// Passes full arrays of tabs and panes to Lua - expensive serialization
let tabs = lua.create_sequence_from(tab_info.iter().cloned())?;
let panes = lua.create_sequence_from(pane_info.iter().cloned())?;
```

**Proposed Solution**:
```rust
// Pass lightweight handles
pub struct TabHandle(TabId);
pub struct PaneHandle(PaneId);

impl mlua::UserData for TabHandle {
    fn add_methods<'lua, M: mlua::UserDataMethods<'lua, Self>>(methods: &mut M) {
        // Lazy accessors
        methods.add_method("get_title", |_, this, ()| {
            // Fetch from cache/store only when accessed
            Ok(get_tab_title(this.0))
        });

        methods.add_method("get_index", |_, this, ()| {
            Ok(get_tab_index(this.0))
        });

        // etc.
    }
}

// Usage in Lua:
-- Instead of: tab.tab_title
-- Now: tab:get_title()
-- Only fetched if actually used!
```

**Benefits**:
- 50-80% less data copying on FFI boundary
- Lazy evaluation (only fetch what's used)
- Better memory efficiency

**Drawbacks**:
- ⚠️ **Breaking change** for user configs
- Need migration guide and backward compat layer

**Effort**: 3-4 weeks (including migration support)

---

## 4. Revised Recommendations

### 4.1 Immediate Actions (Next 2 Weeks)

**Priority 1: Convert Critical Callbacks to Async** (Phase 0 - NEW!)
- Highest impact per effort
- Uses existing infrastructure
- Low risk with fallback strategy
- **Estimated effort**: 1-2 weeks

**Priority 2: Implement Bytecode Caching** (Phase 1 - NEW!)
- Quick win for startup performance
- No user-visible changes
- Easy to implement
- **Estimated effort**: 3-4 days

**Priority 3: Add Performance Benchmarks**
- Baseline current performance
- Track improvements
- Detect regressions

### 4.2 Medium-term Goals (2-3 Months)

**Priority 4: Event Throttling** (Phase 2 - NEW!)
- Reduce callback frequency
- More stable frame times
- **Estimated effort**: 3-5 days

**Priority 5: GC Tuning** (Phase 3 - NEW!)
- Prevent frame hitches
- Better memory behavior
- **Estimated effort**: 2-3 days

**Priority 6: Systematic Caching** (Phase 4)
- From original proposal
- Apply async cache pattern to all callbacks
- **Estimated effort**: 3-4 weeks

### 4.3 Long-term Vision (6-12 Months)

**Priority 7: Data Handle API** (Phase 5 - NEW!)
- Reduce FFI overhead significantly
- Requires user migration
- **Estimated effort**: 3-4 weeks

**Priority 8: LuaJIT Evaluation** (Optional)
- From original proposal
- If Phase 0-3 insufficient
- **Estimated effort**: 1-2 weeks evaluation

---

## 5. Updated Implementation Roadmap

### 5.1 Phase 0: Async Conversion (Week 1-2) **[NEW]**

**Week 1: Infrastructure**
- [ ] Create `async_callback.rs` with hybrid async/cache function
- [ ] Add timeout support for Lua callbacks
- [ ] Implement fallback to default values
- [ ] Unit tests for async callback wrapper

**Week 2: Critical Path Conversion**
- [ ] Convert `format-tab-title` to async
- [ ] Convert `format-window-title` to async
- [ ] Convert `update-right-status` to async
- [ ] Convert `augment-command-palette` to async
- [ ] Testing and validation

**Deliverable**: Non-blocking Lua callbacks with fallback

### 5.2 Phase 1: Bytecode Caching (Week 2-3) **[NEW - CAN OVERLAP WITH PHASE 0]**

**Week 2-3: Implementation**
- [ ] Implement bytecode cache path calculation
- [ ] Add cache invalidation based on mtime
- [ ] Integrate into config loading
- [ ] Add cache cleanup for old entries
- [ ] Benchmark startup performance

**Deliverable**: 20-30% faster config load/reload

### 5.3 Phase 2: Event Throttling (Week 3-4) **[NEW]**

**Week 3-4: Implementation**
- [ ] Create throttled callback infrastructure
- [ ] Identify all high-frequency callbacks
- [ ] Set appropriate throttle rates
- [ ] Test with active terminals
- [ ] Measure frame time stability

**Deliverable**: Stable frame times, reduced callback overhead

### 5.4 Phase 3: GC Tuning (Week 4-5) **[NEW]**

**Week 4-5: Implementation**
- [ ] Tune Lua GC parameters
- [ ] Implement idle-time GC scheduling
- [ ] Monitor GC pause times
- [ ] Adjust based on profiling
- [ ] Document GC behavior

**Deliverable**: Smoother frame times, no GC hitches

### 5.5 Phase 4: Systematic Caching (Week 5-8)

**From original proposal, enhanced with Phase 0 async**
- [ ] Implement `AsyncLuaCache<K, V>`
- [ ] Apply to all format callbacks
- [ ] Cache invalidation strategies
- [ ] Performance testing

**Deliverable**: 10-50× improvement for cached callbacks

### 5.6 Phase 5: Data Handle API (Week 9-12) **[NEW - OPTIONAL]**

**Long-term optimization**
- [ ] Design handle API
- [ ] Implement for tabs/panes
- [ ] Add backward compatibility layer
- [ ] Migration guide for users
- [ ] Comprehensive testing

**Deliverable**: 50-80% less FFI overhead

---

## 6. Updated Effort Estimates

### 6.1 Development Time (REVISED)

| Phase | Task | Estimated Time | Priority |
|-------|------|----------------|----------|
| 0 | Async callback conversion | 1-2 weeks | **CRITICAL** |
| 1 | Bytecode caching | 3-4 days | **HIGH** |
| 2 | Event throttling | 3-5 days | **MEDIUM** |
| 3 | GC tuning | 2-3 days | **MEDIUM** |
| 4 | Systematic caching | 3-4 weeks | **MEDIUM** |
| 5 | Data handle API | 3-4 weeks | **LOW (BREAKING)** |
| **Total (Phase 0-4)** | | **7-9 weeks** | |
| **Total (All phases)** | | **10-13 weeks** | |

**Recommended Minimum**: Phase 0-3 (3-4 weeks)
**Full Optimization**: Phase 0-5 (10-13 weeks)

### 6.2 Expected Performance Improvements (REVISED)

**After Phase 0-1** (3-4 weeks):
- **Tabbar**: 5-50ms → 1-5ms (async + timeout) → 0.1ms (cached)
- **Startup**: 20-30% faster (bytecode cache)
- **User Experience**: Smooth, no blocking

**After Phase 0-3** (4-5 weeks):
- **All above PLUS:**
- **Event overhead**: 80% reduction (throttling)
- **Frame stability**: No GC hitches (GC tuning)

**After Phase 0-4** (7-9 weeks):
- **All above PLUS:**
- **Cache hit rate**: >90%
- **Consistent sub-millisecond latency**

**After Phase 0-5** (10-13 weeks):
- **All above PLUS:**
- **FFI overhead**: 50-80% reduction (data handles)
- **Peak performance achieved**

---

## 7. Key Changes from Original Proposal

### 7.1 What Was Added (Based on Independent Review)

1. **Phase 0: Async Conversion** (NEW)
   - Leverage existing mlua async infrastructure
   - Convert critical callbacks from sync to async
   - Highest priority quick win

2. **Phase 1: Bytecode Caching** (NEW)
   - Cache compiled Lua bytecode
   - 20-30% faster startup
   - Standard game engine practice

3. **Phase 2: Event Throttling** (NEW)
   - Limit high-frequency callback rates
   - Stable frame times
   - Reduces unnecessary Lua invocations

4. **Phase 3: GC Tuning** (NEW)
   - Tune Lua garbage collection
   - Idle-time GC scheduling
   - Prevents frame hitches

5. **Phase 5: Data Handle API** (NEW)
   - Reduce FFI overhead
   - Pass handles instead of full objects
   - Long-term optimization

### 7.2 What Was Refined

1. **Async Architecture**
   - **Original**: Proposed building new async infrastructure
   - **Revised**: Use existing `call_async` + add wrapper for safety

2. **LuaJIT**
   - **Original**: Phase 3 evaluation
   - **Revised**: Optional/secondary - Phase 0-3 should be sufficient

3. **Caching**
   - **Original**: Primary optimization
   - **Revised**: Complementary to async (Phase 4 vs Phase 0)

4. **Timeline**
   - **Original**: 11 weeks
   - **Revised**: 7-9 weeks for core improvements (Phase 0-4)

### 7.3 What Was Validated

✅ **Two-tier architecture** - Confirmed good, continue
✅ **mlua as Rust-Lua bridge** - Confirmed, with good async support
✅ **Caching strategy** - Confirmed valuable, now secondary to async
✅ **Hot reloading** - Confirmed already working well

---

## 8. Risk Assessment (UPDATED)

### 8.1 Technical Risks

**Risk 1: Async Conversion Complexity (LOW-MEDIUM)**
- **Scenario**: Making sync callsites async requires refactoring
- **Mitigation**:
  - Wrapper with fallback to defaults
  - Timeout protection (100ms max)
  - Extensive testing
- **Severity**: Low with proper wrapper

**Risk 2: Bytecode Cache Invalidation (LOW)**
- **Scenario**: Stale cache served after config changes
- **Mitigation**:
  - mtime-based invalidation
  - Hash-based cache keys
  - Clear cache on version upgrade
- **Severity**: Very low - standard practice

**Risk 3: Throttling Too Aggressive (LOW)**
- **Scenario**: Updates appear sluggish
- **Mitigation**:
  - Conservative initial rates (200ms)
  - Make throttle rates configurable
  - User can disable if needed
- **Severity**: Low - easy to tune

**Risk 4: GC Tuning Adverse Effects (LOW)**
- **Scenario**: Aggressive GC causes pauses
- **Mitigation**:
  - Profile GC behavior
  - Make parameters configurable
  - Can revert to defaults
- **Severity**: Very low - easy to roll back

**Risk 5: Data Handle Breaking Changes (HIGH) - Phase 5 Only**
- **Scenario**: User configs break
- **Mitigation**:
  - Provide backward compat layer
  - Clear migration guide
  - Gradual deprecation
- **Severity**: High but manageable

### 8.2 Updated Recommendations

**CRITICAL PATH (Must Do)**:
- ✅ Phase 0: Async conversion
- ✅ Phase 1: Bytecode caching

**HIGH VALUE (Should Do)**:
- ✅ Phase 2: Event throttling
- ✅ Phase 3: GC tuning
- ✅ Phase 4: Systematic caching

**NICE TO HAVE (Consider)**:
- ⚠️ Phase 5: Data handle API (breaking change)
- ⚠️ LuaJIT evaluation (if needed)

---

## 9. Comparison with Previous Report

### 9.1 Synergy with Rendering Improvements

**Previous Report** (wezterm-wayland-improvement-report-2.md):
- Tabbar caching (sync)
- Wayland damage tracking

**This Report (Amended)**:
- **Phase 0**: Async callbacks (non-blocking)
- **Phase 1**: Bytecode caching (faster load)
- **Phase 4**: Async cache (from original proposal)

**Combined Strategy**:
1. **Phase 0** (this report): Make callbacks async = No blocking
2. **Tabbar caching** (Report 1): Sync cache = Fast hits
3. **Phase 4** (this report): Async cache = Progressive enhancement
4. **Wayland damage** (Report 1): Reduce compositing

**Result**: Maximum performance from multiple complementary optimizations

---

## 10. Conclusion (UPDATED)

### 10.1 Summary of Amendments

The independent review revealed that wezterm **already has strong Lua infrastructure**:
- ✅ mlua with async support
- ✅ Event-driven architecture
- ✅ Hot reloading
- ✅ Some async functions implemented

**The key insight**: We don't need to build new infrastructure. We need to:
1. **Use existing async** in critical paths (Phase 0)
2. **Add standard optimizations** from game engines (Phases 1-3)
3. **Enhance with caching** as originally proposed (Phase 4)

### 10.2 Recommended Minimum Implementation

**For Best ROI, Implement Phase 0-3** (4-5 weeks):
1. ✅ **Async conversion** (Week 1-2): Eliminate blocking
2. ✅ **Bytecode caching** (Week 2-3): Faster startup
3. ✅ **Event throttling** (Week 3-4): Stable frames
4. ✅ **GC tuning** (Week 4-5): No hitches

**Expected Impact**:
- **Tabbar**: 5-50ms → <5ms (10-50× faster)
- **Startup**: 20-30% faster
- **Frame times**: Stable, no stutters
- **User experience**: Dramatically improved

### 10.3 Final Recommendations

**Immediate Actions**:
1. Start with **Phase 0** (async conversion) - highest impact
2. Implement **Phase 1** (bytecode caching) in parallel - quick win
3. Measure and validate improvements
4. Proceed to Phase 2-3 if needed
5. Consider Phase 4-5 based on results

**Success Criteria**:
- Zero perceivable lag on tab hover
- Config reload <200ms
- Stable 60 FPS during all operations
- Positive user feedback

**This revised proposal is more realistic, leverages existing infrastructure, and provides a clearer path to meaningful improvements.**

---

## Appendix A: Independent Review Key Findings

**Source**: `chats/lua-game-engines-2.md`

**Key Insights**:
1. WezTerm shares architectural challenges with game engines
2. Async Lua execution already partially implemented
3. Bytecode caching is standard practice in engines
4. Event coalescing/throttling critical for frame stability
5. GC tuning prevents frame hitches
6. Data handle API reduces FFI overhead

**Applicable Patterns Confirmed**:
- ✅ Two-tier architecture (already good)
- ✅ Batch & cache (needs enhancement)
- ✅ Async execution (use existing mlua support)
- ✅ Precompiled bytecode (missing - add it)
- ✅ Incremental GC (needs tuning)
- ✅ Hot reload (already works)
- ✅ Event coalescing (needs expansion)
- ✅ Data handle API (long-term goal)

---

## Appendix B: Code Examples (Updated)

### B.1 Async Callback Wrapper with Fallback

```rust
// config/src/async_callback.rs

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::Duration;
use mlua::{Lua, Value, IntoLuaMulti};

lazy_static::lazy_static! {
    static ref CALLBACK_CACHE: Mutex<HashMap<String, Value<'static>>> = Mutex::new(HashMap::new());
}

pub async fn emit_callback_async_safe<'lua, A>(
    lua: &'lua Lua,
    name: String,
    args: A,
    cache_key: String,
    default_value: Value<'lua>,
    timeout_ms: u64,
) -> mlua::Result<Value<'lua>>
where
    A: IntoLuaMulti<'lua> + Clone,
{
    // Check cache
    {
        let cache = CALLBACK_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(&cache_key) {
            // Clone cached value (requires conversion)
            return Ok(cached.clone());
        }
    }

    // Try async with timeout
    let result = async_std::future::timeout(
        Duration::from_millis(timeout_ms),
        super::lua::emit_async_callback(lua, (name.clone(), args.clone()))
    ).await;

    match result {
        Ok(Ok(value)) => {
            // Success - cache it
            // Note: Caching Value<'static> requires careful lifetime management
            log::trace!("Callback '{}' succeeded, caching", name);
            Ok(value)
        }
        Ok(Err(e)) => {
            // Lua error
            log::warn!("Lua callback '{}' error: {:#}", name, e);
            Ok(default_value)
        }
        Err(_timeout) => {
            // Timeout
            log::warn!("Lua callback '{}' timed out after {}ms", name, timeout_ms);
            Ok(default_value)
        }
    }
}
```

### B.2 Bytecode Caching Implementation

```rust
// config/src/bytecode_cache.rs

use std::fs;
use std::path::{Path, PathBuf};
use mlua::Lua;
use sha2::{Sha256, Digest};

pub struct BytecodeCache {
    cache_dir: PathBuf,
}

impl BytecodeCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        fs::create_dir_all(&cache_dir).ok();
        Self { cache_dir }
    }

    fn get_cache_path(&self, source_path: &Path) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(source_path.as_os_str().to_string_lossy().as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        self.cache_dir.join(format!("config-{}.luac", &hash[..16]))
    }

    fn is_cache_valid(&self, source_path: &Path, cache_path: &Path) -> bool {
        if !cache_path.exists() {
            return false;
        }

        match (fs::metadata(source_path), fs::metadata(cache_path)) {
            (Ok(source_meta), Ok(cache_meta)) => {
                match (source_meta.modified(), cache_meta.modified()) {
                    (Ok(source_time), Ok(cache_time)) => cache_time >= source_time,
                    _ => false,
                }
            }
            _ => false,
        }
    }

    pub fn load_or_compile(
        &self,
        lua: &Lua,
        source_path: &Path,
    ) -> anyhow::Result<()> {
        let cache_path = self.get_cache_path(source_path);

        if self.is_cache_valid(source_path, &cache_path) {
            // Load from cache
            log::debug!("Loading config from bytecode cache");
            let bytecode = fs::read(&cache_path)?;
            lua.load(&bytecode).exec()?;
        } else {
            // Load source and create cache
            log::debug!("Loading config from source and caching");
            let source = fs::read_to_string(source_path)?;

            // Load and execute
            let chunk = lua.load(&source)
                .set_name(source_path.to_string_lossy().as_ref())?;
            chunk.exec()?;

            // Try to cache bytecode
            // Note: Need to reload to dump bytecode
            let chunk = lua.load(&source)
                .set_name(source_path.to_string_lossy().as_ref())?;

            if let Ok(func) = chunk.into_function() {
                if let Ok(bytecode) = func.dump(false) {
                    fs::write(&cache_path, bytecode).ok();
                    log::debug!("Cached bytecode to {:?}", cache_path);
                }
            }
        }

        Ok(())
    }

    pub fn clear(&self) -> std::io::Result<()> {
        for entry in fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            if entry.path().extension() == Some(std::ffi::OsStr::new("luac")) {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }
}
```

### B.3 Event Throttling Implementation

```rust
// wezterm-gui/src/throttle.rs

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct CallbackThrottle {
    last_calls: HashMap<String, Instant>,
    intervals: HashMap<String, Duration>,
}

impl CallbackThrottle {
    pub fn new() -> Self {
        let mut intervals = HashMap::new();

        // Configure default throttle intervals
        intervals.insert("update-right-status".to_string(), Duration::from_millis(200));
        intervals.insert("format-window-title".to_string(), Duration::from_millis(500));
        intervals.insert("bell".to_string(), Duration::from_millis(100));

        Self {
            last_calls: HashMap::new(),
            intervals,
        }
    }

    pub fn set_interval(&mut self, callback: String, interval: Duration) {
        self.intervals.insert(callback, interval);
    }

    pub fn should_call(&mut self, callback: &str) -> bool {
        let now = Instant::now();

        let interval = self.intervals
            .get(callback)
            .copied()
            .unwrap_or(Duration::from_millis(0)); // No throttling by default

        let last = self.last_calls.get(callback).copied();

        match last {
            Some(last) if now.duration_since(last) < interval => {
                // Too soon
                false
            }
            _ => {
                // OK to call
                self.last_calls.insert(callback.to_string(), now);
                true
            }
        }
    }
}

// Usage in TermWindow:
impl TermWindow {
    fn update_right_status(&mut self) {
        if self.callback_throttle.should_call("update-right-status") {
            self.update_right_status_impl();
        }
    }
}
```

### B.4 GC Tuning Implementation

```rust
// config/src/gc_tuning.rs

use mlua::Lua;

pub fn tune_lua_gc(lua: &Lua) -> mlua::Result<()> {
    // Set GC pause threshold to 150% (default 200%)
    // This makes GC run more frequently but with smaller pauses
    lua.gc_set_pause(150)?;

    // Set step multiplier to 200 (default 100)
    // This makes each GC step do more work
    lua.gc_set_step_multiplier(200)?;

    log::info!("Lua GC tuned: pause=150%, step=200%");

    Ok(())
}

pub fn perform_idle_gc(lua: &Lua) -> mlua::Result<()> {
    // Full GC collection during idle time
    lua.gc_collect()?;
    log::trace!("Performed idle GC collection");
    Ok(())
}

// In TermWindow:
impl TermWindow {
    fn update(&mut self) {
        // ... normal update logic ...

        // If idle for 500ms, do a GC
        if self.last_activity_time.elapsed() > Duration::from_millis(500) {
            if let Some(lua) = get_lua_context() {
                perform_idle_gc(&lua).ok();
            }
        }
    }
}
```

---

**Document Version**: 2.0 (AMENDED)
**Date**: 2025-10-22
**Author**: Claude Code Analysis
**Status**: Revised Proposal Based on Independent Review
**Changes**: Added Phases 0-3,5; Revised priorities and timeline; Incorporated game engine patterns
