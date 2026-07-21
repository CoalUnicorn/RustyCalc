//! Memo of `ctx.measure_text` results, keyed `(font_css, text)`.
//!
//! Every measure is a JS-boundary round trip, and a sheet remeasures the
//! same visible text on every fresh paint. Canvas2D-only, like
//! `SetterCache` — the recorder/SVG/PDF backends measure in pure Rust.
//! Overflow policy is a full clear (glide-data-grid's proven approach):
//! the working set refills within one frame and bookkeeping stays trivial.

use std::collections::HashMap;

/// Total entries across all fonts before the next insert wipes the map.
/// 10k cell-text strings is well under a megabyte of heap.
pub(crate) const MEASURE_CACHE_CAP: usize = 10_000;

#[derive(Default)]
pub(crate) struct MeasureCache {
    by_font: HashMap<String, HashMap<String, f64>>,
    entries: usize,
}

impl MeasureCache {
    pub(crate) fn get(&self, font_css: &str, text: &str) -> Option<f64> {
        self.by_font.get(font_css)?.get(text).copied()
    }

    pub(crate) fn insert(&mut self, font_css: &str, text: &str, width: f64) {
        if self.entries >= MEASURE_CACHE_CAP {
            self.clear();
        }
        // Owned-key allocation happens only on the miss path; the font
        // level dedups the css string across every text in that font.
        let per_font = self.by_font.entry(font_css.to_owned()).or_default();
        if per_font.insert(text.to_owned(), width).is_none() {
            self.entries += 1;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.by_font.clear();
        self.entries = 0;
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miss_then_hit() {
        let mut c = MeasureCache::default();
        assert_eq!(c.get("12px Inter", "abc"), None);
        c.insert("12px Inter", "abc", 42.5);
        assert_eq!(c.get("12px Inter", "abc"), Some(42.5));
    }

    #[test]
    fn fonts_do_not_collide() {
        let mut c = MeasureCache::default();
        c.insert("12px Inter", "abc", 10.0);
        c.insert("16px Inter", "abc", 20.0);
        assert_eq!(c.get("12px Inter", "abc"), Some(10.0));
        assert_eq!(c.get("16px Inter", "abc"), Some(20.0));
    }

    #[test]
    fn reinsert_same_key_updates_without_growing() {
        let mut c = MeasureCache::default();
        c.insert("12px Inter", "abc", 10.0);
        c.insert("12px Inter", "abc", 11.0);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("12px Inter", "abc"), Some(11.0));
    }

    #[test]
    fn cap_triggers_full_clear_then_accepts_new_entry() {
        let mut c = MeasureCache::default();
        for i in 0..MEASURE_CACHE_CAP {
            c.insert("12px Inter", &i.to_string(), i as f64);
        }
        assert_eq!(c.len(), MEASURE_CACHE_CAP);
        c.insert("12px Inter", "overflow", 1.0);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("12px Inter", "overflow"), Some(1.0));
        assert_eq!(c.get("12px Inter", "0"), None);
    }

    #[test]
    fn clear_empties() {
        let mut c = MeasureCache::default();
        c.insert("12px Inter", "abc", 10.0);
        c.clear();
        assert_eq!(c.len(), 0);
        assert_eq!(c.get("12px Inter", "abc"), None);
    }
}
