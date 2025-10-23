//! Lua Serialization Cache Module
//!
//! Caches the conversion of Rust structs (TabInformation, PaneInformation)
//! to Lua tables to avoid expensive re-serialization on every frame.
//!
//! The bottleneck is not the Lua callback execution (which is cached separately),
//! but the serialization of input data structures to Lua tables.

use crate::termwindow::{PaneInformation, TabInformation};
use mlua::prelude::*;
use mux::tab::TabId;
use mux::pane::PaneId;
use std::collections::HashMap;
use std::sync::Mutex;

lazy_static::lazy_static! {
    /// Cache for TabInformation Lua tables
    static ref TAB_LUA_CACHE: Mutex<LuaTableCache<TabId, TabInformation>> =
        Mutex::new(LuaTableCache::new("tabs"));
    
    /// Cache for PaneInformation Lua tables  
    static ref PANE_LUA_CACHE: Mutex<LuaTableCache<PaneId, PaneInformation>> =
        Mutex::new(LuaTableCache::new("panes"));
}

/// Cache entry storing a Lua registry key and metadata
struct CacheEntry {
    /// Lua registry key for the cached table
    registry_key: LuaRegistryKey,
    /// Generation number for bulk invalidation
    generation: usize,
}

/// Generic cache for Rust → Lua table conversions
pub struct LuaTableCache<K, T>
where
    K: std::hash::Hash + Eq + Clone,
    T: Clone,
{
    /// Cache entries by ID
    entries: HashMap<K, CacheEntry>,
    /// Current generation number
    generation: usize,
    /// Name for debugging
    name: &'static str,
    /// Phantom data to satisfy type parameter
    _phantom: std::marker::PhantomData<T>,
}

impl<K, T> LuaTableCache<K, T>
where
    K: std::hash::Hash + Eq + Clone,
    T: Clone,
{
    pub fn new(name: &'static str) -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
            name,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Get cached Lua table or create a new one
    /// Note: The cache is invalidated by generation number, not by data comparison
    pub fn get_or_create<'lua, F>(
        &mut self,
        lua: &'lua Lua,
        id: K,
        data: &T,
        create_fn: F,
    ) -> LuaResult<LuaTable<'lua>>
    where
        F: FnOnce(&'lua Lua, &T) -> LuaResult<LuaTable<'lua>>,
    {
        // Check if we have a cached entry with current generation
        if let Some(entry) = self.entries.get(&id) {
            if entry.generation == self.generation {
                // Cache hit! Return the cached table
                return lua.registry_value(&entry.registry_key);
            }
        }

        // Cache miss or stale - create new Lua table
        let table = create_fn(lua, data)?;
        let registry_key = lua.create_registry_value(table.clone())?;

        // Store in cache
        self.entries.insert(
            id.clone(),
            CacheEntry {
                registry_key,
                generation: self.generation,
            },
        );

        log::trace!("{} cache: created table", self.name);

        Ok(table)
    }

    /// Invalidate all cached entries (increments generation)
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        log::debug!("{} cache: invalidated (gen {})", self.name, self.generation);
    }

    /// Clear all entries (for memory cleanup)
    pub fn clear(&mut self) {
        self.entries.clear();
        log::debug!("{} cache: cleared", self.name);
    }

    /// Remove old entries from previous generations
    /// This is called during invalidation to free memory
    pub fn cleanup_old_generations(&mut self) {
        let current_gen = self.generation;
        let before_count = self.entries.len();
        
        self.entries.retain(|_, entry| {
            entry.generation == current_gen
        });
        
        let removed = before_count - self.entries.len();
        if removed > 0 {
            log::debug!("{} cache: removed {} old generation entries", self.name, removed);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Create a Lua table from TabInformation
fn create_tab_info_table<'lua>(
    lua: &'lua Lua,
    tab: &TabInformation,
) -> LuaResult<LuaTable<'lua>> {
    // Create table manually - simpler and faster than going through serialization
    let table = lua.create_table()?;
    
    table.set("tab_id", tab.tab_id)?;
    table.set("tab_index", tab.tab_index)?;
    table.set("is_active", tab.is_active)?;
    table.set("is_last_active", tab.is_last_active)?;
    table.set("tab_title", tab.tab_title.clone())?;
    
    // active_pane is Option<PaneInformation>
    if let Some(ref pane) = tab.active_pane {
        table.set("active_pane", create_pane_info_table(lua, pane)?)?;
    }
    
    table.set("window_id", tab.window_id)?;
    
    Ok(table)
}

/// Create a Lua table from PaneInformation (nested within TabInformation)
fn create_pane_info_table<'lua>(
    lua: &'lua Lua,
    pane: &PaneInformation,
) -> LuaResult<LuaTable<'lua>> {
    let table = lua.create_table()?;
    
    table.set("pane_id", pane.pane_id)?;
    table.set("pane_index", pane.pane_index)?;
    table.set("is_active", pane.is_active)?;
    table.set("is_zoomed", pane.is_zoomed)?;
    table.set("left", pane.left)?;
    table.set("top", pane.top)?;
    table.set("width", pane.width)?;
    table.set("height", pane.height)?;
    table.set("pixel_width", pane.pixel_width)?;
    table.set("pixel_height", pane.pixel_height)?;
    table.set("title", pane.title.clone())?;
    table.set("user_vars", lua.create_table()?)?; // Simplified
    
    Ok(table)
}

/// Get or create cached Lua tables for a slice of TabInformation
pub fn get_tabs_as_lua_sequence<'lua>(
    lua: &'lua Lua,
    tabs: &[TabInformation],
) -> LuaResult<LuaValue<'lua>> {
    let mut cache = TAB_LUA_CACHE.lock().unwrap();
    
    let sequence = lua.create_sequence_from(
        tabs.iter().map(|tab| {
            cache.get_or_create(lua, tab.tab_id, tab, create_tab_info_table)
        }).collect::<LuaResult<Vec<_>>>()?
    )?;
    
    Ok(LuaValue::Table(sequence))
}

/// Get or create cached Lua tables for a slice of PaneInformation
pub fn get_panes_as_lua_sequence<'lua>(
    lua: &'lua Lua,
    panes: &[PaneInformation],
) -> LuaResult<LuaValue<'lua>> {
    let mut cache = PANE_LUA_CACHE.lock().unwrap();
    
    let sequence = lua.create_sequence_from(
        panes.iter().map(|pane| {
            cache.get_or_create(lua, pane.pane_id, pane, create_pane_info_table)
        }).collect::<LuaResult<Vec<_>>>()?
    )?;
    
    Ok(LuaValue::Table(sequence))
}

/// Invalidate tab caches (call when tabs change)
pub fn invalidate_tab_cache() {
    let mut cache = TAB_LUA_CACHE.lock().unwrap();
    cache.invalidate();
}

/// Invalidate pane caches (call when panes change)
pub fn invalidate_pane_cache() {
    let mut cache = PANE_LUA_CACHE.lock().unwrap();
    cache.invalidate();
}

/// Invalidate all Lua serialization caches
pub fn invalidate_all_lua_caches() {
    invalidate_tab_cache();
    invalidate_pane_cache();
    log::debug!("All Lua serialization caches invalidated");
}

/// Cleanup old cache entries from previous generations
/// This is automatically called during invalidation, but can be called manually
pub fn cleanup_lua_caches() {
    {
        let mut cache = TAB_LUA_CACHE.lock().unwrap();
        cache.cleanup_old_generations();
    }
    
    {
        let mut cache = PANE_LUA_CACHE.lock().unwrap();
        cache.cleanup_old_generations();
    }
}

/// Get cache statistics for debugging
pub fn get_cache_stats() -> (usize, usize) {
    let tab_count = TAB_LUA_CACHE.lock().unwrap().len();
    let pane_count = PANE_LUA_CACHE.lock().unwrap().len();
    (tab_count, pane_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic() {
        let cache: LuaTableCache<u32, String> = LuaTableCache::new("test");
        
        assert_eq!(cache.len(), 0);
        
        // Cache starts empty
        assert!(cache.entries.is_empty());
        assert_eq!(cache.generation, 0);
    }

    #[test]
    fn test_cache_invalidate() {
        let mut cache: LuaTableCache<u32, String> = LuaTableCache::new("test");
        
        // Invalidate increments generation
        cache.invalidate();
        assert_eq!(cache.generation, 1);
        
        cache.invalidate();
        assert_eq!(cache.generation, 2);
    }

    #[test]
    fn test_cache_cleanup() {
        let mut cache: LuaTableCache<u32, String> = LuaTableCache::new("test");
        
        // Cleanup with empty cache should be safe
        cache.cleanup_old_generations();
        assert_eq!(cache.len(), 0);
    }
}

