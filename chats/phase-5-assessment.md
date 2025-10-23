# Phase 5 Assessment: Memory Operations Now the Bottleneck

## Date
2025-10-23

## Status

**Phase 4 optimizations deployed**: ✅ Confirmed working  
**Performance**: ❌ Still slow despite 4% Lua reduction  
**New bottleneck discovered**: Memory operations (14.33%)

## Key Finding: Lua Optimizations Worked, But Uncovered Deeper Issue

### What We Fixed (Phase 4) ✅

| Component | Before (perf-4) | After (perf-5) | Improvement |
|-----------|----------------|----------------|-------------|
| `get_or_create` | 1.22% | 0.02% | ✅ **-1.20%** (removed `Instant::now()`) |
| `mlua::table::raw_set` | 6.32% | 5.28% | ✅ **-1.04%** (fewer calls) |
| `mlua::lua::Lua::create_string` | 5.78% | 4.64% | ✅ **-1.14%** (fewer calls) |
| `mlua::function::Function::call` | 3.02% | 2.94% | ✅ **-0.08%** (fewer calls) |
| **Total Lua overhead** | **~17%** | **~13%** | ✅ **-4% reduction!** |

**Phase 4 optimizations ARE working!**

### But the System is Still Slow

**New #1 bottleneck: `__memmove_avx512_unaligned_erms` at 14.33%**

This is memory copying - moving data around in RAM.

### Current Performance Profile (perf-report.5)

| Component | CPU % | Notes |
|-----------|-------|-------|
| **Memory operations** | | |
| `__memmove_avx512_unaligned_erms` | 14.33% | ❌ **NEW bottleneck!** |
| `alloc::raw_vec::RawVecInner` | 2.59% | Vector allocations |
| `core::slice::raw::from_raw_parts` | 2.43% | Slice operations |
| `alloc::alloc::Global::alloc_impl` | 0.96% | Memory allocation |
| **Subtotal: Memory** | **~20%** | **Dominant issue** |
| **Lua operations** | | |
| `mlua::table::Table::raw_set` | 5.28% | ✅ Down from 6.32% |
| `mlua::lua::Lua::create_string` | 4.64% | ✅ Down from 5.78% |
| `mlua::function::Function::call` | 2.94% | ✅ Down from 3.02% |
| `luahelper::dynamic_to_lua_value` | 1.21% | Conversion |
| `mlua::util::push_table` | 1.20% | Table creation |
| `mlua::lua::Lua::push_ref` | 1.09% | Reference handling |
| **Subtotal: Lua** | **~16%** | Still significant |
| **Other** | | |
| BTree operations | 1.67% | HashMap overhead |
| Slice comparison | 1.56% | String/data comparison |
| **TOTAL** | **~40%** | **Major overhead!** |

## Root Cause Analysis

### Why Memory Operations Dominate

**The 14.33% `memmove` is likely from:**

1. **String cloning** (tab titles, pane titles, etc.)
2. **Vector resizing** (growing Vec<TabInformation>, Vec<PaneInformation>)
3. **Data copying** (passing TabInformation/PaneInformation around)
4. **Lua string operations** (converting Rust strings → Lua strings)

### Evidence

Looking at the code flow:

```rust
// In TabBarState::new():

// Pass 1: Clone all tab_info data
let initial_titles: Vec<TitleText> = tab_info
    .iter()
    .map(|tab| compute_tab_title(tab, tab_info, pane_info, ...))  // ← Clones
    .collect();  // ← Allocates Vec

// Pass 2: Clone again for final titles
let tab_titles: Vec<TitleText> = tab_info
    .iter()
    .map(|tab| compute_tab_title(tab, tab_info, pane_info, ...))  // ← Clones again!
    .collect();  // ← Allocates another Vec

// In compute_tab_title() → call_format_tab_title():
let tabs = get_tabs_as_lua_sequence(&lua, tab_info)?;  // ← Creates Lua tables (may clone)
let panes = get_panes_as_lua_sequence(&lua, pane_info)?;  // ← Creates Lua tables (may clone)

// Lua callback returns:
let result: Vec<FormatItem> = ...;  // ← Allocates result
```

**For 5 tabs at 60 FPS**:
- 2 passes × 5 tabs = **10 Vec allocations per frame**
- Each Vec contains `TitleText` with `Vec<FormatItem>`
- **600 allocations/second** → lots of memory copying

### Why We Didn't See This Before

**Lua overhead masked the memory overhead**:
- When Lua was 17%, memory ops were hidden in the noise
- Now that Lua is 13%, memory ops (14.33%) are visible
- **This was ALWAYS there**, we just couldn't see it!

### The Fundamental Problem

**WezTerm's tab bar rendering is fundamentally expensive**:

1. **Multiple data transformations**:
   - `TabInformation` → Lua tables → Lua callback → `FormatItem` → `Line`

2. **Lots of cloning**:
   - Clone tab/pane data for Lua
   - Clone Lua results back to Rust
   - Clone for rendering

3. **No caching of the final result**:
   - We cache Lua tables (Phase 3)
   - We cache Lua callback results (Phase 0)
   - **But we DON'T cache the final `TabBarState`!**

4. **Recomputation on every frame**:
   - Even if tabs don't change
   - Even if window size doesn't change
   - **Always recompute everything**

## Why It's Still Slow

### The Math Doesn't Add Up

**Total overhead in profile**: ~40%  
**Target frame budget at 60 FPS**: 16.67ms  
**Actual time spent on tab bar**: 40% of 16.67ms = **6.67ms**

**If each frame takes 6.67ms just for tab bar**, that's:
- **150 FPS max** if ONLY rendering tab bar
- But there's also:
  - Terminal rendering
  - Text rendering
  - Wayland compositing
  - Other UI

**Result**: Can't maintain 60 FPS!

### But Wait - Where's the Rest?

The profile shows:
- Memory: 20%
- Lua: 16%
- Other: 4%
- **Total: 40%**

**But we're only looking at the HOT functions!**

**Missing from the profile**:
- Terminal rendering: ???%
- Text rasterization: ???%
- GPU upload: ???%
- Wayland protocol: ???%
- Event handling: ???%

**Hypothesis**: Tab bar overhead (40%) is SO HIGH that it:
1. Dominates the profile
2. Pushes frame time over budget
3. Causes dropped frames
4. Feels slow

## The Real Solution: Cache the TabBarState

### Current Architecture (Expensive)

```rust
// EVERY FRAME at 60 FPS:
fn paint() {
    let tabs = get_tab_information();     // Create Vec<TabInformation>
    let panes = get_pane_information();   // Create Vec<PaneInformation>
    
    let tab_bar = TabBarState::new(       // Compute EVERYTHING
        width,
        mouse_pos,
        &tabs,      // Pass all data
        &panes,     // Pass all data
        colors,
        config,
        left_status,
        right_status,
    );  // ← 40% CPU spent here!
    
    render_tab_bar(&tab_bar);
}
```

**Cost**: 40% CPU **every frame**, even if nothing changed!

### Proposed Architecture (Efficient)

```rust
// Cache the tab bar state
struct TermWindow {
    cached_tab_bar: Option<CachedTabBar>,
    // ...
}

struct CachedTabBar {
    state: TabBarState,
    width: usize,
    tabs_hash: u64,          // Hash of tab state
    config_gen: usize,       // Config generation
    mouse_pos: Option<usize>,
}

fn paint() {
    let tabs = get_tab_information();
    let tabs_hash = calculate_hash(&tabs);
    
    // Check if we can reuse cached tab bar
    if let Some(cached) = &self.cached_tab_bar {
        if cached.width == width &&
           cached.tabs_hash == tabs_hash &&
           cached.config_gen == config.generation &&
           cached.mouse_pos == mouse_pos {
            // CACHE HIT - use cached tab bar (nearly free!)
            render_tab_bar(&cached.state);
            return;
        }
    }
    
    // CACHE MISS - recompute
    let tab_bar = TabBarState::new(...);  // 40% CPU
    
    // Store in cache
    self.cached_tab_bar = Some(CachedTabBar {
        state: tab_bar.clone(),
        width,
        tabs_hash,
        config_gen: config.generation,
        mouse_pos,
    });
    
    render_tab_bar(&tab_bar);
}
```

**Expected improvement**:
- **Cache hit rate during resize**: ~90% (tabs don't change)
- **CPU reduction**: 40% × 0.9 = **36% saved!**
- **Remaining cost**: 40% × 0.1 = **4%** (only on cache misses)

### Why This Works

**During resize (common case)**:
- Tabs don't change → hash matches
- Config doesn't change → gen matches
- Mouse might move, but we can tolerate stale hover
- **Result**: 36% CPU saved!

**During tab operations (rare case)**:
- Tab added/removed → hash changes → recompute (acceptable)
- Config changed → gen changes → recompute (acceptable)

## Recommended Implementation: Option D (Cache TabBarState)

### Changes Needed

**1. Add cache to TermWindow** (`termwindow/mod.rs`):
```rust
pub struct TermWindow {
    // ... existing fields ...
    
    // Tab bar caching
    cached_tab_bar: Option<CachedTabBar>,
}

struct CachedTabBar {
    state: TabBarState,
    width: usize,
    tabs_version: usize,     // Increment when tabs change
    config_gen: usize,
    left_status: String,
    right_status: String,
}
```

**2. Add version tracking** (`termwindow/mod.rs`):
```rust
impl TermWindow {
    // Track when tabs change
    fn on_tab_state_changed(&mut self) {
        self.tabs_version += 1;
        self.cached_tab_bar = None;  // Invalidate
    }
}
```

**3. Use cache in paint** (`termwindow/render/tab_bar.rs` or equivalent):
```rust
fn compute_tab_bar(&mut self, ...) -> TabBarState {
    // Check cache
    if let Some(cached) = &self.cached_tab_bar {
        if cached.width == width &&
           cached.tabs_version == self.tabs_version &&
           cached.config_gen == self.config.generation &&
           cached.left_status == left_status &&
           cached.right_status == right_status {
            log::trace!("Tab bar cache hit!");
            return cached.state.clone();  // Cheap clone
        }
    }
    
    // Cache miss - recompute
    log::trace!("Tab bar cache miss - recomputing");
    let state = TabBarState::new(...);
    
    // Store in cache
    self.cached_tab_bar = Some(CachedTabBar {
        state: state.clone(),
        width,
        tabs_version: self.tabs_version,
        config_gen: self.config.generation,
        left_status: left_status.to_string(),
        right_status: right_status.to_string(),
    });
    
    state
}
```

**4. Invalidate on changes**:
```rust
// In event handlers:
fn on_tab_added/removed/switched(...) {
    self.on_tab_state_changed();
}

fn on_config_reload(...) {
    self.cached_tab_bar = None;
}
```

### Expected Results

| Scenario | Cache Hit? | CPU Cost |
|----------|------------|----------|
| Resize window | ✅ Yes | ~4% (cache lookup + render) |
| Mouse move | ✅ Yes | ~4% (can ignore hover) |
| Switch tab | ❌ No | ~40% (recompute once) |
| Add/remove tab | ❌ No | ~40% (recompute once) |
| Config change | ❌ No | ~40% (recompute once) |

**During resize (most critical)**:
- Before: 40% CPU every frame
- After: 4% CPU every frame
- **Reduction: 36%**

**Overall performance**:
- Lua: 13%
- Memory (on cache miss): 20% → 2% (10% hit rate)
- Memory (on cache hit): ~0.5%
- **Total during resize: ~17%** (was 40%)
- **Net improvement: 23% CPU reduction**

**Should achieve smooth 60 FPS!**

## Alternative: Option E (Ignore Mouse Hover During Resize)

**Simpler hack** if we just want resize to be smooth:

```rust
fn compute_tab_bar(&mut self, ...) -> TabBarState {
    // During resize, ignore mouse position (no hover effects)
    let mouse_pos_for_cache = if self.is_resizing {
        None  // Ignore mouse during resize
    } else {
        mouse_pos
    };
    
    // Check cache (without mouse pos)
    if let Some(cached) = &self.cached_tab_bar {
        if cached.width == width && ... {
            return cached.state.clone();
        }
    }
    
    // Recompute with mouse_pos_for_cache
    let state = TabBarState::new(..., mouse_pos_for_cache, ...);
    // ...
}
```

**Benefits**:
- Much simpler (2-3 lines of code)
- Same 36% CPU reduction during resize
- Hover effects work normally (just not during resize)

**Drawbacks**:
- No hover effects while resizing
- Requires detecting "is resizing" state

## Comparison with Original Goals

### Original Target (from lua-change-proposal-2.md)

**Phase 0 target**: Cache Lua callbacks  
**Phase 0 result**: ✅ Achieved (tab/window title cached)

**Overall target**: Eliminate Lua overhead  
**Current state**: ⚠️ Lua reduced to 13% (was 20%), but **new bottleneck emerged**

### Why the Target Shifted

**Original assumption**: "Lua is the bottleneck"  
**Reality**: "Lua PLUS memory operations are the bottleneck"

**The optimization journey**:
1. Phase 0: Cached Lua computations → 5% saved
2. Phase 3: Cached Lua serialization → 5% saved
3. Phase 4: Removed duplicate calls → 4% saved
4. **Total Lua reduction: 14%** (20% → 6% direct, but 13% total with new sources)

**But**: This exposed the 14% memory overhead that was always there!

## Immediate Next Steps

### Quick Win: Cache TabBarState (Recommended)

**Effort**: 4-6 hours  
**Risk**: Low  
**Expected**: 36% CPU reduction during resize

**Implementation**:
1. Add `cached_tab_bar: Option<CachedTabBar>` to `TermWindow`
2. Add version tracking for tab changes
3. Check cache before `TabBarState::new()`
4. Invalidate on tab/config changes
5. Test and profile

**Verification**:
```bash
perf record -F 99 -g ./wezterm start
# Resize for 10 seconds
perf script > perf-report.6
```

**Expected in perf-report.6**:
- `__memmove_avx512`: 14.33% → ~1.5%
- `TabBarState::new`: Common → Rare
- Total overhead: ~40% → ~17%
- **Smooth 60 FPS resize!**

### Alternative: Ignore Hover During Resize (Hacky but Fast)

**Effort**: 30 minutes  
**Risk**: Very low  
**Expected**: 36% CPU reduction during resize

Simpler but less elegant. Good for quick testing.

## Conclusion

**Phase 4 optimizations worked as expected**:
- ✅ Removed `last_access` overhead: 1.20% saved
- ✅ Removed duplicate computation: ~3% saved
- ✅ Total Lua reduction: 4%

**But uncovered the real bottleneck**:
- ❌ Memory operations: 14.33% (memmove)
- ❌ Total overhead: 40% (too high for 60 FPS)

**Root cause**: **No caching of the final TabBarState**
- Recomputes everything on every frame
- Lots of cloning and memory allocation
- 90%+ of frames are wasted work (tabs don't change during resize)

**Solution**: **Cache the TabBarState**
- Check if tabs/config changed
- Reuse cached tab bar if unchanged
- **Expected: 36% CPU reduction during resize**
- **Should achieve smooth 60 FPS!**

**Next action**: Implement TabBarState caching (Option D).

