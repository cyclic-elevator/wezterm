//! Callback Cache Module
//!
//! Provides caching for expensive Lua callbacks to reduce FFI overhead during
//! high-frequency events like window resizing.
//!
//! This module implements a generation-based cache that stores the results of
//! Lua callback invocations. When the same input state is encountered, the
//! cached result is returned instantly without invoking Lua.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

lazy_static::lazy_static! {
    static ref WINDOW_TITLE_CACHE: Mutex<CallbackCache<String>> =
        Mutex::new(CallbackCache::new());
    
    static ref STATUS_CACHE: Mutex<CallbackCache<Vec<StatusItem>>> =
        Mutex::new(CallbackCache::new());
}

/// Represents a status item (simplified for caching purposes)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StatusItem {
    pub text: String,
    // Add other fields as needed
}

/// Cache key for window title computation
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct WindowTitleKey {
    active_tab_id: Option<usize>,
    active_pane_id: Option<usize>,
    active_tab_title: String,
    active_pane_title: String,
    num_tabs: usize,
    is_zoomed: bool,
}

impl WindowTitleKey {
    pub fn new(
        active_tab_id: Option<usize>,
        active_pane_id: Option<usize>,
        active_tab_title: String,
        active_pane_title: String,
        num_tabs: usize,
        is_zoomed: bool,
    ) -> Self {
        Self {
            active_tab_id,
            active_pane_id,
            active_tab_title,
            active_pane_title,
            num_tabs,
            is_zoomed,
        }
    }
}

/// Cache key for status updates
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct StatusKey {
    active_pane_id: Option<usize>,
    active_pane_title: String,
    // Add a timestamp bucket to allow periodic updates
    // (rounded to 200ms intervals for status updates)
    time_bucket: u64,
}

impl StatusKey {
    pub fn new(active_pane_id: Option<usize>, active_pane_title: String) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        
        // Round to 200ms buckets to allow status to update periodically
        // even if other state hasn't changed
        let time_bucket = (now / 200) as u64;
        
        Self {
            active_pane_id,
            active_pane_title,
            time_bucket,
        }
    }
}

/// Generic cache entry
struct CacheEntry<T> {
    value: T,
    generation: usize,
}

/// Generic callback cache with generation-based invalidation
pub struct CallbackCache<T> {
    entries: HashMap<u64, CacheEntry<T>>,
    generation: usize,
}

impl<T: Clone> CallbackCache<T> {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
        }
    }

    pub fn get<K: Hash>(&self, key: &K) -> Option<T> {
        let hash = Self::hash_key(key);
        self.entries.get(&hash).and_then(|entry| {
            if entry.generation == self.generation {
                Some(entry.value.clone())
            } else {
                None
            }
        })
    }

    pub fn insert<K: Hash>(&mut self, key: K, value: T) {
        let hash = Self::hash_key(&key);
        self.entries.insert(
            hash,
            CacheEntry {
                value,
                generation: self.generation,
            },
        );
    }

    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        
        // Clean up old entries (keep only current generation)
        let current_generation = self.generation;
        self.entries
            .retain(|_, entry| entry.generation == current_generation);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn hash_key<K: Hash>(key: &K) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }
}

/// Get window title with caching
pub fn get_window_title_cached<F>(key: WindowTitleKey, compute_fn: F) -> String
where
    F: FnOnce() -> Option<String>,
{
    // Check cache first
    {
        let cache = WINDOW_TITLE_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(&key) {
            return cached; // Cache hit
        }
    }

    // Cache miss - compute now
    let result = compute_fn();

    match result {
        Some(title) => {
            // Cache the result
            let mut cache = WINDOW_TITLE_CACHE.lock().unwrap();
            cache.insert(key, title.clone());
            title
        }
        None => {
            // Lua returned None - generate default (don't cache)
            generate_default_window_title(&key)
        }
    }
}

/// Get status with caching
pub fn get_status_cached<F>(key: StatusKey, compute_fn: F) -> Vec<StatusItem>
where
    F: FnOnce() -> Option<Vec<StatusItem>>,
{
    // Check cache first
    {
        let cache = STATUS_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(&key) {
            return cached; // Cache hit
        }
    }

    // Cache miss - compute now
    let result = compute_fn();

    match result {
        Some(status) => {
            // Cache the result
            let mut cache = STATUS_CACHE.lock().unwrap();
            cache.insert(key, status.clone());
            status
        }
        None => {
            // Lua returned None - return empty status
            Vec::new()
        }
    }
}

/// Invalidate window title cache
pub fn invalidate_window_title_cache() {
    let mut cache = WINDOW_TITLE_CACHE.lock().unwrap();
    cache.invalidate();
}

/// Invalidate status cache
pub fn invalidate_status_cache() {
    let mut cache = STATUS_CACHE.lock().unwrap();
    cache.invalidate();
}

/// Invalidate all caches
pub fn invalidate_all_caches() {
    invalidate_window_title_cache();
    invalidate_status_cache();
    // Also invalidate tab title cache
    crate::tab_title_cache::invalidate_tab_title_cache();
}

/// Generate a sensible default window title
fn generate_default_window_title(key: &WindowTitleKey) -> String {
    if key.num_tabs == 1 {
        format!(
            "{}{}",
            if key.is_zoomed { "[Z] " } else { "" },
            key.active_pane_title
        )
    } else {
        format!(
            "{}[{}/{}] {}",
            if key.is_zoomed { "[Z] " } else { "" },
            key.active_tab_id.map(|id| id + 1).unwrap_or(1),
            key.num_tabs,
            key.active_pane_title
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_title_cache() {
        let mut cache = CallbackCache::<String>::new();
        
        let key = WindowTitleKey::new(
            Some(0),
            Some(0),
            "Tab 1".to_string(),
            "Pane Title".to_string(),
            1,
            false,
        );
        
        // Cache miss
        assert!(cache.get(&key).is_none());
        
        // Insert
        cache.insert(key.clone(), "Test Title".to_string());
        
        // Cache hit
        assert_eq!(cache.get(&key), Some("Test Title".to_string()));
        
        // Invalidate
        cache.invalidate();
        
        // Cache miss after invalidation
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn test_status_cache() {
        let mut cache = CallbackCache::<Vec<StatusItem>>::new();
        
        let key = StatusKey::new(Some(0), "Pane Title".to_string());
        
        let status = vec![StatusItem {
            text: "Test Status".to_string(),
        }];
        
        // Cache miss
        assert!(cache.get(&key).is_none());
        
        // Insert
        cache.insert(key.clone(), status.clone());
        
        // Cache hit
        assert_eq!(cache.get(&key), Some(status));
    }

    #[test]
    fn test_generation_based_invalidation() {
        let mut cache = CallbackCache::<String>::new();
        
        let key1 = WindowTitleKey::new(
            Some(0),
            Some(0),
            "Tab 1".to_string(),
            "Pane 1".to_string(),
            1,
            false,
        );
        let key2 = WindowTitleKey::new(
            Some(1),
            Some(1),
            "Tab 2".to_string(),
            "Pane 2".to_string(),
            2,
            false,
        );
        
        cache.insert(key1.clone(), "Title 1".to_string());
        cache.insert(key2.clone(), "Title 2".to_string());
        
        assert_eq!(cache.entries.len(), 2);
        
        // Invalidate should clean up old entries
        cache.invalidate();
        
        // All old entries should be gone
        assert_eq!(cache.entries.len(), 0);
        
        // Add new entry after invalidation
        cache.insert(key1.clone(), "New Title 1".to_string());
        assert_eq!(cache.get(&key1), Some("New Title 1".to_string()));
    }

    #[test]
    fn test_default_window_title_generation() {
        let key = WindowTitleKey::new(
            Some(0),
            Some(0),
            "Tab 1".to_string(),
            "bash".to_string(),
            1,
            false,
        );
        
        let title = generate_default_window_title(&key);
        assert_eq!(title, "bash");
        
        let key_multi = WindowTitleKey::new(
            Some(1),
            Some(0),
            "Tab 2".to_string(),
            "vim".to_string(),
            3,
            true,
        );
        
        let title_multi = generate_default_window_title(&key_multi);
        assert_eq!(title_multi, "[Z] [2/3] vim");
    }
}

