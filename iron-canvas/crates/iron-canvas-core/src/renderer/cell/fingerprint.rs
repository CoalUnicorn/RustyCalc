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

use crate::style::{BorderItem, CellKind, CellStyle};

use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;

/// Fingerprint the bulk-fetched buffers for one pane. Same range +
/// same buffers ⇒ same `u64` (modulo `DefaultHasher` collision, 2⁻⁶⁴).
///
/// Range is folded in so two panes with structurally-identical data at
/// different addresses don't collide.
pub fn compute_pane_fingerprint(
    styles: &[Fetched<CellStyle>],
    values: &[Fetched<String>],
    cell_types: &[Fetched<CellKind>],
    range: RCRange,
) -> u64 {
    let mut h = DefaultHasher::new();
    h.write_i32(range.r1);
    h.write_i32(range.c1);
    h.write_i32(range.r2);
    h.write_i32(range.c2);
    // `Absent` and `BridgeFailed` both hash as the empty tag `0` — they paint
    // identically *within a single frame's walk* (nothing drawn), so the
    // fingerprint cannot tell them apart and Stage 1 stays behavior-preserving.
    // The hold-on-`BridgeFailed` decision is made by the preflight *before* the
    // fingerprint is committed (Stage 2), never here.
    for s in styles {
        match s {
            Fetched::Absent | Fetched::BridgeFailed => h.write_u8(0),
            Fetched::Value(style) => {
                h.write_u8(1);
                StyleDigest(style).hash(&mut h);
            }
        }
    }
    for v in values {
        match v {
            Fetched::Absent | Fetched::BridgeFailed => h.write_u8(0),
            Fetched::Value(text) => {
                h.write_u8(1);
                h.write_usize(text.len());
                h.write(text.as_bytes());
            }
        }
    }
    for t in cell_types {
        match t {
            Fetched::Absent | Fetched::BridgeFailed => h.write_u8(0),
            Fetched::Value(ct) => {
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
pub struct StyleDigest<'a>(pub &'a CellStyle);

impl<'a> Hash for StyleDigest<'a> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let s = self.0;

        s.fill_color.hash(state);

        s.font.strike.hash(state);
        s.font.underline.hash(state);
        s.font.bold.hash(state);
        s.font.italic.hash(state);
        // f64 is not Hash — hash the bit pattern instead. Font size is always
        // a finite positive number here, so to_bits() produces a stable value.
        s.font.size.to_bits().hash(state);
        s.font.color.hash(state);
        s.font.name.hash(state);

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
