//! Renderer-lifetime intern tables: dedup CSS strings into `Rc<str>` so
//! the per-cell hot path is `Rc::clone` instead of `String::clone` (or
//! `format!`) for repeated colors / fonts / column labels.
//!
//! Two tables, two keying strategies:
//! - [`FontIntern`] — composite key `(size, bold, italic, family)`, linear
//!   scan; bounded by ~10 unique tuples per realistic sheet.
//! - [`ColorIntern`] — raw `&str` key into normalized `Rc<str>` value;
//!   linear scan, bounded by the small set of distinct colors a sheet uses.

use std::cell::RefCell;
use std::rc::Rc;

use super::font;
use crate::painter::CssColor;

/// Renderer-lifetime intern table for `ctx.font` strings.
///
/// `font::build` is the *only* allocation source on the per-cell text path
/// that doesn't depend on cell content. Realistic spreadsheets touch fewer
/// than ~10 unique (size, bold, italic, family) tuples, so a linear scan
/// beats a HashMap for the actual cardinality. Lives on `RendererCore`,
/// not `FrameCache`, because it is *cross-frame*: the same fonts repeat
/// every repaint.
pub struct FontIntern {
    entries: RefCell<Vec<(FontKey, Rc<str>)>>,
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
            entries: RefCell::new(Vec::new()),
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
        let mut entries = self.entries.borrow_mut();
        let hit = entries.iter().find(|(key, _)| {
            key.size_bits == size_bits
                && key.bold == bold
                && key.italic == italic
                && &*key.family == family
        });
        if let Some((_, css)) = hit {
            return Rc::clone(css);
        }
        let css: Rc<str> = font::build(size_px, bold, italic, family, fallback).into();
        entries.push((
            FontKey {
                size_bits,
                bold,
                italic,
                family: family.into(),
            },
            Rc::clone(&css),
        ));
        css
    }
}

impl Default for FontIntern {
    fn default() -> Self {
        Self::new()
    }
}

/// Renderer-lifetime intern table for per-cell color strings (border + text
/// overrides). Keyed by the **raw** `&str` from the model so cache-hit lookups
/// stay zero-alloc (no normalization on the hit path); the value is the
/// `CssColor`-normalized output the painter actually consumes. Two raws that
/// normalize to the same color produce two entries — accepted, cardinality is
/// bounded by the small set of distinct colors a sheet uses.
pub struct ColorIntern {
    entries: RefCell<Vec<(Box<str>, Rc<str>)>>,
}

impl ColorIntern {
    pub fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
        }
    }

    /// Returns the interned normalized color for `raw`. Hit: `Rc::clone`.
    /// Miss: one `CssColor::new(raw).into_string()` + one `Box<str>` key +
    /// one `Rc<str>` value, then `Rc::clone` for the return.
    pub fn get(&self, raw: &str) -> Rc<str> {
        let mut entries = self.entries.borrow_mut();
        if let Some((_, css)) = entries.iter().find(|(key, _)| &**key == raw) {
            return Rc::clone(css);
        }
        let css: Rc<str> = CssColor::new(raw).into_string().into();
        entries.push((raw.into(), Rc::clone(&css)));
        css
    }
}

impl Default for ColorIntern {
    fn default() -> Self {
        Self::new()
    }
}
