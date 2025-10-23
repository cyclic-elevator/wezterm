# Phase 4 Assessment: Why Optimization Didn't Work as Expected

## Date
2025-10-23

## Status

**Code deployed**: ✅ perf-report.4 shows `lua_ser_cache` functions are present  
**Performance**: ❌ Still slow - Lua overhead reduced from 16% → 10%, but not enough

## Key Finding: We Optimized the Wrong Layer (Again!)

### What We Fixed

✅ **Lua table serialization is now cached**
- `create_tab_info_table`: 0.01% (was ~3%)
- `create_pane_info_table`: 0.01% (was ~2%)
- **Serialization overhead eliminated**: ~5% saved

### What's Still Broken

❌ **Lua callbacks are still called every frame**
- `mlua::table::Table::raw_set`: **6.32%** (was 6.58%)
- `mlua::lua::Lua::create_string`: **5.78%** (was 6.01%)
- **Total Lua overhead**: **~10%** (was 16%)

### The Root Cause

**The Lua callback (`format-tab-title`) is STILL CALLED on every frame**, even though:
1. ✅ Input data (TabInformation) is cached as Lua tables
2. ✅ Tab title computation result is cached
3. ❌ But we pass cached Lua tables to the callback **every time**

**Why this is expensive**:
```rust
// In TabBarState::new, line 323-342:
let tab_titles: Vec<TitleText> = tab_info
    .iter()
    .map(|tab| compute_tab_title(tab, tab_info, pane_info, ...))  // Called 5x
    .collect();

// Then line 412-426:
for (tab_idx, tab_title) in tab_titles.iter().enumerate() {
    let tab_title = compute_tab_title(   // Called 5x AGAIN!
        &tab_info[tab_idx],
        tab_info,  // ← Cached Lua tables
        pane_info, // ← Cached Lua tables
        config,
        hover,
        tab_title_len,
    );
}
```

**For 5 tabs: 10 calls to `compute_tab_title` per frame × 60 FPS = 600 Lua callbacks/sec!**

### Evidence from perf-report.4

| Component | CPU % | Status |
|-----------|-------|--------|
| **Lua serialization (our fix)** | | |
| `create_tab_info_table` | 0.01% | ✅ **Fixed** (was ~3%) |
| `create_pane_info_table` | 0.01% | ✅ **Fixed** (was ~2%) |
| `get_or_create` (cache lookup) | 1.22% | ⚠️ **New overhead** |
| **Lua callback execution (not fixed)** | | |
| `mlua::table::Table::raw_set` | 6.32% | ❌ **Still high** |
| `mlua::lua::Lua::create_string` | 5.78% | ❌ **Still high** |
| `mlua::function::Function::call` | 3.02% | ❌ **Still high** |
| **Tab title caching** | | |
| `get_tab_title_cached` | 0.04% | ✅ **Working** |
| `TabTitleCache::insert` | 0.57% | ⚠️ **Cache misses!** |

### Why Lua Overhead Remains

**The cached Lua tables are passed to the callback, which:**
1. **Triggers Lua VM execution** (3.02% in `Function::call`)
2. **Creates NEW Lua tables** for the RESULT (6.32% in `raw_set`)
3. **Interns strings** from the result (5.78% in `create_string`)
4. **Triggers Lua GC** (1.53% in `luaC_step`)

**Caching the INPUT tables doesn't help if we still CREATE OUTPUT tables!**

### Why Cache Misses Happen

From perf data: `TabTitleCache::insert` is 0.57% - cache MISSES are happening!

**Likely causes**:
1. **Hover state changes** - cache key includes `hover`, which changes on mouse move
2. **Tab width changes** - cache key includes `tab_width_max`, which changes on resize
3. **Recomputation** - Line 419 recomputes with different parameters

**Result**: Even though we have caching, it's frequently invalidated.

### The Unnecessary Work

Looking at `TabBarState::new`:

**Lines 323-342**: Compute titles once
```rust
let tab_titles: Vec<TitleText> = tab_info
    .iter()
    .map(|tab| compute_tab_title(...))  // Call 1-5
    .collect();
```

**Lines 412-426**: **Recompute AGAIN**
```rust
for (tab_idx, tab_title) in tab_titles.iter().enumerate() {
    let tab_title = compute_tab_title(...);  // Call 6-10 (DUPLICATE!)
}
```

**Why?** "Recompute the title so that it factors in both the hover state and the adjusted maximum tab width"

**Problem**: This means cache misses on EVERY frame because hover/width changes!

### New Overhead Introduced

**`LuaTableCache::get_or_create`: 1.22%**
- Most of it (1.19%) is `clock_gettime` for `last_access` tracking
- We're calling `Instant::now()` on **every cache lookup**
- With 10 calls per frame × 60 FPS = 600 calls/sec
- **This is pure waste** - we don't even use `last_access`

## Why No Debug Logs

**Issue**: No debug logs were seen

**Possible causes**:
1. **Log level**: `RUST_LOG=debug` might not have been set
2. **Release build**: Debug logs are compiled out in release builds
3. **Log filtering**: The debug! macro might be filtered

**Evidence from code**:
```rust
log::debug!(
    "Lua serialization (cached): tabs={} in {:?}, panes={} in {:?}",
    ...
);
```

**In release builds**: `debug!` is a no-op unless explicitly enabled.

## Performance Analysis

### Expected vs. Actual

| Metric | Expected | Actual | Reason |
|--------|----------|--------|--------|
| Lua serialization | ~1% | ~1.3% | ✅ **Achieved** (but new overhead) |
| Lua callback overhead | ~2% | ~10% | ❌ **Still high** (callbacks still called) |
| Total Lua | ~2-3% | ~11% | ❌ **Not achieved** |

### Why We Didn't Achieve Expected Results

**Our assumption**: Lua overhead = serialization cost  
**Reality**: Lua overhead = serialization + callback execution + result creation + GC

**What we fixed**: Serialization (5% → 0.02%)  
**What's still broken**: Callback execution + result creation (still 10%)

### The Real Bottleneck

**Lua callbacks are called TOO OFTEN**:
- 10 calls per frame (5 tabs × 2 computations)
- 60 FPS = **600 callbacks/second**
- Each callback:
  1. Executes Lua code (3%)
  2. Creates result tables (6%)
  3. Interns strings (6%)
  4. Triggers GC (1.5%)

**Solution**: Don't call the callback if the result is cached!

## Root Cause Summary

### Problem 1: Cache Bypass

**In `TabBarState::new`, line 419**:
```rust
let tab_title = compute_tab_title(
    &tab_info[tab_idx],
    tab_info,
    pane_info,
    config,
    hover,       // ← Changes on mouse move
    tab_title_len,  // ← Changes on resize
);
```

**This bypasses the cache** because:
- Cache key changes on hover
- Cache key changes on tab_title_len
- **Result**: Cache miss on every frame during resize/hover

### Problem 2: Unnecessary Recomputation

**Why recompute?** Comment says:
> "Recompute the title so that it factors in both the hover state and the adjusted maximum tab width"

**But**: The first computation (line 330) already has the title!

**This is architectural waste**: Computing twice when once is sufficient.

### Problem 3: Expensive Cache Lookups

**`Instant::now()` in `get_or_create`**:
```rust
if let Some(entry) = self.entries.get_mut(&id) {
    entry.last_access = Instant::now();  // ← 1.19% CPU!
    return lua.registry_value(&entry.registry_key);
}
```

**Cost**: 1.22% for cache lookups, mostly from `clock_gettime` syscall

## The Real Solution

### Option A: Don't Recompute Tab Titles (Fastest)

**Change `TabBarState::new`** to NOT recompute:

```rust
let tab_titles: Vec<TitleText> = tab_info
    .iter()
    .map(|tab| {
        let hover = is_tab_hover(mouse_x, current_x, ...);
        compute_tab_title(tab, tab_info, pane_info, config, hover, tab_max_width)
    })
    .collect();

// Then in the loop:
for (tab_idx, tab_title) in tab_titles.iter().enumerate() {
    // USE the cached tab_title, don't recompute!
    let tab_title_line = render_tab_title(&tab_title, cell_attrs);
}
```

**Expected improvement**: 50% reduction in Lua calls → 5% CPU reduction

### Option B: Remove `last_access` Tracking

**In `lua_ser_cache.rs`**:

Remove `last_access` field and `Instant::now()` calls:

```rust
struct CacheEntry {
    registry_key: LuaRegistryKey,
    generation: usize,
    // Remove: last_access: Instant,
}

pub fn get_or_create<'lua, F>(...) {
    if let Some(entry) = self.entries.get_mut(&id) {
        // Remove: entry.last_access = Instant::now();
        return lua.registry_value(&entry.registry_key);
    }
    ...
}
```

**Expected improvement**: 1.2% CPU reduction

### Option C: Smarter Cache Key

**Change cache key** to NOT include hover/width:

```rust
// Current:
TabCacheKey {
    tab_id,
    hover,  // ← Remove this
    tab_max_width,  // ← Remove this
    ...
}

// New:
TabCacheKey {
    tab_id,
    // Base computation only
    ...
}

// Then apply hover/width styling in Rust, not Lua
```

**Expected improvement**: Higher cache hit rate → 3-5% CPU reduction

### Option D: Batch Lua Calls

**Call Lua once** for all tabs, not per-tab:

```rust
// Instead of:
for tab in tabs {
    let title = call_lua_format_tab_title(tab);  // 5 calls
}

// Do:
let all_titles = call_lua_format_all_tab_titles(tabs);  // 1 call
```

**Expected improvement**: Reduce Lua VM overhead by 80% → 8% CPU reduction

## Recommended Action Plan

### Quick Win 1: Remove `last_access` Tracking (30 minutes)

**File**: `wezterm-gui/src/lua_ser_cache.rs`

Remove `last_access` field and all `Instant::now()` calls.

**Expected**: 1.2% CPU reduction  
**Risk**: Very low  
**Effort**: Minimal

### Quick Win 2: Don't Recompute Tab Titles (1-2 hours)

**File**: `wezterm-gui/src/tabbar.rs`

Change `TabBarState::new` to compute titles once with hover state, not twice.

**Expected**: 5% CPU reduction  
**Risk**: Low (might affect hover rendering)  
**Effort**: Low

### Medium Win: Fix Tab Title Cache Key (2-4 hours)

**File**: `wezterm-gui/src/tab_title_cache.rs`

Change cache key to not include `hover` or `tab_max_width`. Apply styling in Rust.

**Expected**: 3-5% CPU reduction  
**Risk**: Medium (changes rendering logic)  
**Effort**: Medium

### Long-term: Remove Lua from Hot Path (1-2 weeks)

Render tab bar in pure Rust, only call Lua for custom user formatting.

**Expected**: Eliminate remaining 10% Lua overhead  
**Risk**: High (breaking change)  
**Effort**: High

## Immediate Next Steps

### Step 1: Quick Fix - Remove `last_access` (Do Now)

1. Edit `lua_ser_cache.rs`
2. Remove `last_access` field
3. Remove `Instant::now()` calls
4. Rebuild and profile

**Expected**: perf-report.5 shows `get_or_create` reduced from 1.22% → 0.03%

### Step 2: Quick Fix - Don't Recompute (Do Now)

1. Edit `tabbar.rs`, line 412-426
2. Remove duplicate `compute_tab_title` call
3. Use the already-computed `tab_title` from line 323
4. Rebuild and profile

**Expected**: Lua overhead reduced from 10% → 5%

### Step 3: Profile Again

After both quick fixes:

```bash
perf record -F 99 -g ./wezterm start
# Resize for 10 seconds
perf script > chats/perf-report.5
```

**Expected results**:
- `get_or_create`: 1.22% → 0.03%
- `mlua::*` functions: 10% → 5%
- Total improvement: ~6% CPU reduction
- Should achieve smooth 60 FPS

## Conclusion

**Phase 3 optimization worked, but only partially**:
- ✅ Lua table serialization: Fixed (5% → 0.02%)
- ✅ Cache infrastructure: Working correctly
- ❌ Lua callback execution: Still expensive (10%)
- ❌ Cache hit rate: Poor (frequent misses)
- ❌ New overhead: `last_access` tracking (1.2%)

**Root cause**: We cached the input, but the Lua callbacks are still called on every frame, creating expensive output.

**Solution**: 
1. Remove `last_access` tracking (1.2% saved)
2. Don't recompute tab titles (5% saved)
3. **Total**: ~6% CPU reduction → should achieve smooth 60 FPS

**Next**: Implement two quick fixes and profile again.

