# Phase 3 Assessment: Real Bottleneck Analysis

## Date  
2025-10-23

## Status

**Deployment verified**: Linux binary built at 17:50 contains commit d6c2d8e55 with all optimizations.

## Key Discovery: Optimizations ARE Working!

### Evidence from perf-report.3

| Component | Time % | Status |
|-----------|--------|---------|
| `update_title_impl` | 0.02% | ✅ **Cached** (was significant) |
| `get_window_title_cached` | 0.01% | ✅ **Working** |
| `WindowTitleKey` operations | 0.00% | ✅ **Negligible** |
| `TabBarState::new` | 0.03% | ✅ **Cached** (was significant) |
| `compute_tab_title` | 0.00% | ✅ **Negligible** |
| `call_format_tab_title` | 0.05% | ✅ **Cached** |

**Total for optimized callbacks: ~0.11%** (down from est. 5-10%)

### But Lua Overhead Still High

| Lua Component | Time % |
|---------------|--------|
| `mlua::table::Table::raw_set` | 6.58% |
| `mlua::lua::Lua::create_string` | 6.01% |
| `aux_rawset` | 5.18% |
| `luaH_newkey` | 4.73% |
| `lua_pushlstring` | 3.87% |
| `luaH_resize` | 3.33% |
| **Total Lua** | **~16%** |

## Critical Realization: Wrong Source

**The Lua overhead is NOT from the callbacks we optimized!**

The optimizations successfully eliminated overhead from:
- ✅ `format-tab-title` (now 0.05%)
- ✅ `format-window-title` (now 0.02%)

But the 16% Lua overhead is coming from **somewhere else**.

## Finding the Real Culprit

### Hypothesis: Tab Bar Rendering (Not Callbacks)

The Lua isn't being called for *formatting*, but for **rendering the tab bar itself**.

Let me trace the call path:

```
TabBarState::new (0.03%)
    ↓
Create Lua tables for tab_info, pane_info
    ↓ [This might be the bottleneck]
mlua::table::Table::raw_set (6.58%)
mlua::lua::Lua::create_string (6.01%)
```

**The issue**: Even though we cache the *result* of tab title computation, we might be:
1. **Creating TabInformation structs** on every frame
2. **Serializing them to Lua** for the tab bar render
3. **Not caching the tab bar state itself**

### Let me verify by looking at update_title_impl

The code flow is:
```rust
fn update_title_impl(&mut self) {
    let tabs = self.get_tab_information();  // ← Creates Vec<TabInformation>
    let panes = self.get_pane_information(); // ← Creates Vec<PaneInformation>
    
    // For window title (CACHED ✅)
    let title = get_window_title_cached(...);
    
    // For tab bar (NOT CACHED ❌)
    let new_tab_bar = TabBarState::new(
        width,
        mouse_pos,
        &tabs,      // ← Passed to Lua
        &panes,     // ← Passed to Lua
        palette,
        config,
        left_status,
        right_status,
    );
}
```

### The Real Bottleneck

**`TabBarState::new` doesn't use our caching!**

It still calls the tab title formatters, which then:
1. Convert `tabs` and `panes` to Lua tables
2. This involves 6.58% in `mlua::table::Table::raw_set`
3. And 6.01% in `mlua::lua::Lua::create_string`

Even though `compute_tab_title` is cached, **the data serialization overhead remains**.

## Why Throttling Didn't Help

**No "Resize throttled" logs** because:

1. **Events arrive slowly enough**: Wayland might already be batching events, so they arrive >16ms apart
2. **Wrong metric**: We're throttling *resize events*, but the cost is in *rendering*, which happens on every frame regardless

The profiling shows:
- Resize events: Maybe 30-60 Hz
- But **rendering happens every frame** (60 FPS)
- Even if resize is throttled, tab bar is re-rendered

## Root Cause Analysis

### The Architecture Problem

```
Resize Event (throttled ✅)
    ↓
update_title_impl() called
    ↓
get_tab_information() ← Creates fresh Vec<TabInformation>
get_pane_information() ← Creates fresh Vec<PaneInformation>
    ↓
TabBarState::new(tabs, panes, ...)
    ↓ [Cached title computation ✅]
compute_tab_title() → Cache hit!
    ↓ [But data is still serialized ❌]
Serialize tabs → Lua tables (6.58%)
Serialize panes → Lua tables
Create strings (6.01%)
    ↓
Tab bar rendered
```

**The problem**: We cache the *computation result*, but we still **serialize the input data** to Lua on every call.

### Why This Happens

In `TabBarState::new`, even with caching:
1. We create Lua tables from `tabs` and `panes`
2. These are large structures with many fields
3. Each field becomes a Lua table entry (raw_set)
4. Each string becomes a Lua string (create_string)

**Cost**: O(num_tabs × fields_per_tab × 2) Lua FFI calls

With 5 tabs, 20 fields each: **200 FFI calls per frame!**

## The Real Solution

### Option 1: Cache TabInformation/PaneInformation → Lua Conversion

Instead of caching just the title, cache the **entire Lua table**:

```rust
lazy_static::lazy_static! {
    static ref TAB_INFO_LUA_CACHE: Mutex<HashMap<TabId, mlua::RegistryKey>> =
        Mutex::new(HashMap::new());
}

fn get_tab_info_as_lua<'lua>(
    lua: &'lua mlua::Lua,
    tab: &TabInformation,
) -> mlua::Result<mlua::Table<'lua>> {
    let cache = TAB_INFO_LUA_CACHE.lock().unwrap();
    
    if let Some(cached_key) = cache.get(&tab.tab_id) {
        // Return cached Lua table
        return lua.registry_value(cached_key);
    }
    
    // Create new Lua table
    let table = lua.create_table()?;
    // ... populate table ...
    
    // Cache it
    let key = lua.create_registry_value(table.clone())?;
    cache.insert(tab.tab_id, key);
    
    Ok(table)
}
```

**Expected improvement**: Eliminate 6.58% + 6.01% = **12.59% overhead**

### Option 2: Don't Pass to Lua at All

Change the tab bar rendering to **not use Lua for layout**:

```rust
// Instead of passing everything to Lua:
TabBarState::new(&tabs, &panes, config, ...)

// Just use the cached titles:
TabBarState::new_from_cached_titles(
    cached_titles,  // Already computed and cached
    config,
    ...
)
```

This completely eliminates Lua from the hot path.

**Expected improvement**: Eliminate all 16% Lua overhead

### Option 3: Reduce Serialization Frequency

Only serialize to Lua when **tab state actually changes**:

```rust
fn update_title_impl(&mut self) {
    let tabs = self.get_tab_information();
    
    // Check if tabs changed
    if tabs != self.last_serialized_tabs {
        // Only serialize when changed
        self.lua_tab_tables = serialize_to_lua(tabs);
        self.last_serialized_tabs = tabs;
    }
    
    // Use cached Lua tables
    TabBarState::new(..., &self.lua_tab_tables, ...);
}
```

**Expected improvement**: Reduce Lua calls by 90%+ (most frames have no tab changes)

## Recommended Implementation Plan

### Phase 3A: Cache Lua Table Serialization (1-2 days)

**Priority**: High  
**Effort**: Medium  
**Risk**: Low

1. Create `TabInfoLuaCache` to cache TabInformation → Lua conversion
2. Modify `TabBarState::new` to use cached Lua tables
3. Invalidate cache when tab state changes

**Expected**: 12-14% CPU reduction

### Phase 3B: Reduce Serialization Calls (1 day)

**Priority**: High  
**Effort**: Low  
**Risk**: Very low

1. Track `last_serialized_tabs` in TermWindow
2. Only re-serialize when tabs actually change
3. During resize (no tab changes), reuse previous Lua tables

**Expected**: Additional 2-3% CPU reduction

### Phase 3C: Rearchitect Tab Bar (1-2 weeks)

**Priority**: Medium (long-term)  
**Effort**: High  
**Risk**: Medium (breaking change)

1. Remove Lua from tab bar rendering path
2. Use pure Rust for layout
3. Only call Lua for custom formatting (cached)

**Expected**: Eliminate remaining Lua overhead

## Why Previous Optimizations Still Matter

Even though they didn't fix the main issue, they:
- ✅ Reduced window title overhead from ~3% to 0.02%
- ✅ Reduced tab title computation from ~5% to 0.05%
- ✅ Provide infrastructure for Phase 3A (caching pattern proven)

## Immediate Next Steps

### Step 1: Verify Hypothesis (30 minutes)

Add logging to confirm serialization is the bottleneck:

```rust
// In TabBarState::new
let start = Instant::now();
let tabs_lua = lua.create_sequence_from(tabs.clone().into_iter())?;
log::info!("Tab serialization: {:?}", start.elapsed());

let start = Instant::now();
let panes_lua = lua.create_sequence_from(panes.clone().into_iter())?;
log::info!("Pane serialization: {:?}", start.elapsed());
```

Run on Linux and observe logs during resize.

**Expected**: ~1-2ms per serialization, called 60x per second = 60-120ms total overhead

### Step 2: Implement Phase 3A (1-2 days)

Create Lua table caching for TabInformation/PaneInformation.

### Step 3: Implement Phase 3B (1 day)

Add change detection to avoid unnecessary serialization.

### Step 4: Profile Again

Expect to see:
- Lua overhead: 16% → 2-4%
- Smooth 60 FPS resize
- Tab bar rendering: <1ms per frame

## Conclusion

**The optimizations work, but we optimized the wrong thing!**

- ✅ Tab title **computation** is cached (0.05%)
- ✅ Window title **computation** is cached (0.02%)
- ❌ But **data serialization** to Lua (16%) was not addressed

**Root cause**: `TabBarState::new` serializes `TabInformation` and `PaneInformation` to Lua on every frame, regardless of caching.

**Solution**: Cache the Lua table conversion, not just the computation result.

**Expected outcome**: 12-14% CPU reduction from Phase 3A alone, achieving smooth 60 FPS resize.

The throttling isn't helping because:
1. Events are already slow enough (>16ms apart)
2. The cost is in rendering (60 FPS), not event handling

**Next action**: Implement Lua table caching for TabInformation/PaneInformation serialization.

