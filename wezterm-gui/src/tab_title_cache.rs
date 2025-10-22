use crate::tabbar::TitleText;
use crate::termwindow::TabInformation;
use finl_unicode::grapheme_clusters::Graphemes;
use mux::tab::TabId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use termwiz::cell::unicode_column_width;
use termwiz::color::AnsiColor;
use termwiz_funcs::{FormatColor, FormatItem};
use wezterm_term::Progress;

lazy_static::lazy_static! {
    static ref TAB_TITLE_CACHE: Arc<Mutex<TabTitleCache>> =
        Arc::new(Mutex::new(TabTitleCache::new()));
}

/// Simplified representation of Progress for hashing
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
enum ProgressState {
    None,
    Percentage(u8),
    Error(u8),
    Indeterminate,
}

impl From<&Progress> for ProgressState {
    fn from(progress: &Progress) -> Self {
        match progress {
            Progress::None => ProgressState::None,
            Progress::Percentage(p) => ProgressState::Percentage(*p),
            Progress::Error(p) => ProgressState::Error(*p),
            Progress::Indeterminate => ProgressState::Indeterminate,
        }
    }
}

/// Cache key that uniquely identifies a tab title computation
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct TabCacheKey {
    tab_id: TabId,
    title: String,
    is_active: bool,
    hover: bool,
    // Include active pane info that affects rendering
    active_pane_title: Option<String>,
    active_pane_has_unseen_output: Option<bool>,
    active_pane_progress: Option<ProgressState>,
}

impl TabCacheKey {
    pub fn from_tab_info(tab: &TabInformation, hover: bool) -> Self {
        let (active_pane_title, active_pane_has_unseen_output, active_pane_progress) =
            if let Some(pane) = &tab.active_pane {
                (
                    Some(pane.title.clone()),
                    Some(pane.has_unseen_output),
                    Some(ProgressState::from(&pane.progress)),
                )
            } else {
                (None, None, None)
            };

        Self {
            tab_id: tab.tab_id,
            title: tab.tab_title.clone(),
            is_active: tab.is_active,
            hover,
            active_pane_title,
            active_pane_has_unseen_output,
            active_pane_progress,
        }
    }
}

struct CachedTitleEntry {
    title: TitleText,
    generation: usize,
    computed_at: Instant,
}

pub struct TabTitleCache {
    entries: HashMap<TabCacheKey, CachedTitleEntry>,
    generation: usize,
}

impl TabTitleCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
        }
    }

    pub fn get(&self, key: &TabCacheKey) -> Option<TitleText> {
        self.entries.get(key).and_then(|entry| {
            // Only return if generation matches (cache still valid)
            if entry.generation == self.generation {
                Some(entry.title.clone())
            } else {
                None
            }
        })
    }

    pub fn insert(&mut self, key: TabCacheKey, title: TitleText) {
        self.entries.insert(
            key,
            CachedTitleEntry {
                title,
                generation: self.generation,
                computed_at: Instant::now(),
            },
        );
    }

    pub fn invalidate(&mut self) {
        self.generation += 1;
        // Optionally clear old entries to prevent unbounded growth
        let current_generation = self.generation;
        self.entries.retain(|_, entry| entry.generation == current_generation);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.generation += 1;
    }
}

/// Generate a sensible default title when Lua fails or times out
pub fn generate_default_title(tab: &TabInformation, max_width: usize) -> TitleText {
    let mut items = vec![];
    let mut len = 0;

    // Determine the title text
    let title_text = if let Some(pane) = &tab.active_pane {
        if tab.tab_title.is_empty() {
            pane.title.clone()
        } else {
            tab.tab_title.clone()
        }
    } else {
        if tab.tab_title.is_empty() {
            format!("Tab {}", tab.tab_index + 1)
        } else {
            tab.tab_title.clone()
        }
    };

    // Add progress indicator if present
    if let Some(pane) = &tab.active_pane {
        match pane.progress {
            Progress::None => {}
            Progress::Percentage(pct) | Progress::Error(pct) => {
                let graphic = format!("{} ", pct_to_glyph(pct));
                len += unicode_column_width(&graphic, None);
                let color = if matches!(pane.progress, Progress::Percentage(_)) {
                    FormatItem::Foreground(FormatColor::AnsiColor(AnsiColor::Green))
                } else {
                    FormatItem::Foreground(FormatColor::AnsiColor(AnsiColor::Red))
                };
                items.push(color);
                items.push(FormatItem::Text(graphic));
                items.push(FormatItem::Foreground(FormatColor::Default));
            }
            Progress::Indeterminate => {}
        }
    }

    // Truncate if needed
    let display_title = if unicode_column_width(&title_text, None) > max_width {
        let mut truncated = String::new();
        let mut current_width = 0;
        let max_content_width = max_width.saturating_sub(1); // Reserve space for ellipsis
        
        for grapheme in Graphemes::new(&title_text) {
            let grapheme_width = unicode_column_width(grapheme, None);
            if current_width + grapheme_width > max_content_width {
                break;
            }
            truncated.push_str(grapheme);
            current_width += grapheme_width;
        }
        format!("{}…", truncated)
    } else {
        title_text
    };

    len += unicode_column_width(&display_title, None);
    items.push(FormatItem::Text(display_title));

    TitleText { items, len }
}

/// Helper function to map progress percentage to a glyph
fn pct_to_glyph(pct: u8) -> char {
    match pct {
        0..=5 => '\u{f0130}',    // empty circle
        6..=18 => '\u{f0a9e}',   // centered at 12
        19..=31 => '\u{f0a9f}',  // centered at 25
        32..=43 => '\u{f0aa0}',  // centered at 37.5
        44..=56 => '\u{f0aa1}',  // half-filled circle
        57..=68 => '\u{f0aa2}',  // centered at 62.5
        69..=81 => '\u{f0aa3}',  // centered at 75
        82..=94 => '\u{f0aa4}',  // centered at 88
        95..=100 => '\u{f0aa5}', // filled circle
        _ => '\u{f0aa5}',
    }
}

/// Main API: Get tab title with caching
/// Returns cached title instantly on cache hit, otherwise computes and caches
pub fn get_tab_title_cached<F>(
    tab: &TabInformation,
    hover: bool,
    compute_fn: F,
) -> TitleText
where
    F: FnOnce() -> Option<TitleText>,
{
    let key = TabCacheKey::from_tab_info(tab, hover);

    // 1. Check cache first
    {
        let cache = TAB_TITLE_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(&key) {
            return cached; // ✅ Cache hit - instant return
        }
    }

    // 2. Cache miss - compute now (must stay on main thread for Lua access)
    let result = compute_fn();

    match result {
        Some(title) => {
            // Success - cache it
            let mut cache = TAB_TITLE_CACHE.lock().unwrap();
            cache.insert(key, title.clone());
            title
        }
        None => {
            // Lua returned None or error - use default
            let max_width = 100; // reasonable default
            let default_title = generate_default_title(tab, max_width);
            // Don't cache the default to allow retry on next hover
            default_title
        }
    }
}

/// Invalidate the cache (call when tabs or config change)
pub fn invalidate_tab_title_cache() {
    let mut cache = TAB_TITLE_CACHE.lock().unwrap();
    cache.invalidate();
}

/// Clear the entire cache
pub fn clear_tab_title_cache() {
    let mut cache = TAB_TITLE_CACHE.lock().unwrap();
    cache.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_hit() {
        let mut cache = TabTitleCache::new();
        let key = TabCacheKey {
            tab_id: 1,
            title: "test".to_string(),
            is_active: true,
            hover: false,
            active_pane_title: None,
            active_pane_has_unseen_output: None,
            active_pane_progress: None,
        };
        let title = TitleText {
            items: vec![FormatItem::Text("test".to_string())],
            len: 4,
        };

        cache.insert(key.clone(), title.clone());
        let retrieved = cache.get(&key);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().len, 4);
    }

    #[test]
    fn test_cache_invalidation() {
        let mut cache = TabTitleCache::new();
        let key = TabCacheKey {
            tab_id: 1,
            title: "test".to_string(),
            is_active: true,
            hover: false,
            active_pane_title: None,
            active_pane_has_unseen_output: None,
            active_pane_progress: None,
        };
        let title = TitleText {
            items: vec![FormatItem::Text("test".to_string())],
            len: 4,
        };

        cache.insert(key.clone(), title);
        cache.invalidate();
        
        // After invalidation, the old entry should not be returned
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn test_cache_fallback() {
        // Test that when Lua returns None, we get a default title
        // This simulates what happens when there's no Lua callback configured
        let tab = TabInformation {
            tab_id: 1,
            tab_index: 0,
            is_active: true,
            is_last_active: false,
            active_pane: None,
            window_id: 0,
            tab_title: "Test Tab".to_string(),
        };
        
        let result = get_tab_title_cached(&tab, false, || None);
        
        // Should get a default title
        assert!(result.len > 0);
        assert_eq!(result.items.len(), 1);
    }
}

