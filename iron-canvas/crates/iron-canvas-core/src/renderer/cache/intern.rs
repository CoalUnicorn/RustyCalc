//! Renderer-lifetime intern tables: dedup CSS strings into `Rc<str>` so
//! the per-cell hot path is `Rc::clone` instead of `String::clone` (or
//! `format!`) for repeated colors / fonts / column labels.
//!
//! Two tables, two keying strategies:
//! - [`FontIntern`] — composite key `(size, bold, italic, family)`, linear
//!   scan; bounded by ~10 unique tuples per realistic sheet.
//! - [`ColorIntern`] — `ColorKey` into a normalized `Rc<str>` value: the
//!   model's raw `&str` for border / text overrides, or a parsed `[u8; 3]`
//!   for the conditional-formatting data bar. Linear scan, bounded by the
//!   small set of distinct colors a sheet uses.

use std::cell::RefCell;
use std::rc::Rc;

use super::font;
use crate::painter::CssColor;

/// Renderer-lifetime intern table: a `RefCell`-guarded vec scanned linearly,
/// with the value built only on a miss.
///
/// Linear scan beats a `HashMap` at the cardinality these tables see (fewer
/// than ~10 unique fonts, and a handful of colors, per realistic sheet) and
/// keeps the insert-only lifetime obvious. `probe` tests a stored key without
/// building one, so the hit path stays allocation-free even when the stored
/// key owns a `Box<str>` the caller only has as a `&str`.
struct LinearIntern<K, V> {
    entries: RefCell<Vec<(K, V)>>,
}

impl<K, V: Clone> LinearIntern<K, V> {
    fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
        }
    }

    /// Cached value for the entry `probe` accepts; `make_key` + `make_value`
    /// run once on a miss. Hit: one `V::clone` ([`Rc::clone`] for both tables
    /// above). Miss: one scan, one key, one value, inserted together.
    fn get_or_insert(
        &self,
        probe: impl Fn(&K) -> bool,
        make_key: impl FnOnce() -> K,
        make_value: impl FnOnce() -> V,
    ) -> V {
        let mut entries = self.entries.borrow_mut();
        if let Some((_, value)) = entries.iter().find(|(key, _)| probe(key)) {
            return value.clone();
        }
        let value = make_value();
        entries.push((make_key(), value.clone()));
        value
    }
}

/// Renderer-lifetime intern table for `ctx.font` strings.
///
/// `font::build` is the *only* allocation source on the per-cell text path
/// that doesn't depend on cell content. Realistic spreadsheets touch fewer
/// than ~10 unique (size, bold, italic, family) tuples, so a linear scan
/// beats a HashMap for the actual cardinality. Lives on `RendererCore`,
/// not `FrameCache`, because it is *cross-frame*: the same fonts repeat
/// every repaint.
pub struct FontIntern {
    entries: LinearIntern<FontKey, Rc<str>>,
}

#[derive(PartialEq, Eq)]
struct FontKey {
    size_bits: u64,
    bold: bool,
    italic: bool,
    family: Box<str>,
}

impl FontIntern {
    pub fn new() -> Self {
        Self {
            entries: LinearIntern::new(),
        }
    }

    /// Returns the interned `ctx.font` string for `(size_px, bold, italic, family)`.
    /// Cache hit: zero alloc, just an `Rc::clone`. Miss: one `font::build` +
    /// one `Box<str>` for the key family + one `Rc<str>` for the value.
    pub fn get_or_build(
        &self,
        size_px: f64,
        bold: bool,
        italic: bool,
        family: &str,
        fallback: &str,
    ) -> Rc<str> {
        let size_bits = size_px.to_bits();
        self.entries.get_or_insert(
            |key| {
                key.size_bits == size_bits
                    && key.bold == bold
                    && key.italic == italic
                    && &*key.family == family
            },
            || FontKey {
                size_bits,
                bold,
                italic,
                family: family.into(),
            },
            || font::build(size_px, bold, italic, family, fallback).into(),
        )
    }
}

impl Default for FontIntern {
    fn default() -> Self {
        Self::new()
    }
}

/// Renderer-lifetime intern table for per-cell color strings (border + text
/// overrides, CF data-bar fills). Keyed by a `ColorKey` so the hit path
/// stays zero-alloc: the model's raw string for border / text overrides, a
/// parsed rgb triple for the data bar, whose CSS color the renderer would
/// otherwise re-format once per decorated cell per frame. The value is the
/// normalized output the painter actually consumes; two keys that normalize
/// to the same color produce two entries — accepted, cardinality is bounded
/// by the small set of distinct colors a sheet uses.
pub struct ColorIntern {
    entries: LinearIntern<ColorKey, Rc<str>>,
}

/// One interned color's identity. `Raw` keeps the model's own string;
/// `Rgb` keeps the parsed triple, so no `String` is built to look up a
/// data-bar color.
enum ColorKey {
    Raw(Box<str>),
    Rgb([u8; 3]),
}

impl ColorIntern {
    pub fn new() -> Self {
        Self {
            entries: LinearIntern::new(),
        }
    }

    /// Returns the interned normalized color for `raw`. Hit: `Rc::clone`.
    /// Miss: one `CssColor::new(raw).into_string()` + one `Box<str>` key +
    /// one `Rc<str>` value, then `Rc::clone` for the return.
    pub fn get(&self, raw: &str) -> Rc<str> {
        self.entries.get_or_insert(
            |key| matches!(key, ColorKey::Raw(k) if &**k == raw),
            || ColorKey::Raw(raw.into()),
            || CssColor::new(raw).into_string().into(),
        )
    }

    /// Returns the interned `#rrggbb` color for a parsed rgb triple. Hit:
    /// `Rc::clone`, no formatting. Miss: one `format!` for the whole
    /// renderer lifetime of that color + one `Rc<str>` value, then
    /// `Rc::clone` for the return. Lowercase hex digits make the result its
    /// own `CssColor` normalization, so no second pass runs.
    pub fn get_rgb(&self, rgb: [u8; 3]) -> Rc<str> {
        self.entries.get_or_insert(
            |key| matches!(key, ColorKey::Rgb(k) if *k == rgb),
            || ColorKey::Rgb(rgb),
            || rgb_hex(rgb).into(),
        )
    }
}

/// Format an `[R, G, B]` triple as a `#rrggbb` CSS color string.
fn rgb_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

impl Default for ColorIntern {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_formatting_is_lowercase_and_zero_padded() {
        let intern = ColorIntern::new();
        assert_eq!(&*intern.get_rgb([0x33, 0x66, 0xCC]), "#3366cc");
        assert_eq!(&*intern.get_rgb([0, 8, 255]), "#0008ff");
    }

    #[test]
    fn rgb_repeat_hits_reuse_one_string() {
        let intern = ColorIntern::new();
        let first = intern.get_rgb([0x33, 0x66, 0xCC]);
        let second = intern.get_rgb([0x33, 0x66, 0xCC]);
        assert!(
            Rc::ptr_eq(&first, &second),
            "a repeated rgb must reuse the formatted string, not re-format it"
        );
        assert!(!Rc::ptr_eq(&first, &intern.get_rgb([0, 0, 0])));
    }

    #[test]
    fn raw_and_rgb_paths_agree_on_the_painter_visible_string() {
        let intern = ColorIntern::new();
        assert_eq!(
            &*intern.get("#3366CC"),
            &*intern.get_rgb([0x33, 0x66, 0xcc])
        );
    }
}
