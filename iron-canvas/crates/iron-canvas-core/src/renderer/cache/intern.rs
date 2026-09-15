//! Renderer-lifetime intern tables: dedup repeated strings into `Rc<str>` so
//! the per-cell hot path is `Rc::clone` instead of `String::clone` (or
//! `format!`) for repeated colors and fonts.
//!
//! Each table uses `LinearIntern` with its own key and value constructor.
//! A lookup builds the key and value only on a miss:
//! - [`FontIntern`] — size, bold, italic, family, and the fallback when the
//!   family is blank. The value comes from `font::build`.
//! - [`ColorIntern`] — `ColorKey` into a normalized `Rc<str>` value: the
//!   model's raw `&str` for border / text overrides, or a parsed `[u8; 3]`
//!   for the conditional-formatting data bar.
//!
//! Entries remain for the renderer lifetime. Neither table has a size limit.

use std::cell::RefCell;
use std::rc::Rc;

use super::font;
use crate::painter::CssColor;

/// Renderer-lifetime intern table: a `RefCell`-guarded vec scanned linearly,
/// with the value built only on a miss.
///
/// A lookup scans the stored keys. `probe` borrows each key, so a hit does
/// not allocate a key. Both callers use `Rc<str>` values, so cloning a hit
/// does not allocate a value either.
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
/// Stored on `RendererCore` so repeated fonts reuse the same string across
/// cells and frames. Only a cache miss calls `font::build`.
pub struct FontIntern {
    entries: LinearIntern<FontKey, Rc<str>>,
}

#[derive(PartialEq, Eq)]
struct FontKey {
    size_bits: u64,
    bold: bool,
    italic: bool,
    family: Box<str>,
    fallback: Option<Box<str>>,
}

impl FontIntern {
    pub fn new() -> Self {
        Self {
            entries: LinearIntern::new(),
        }
    }

    /// Returns the interned `ctx.font` string for the supplied font.
    /// The key includes `fallback` only when `family` is blank.
    /// A hit clones the `Rc` without allocation. A miss builds the CSS
    /// string and stores the family and any used fallback in the key.
    pub fn get_or_build(
        &self,
        size_px: f64,
        bold: bool,
        italic: bool,
        family: &str,
        fallback: &str,
    ) -> Rc<str> {
        let size_bits = size_px.to_bits();
        let used_fallback = family.trim().is_empty().then_some(fallback);
        self.entries.get_or_insert(
            |key| {
                key.size_bits == size_bits
                    && key.bold == bold
                    && key.italic == italic
                    && &*key.family == family
                    && key.fallback.as_deref() == used_fallback
            },
            || FontKey {
                size_bits,
                bold,
                italic,
                family: family.into(),
                fallback: used_fallback.map(Into::into),
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
/// to the same color produce two entries. Entries remain until the renderer
/// is dropped.
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
