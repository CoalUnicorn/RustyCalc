//! Per-pane content fingerprint for the Stage 1 paint-skip optimization.
//!
//! `compute_pane_fingerprint` walks the bulk-fetched buffers
//! (`pane_styles`, `pane_values`, `pane_cell_types`) and produces a `u64`
//! summary of the painted-visible state. `render_pane` will compare the
//! result against `frame.prev_pane_fingerprints[pane]` on slots-reuse
//! frames: equal → skip the 4-pass walk; differ → repaint.
//!
//! Hash domain — the set of inputs that determine painted pixels.
//! Anything that affects paint MUST be included; anything that doesn't
//! affect paint must NOT be. Two cells whose digests match must paint
//! identical pixels, otherwise the skip leaks visual staleness.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ironcalc_base::types::{BorderItem, CellType, Style};

use crate::types::coord::RCRange;

/// Fingerprint the bulk-fetched buffers for one pane. Same range +
/// same buffers ⇒ same `u64` (modulo `DefaultHasher` collision, 2⁻⁶⁴).
///
/// Range is folded in so two panes with structurally-identical data at
/// different addresses don't collide.
pub fn compute_pane_fingerprint(
    styles: &[Option<Style>],
    values: &[Option<String>],
    cell_types: &[Option<CellType>],
    range: RCRange,
) -> u64 {
    let mut h = DefaultHasher::new();
    h.write_i32(range.r1);
    h.write_i32(range.c1);
    h.write_i32(range.r2);
    h.write_i32(range.c2);
    for s in styles {
        match s {
            None => h.write_u8(0),
            Some(style) => {
                h.write_u8(1);
                StyleDigest(style).hash(&mut h);
            }
        }
    }
    for v in values {
        match v {
            None => h.write_u8(0),
            Some(text) => {
                h.write_u8(1);
                h.write_usize(text.len());
                h.write(text.as_bytes());
            }
        }
    }
    for t in cell_types {
        match t {
            None => h.write_u8(0),
            Some(ct) => {
                h.write_u8(1);
                std::mem::discriminant(ct).hash(&mut h);
            }
        }
    }
    h.finish()
}

/// Hashable view over the subset of `Style` fields that affect painted
/// pixels. The field selection is load-bearing: a paint-read field the
/// digest misses ⇒ stale pixels on skip; a paint-irrelevant field the
/// digest includes ⇒ unnecessary repaint when only that field changed.
pub struct StyleDigest<'a>(pub &'a Style);

impl<'a> Hash for StyleDigest<'a> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let s = self.0;

        s.fill.color.hash(state);

        s.font.strike.hash(state);
        s.font.u.hash(state);
        s.font.b.hash(state);
        s.font.i.hash(state);
        s.font.sz.hash(state);
        s.font.color.hash(state);
        s.font.name.hash(state);
        s.font.family.hash(state);

        match &s.alignment {
            None => state.write_u8(0),
            Some(a) => {
                state.write_u8(1);
                std::mem::discriminant(&a.horizontal).hash(state);
                std::mem::discriminant(&a.vertical).hash(state);
                a.wrap_text.hash(state);
            }
        }

        hash_border_item(&s.border.left, state);
        hash_border_item(&s.border.right, state);
        hash_border_item(&s.border.top, state);
        hash_border_item(&s.border.bottom, state);
        s.border.diagonal_up.hash(state);
        s.border.diagonal_down.hash(state);
    }
}

fn hash_border_item<H: Hasher>(b: &Option<BorderItem>, state: &mut H) {
    match b {
        None => state.write_u8(0),
        Some(bi) => {
            state.write_u8(1);
            std::mem::discriminant(&bi.style).hash(state);
            bi.color.hash(state);
        }
    }
}
