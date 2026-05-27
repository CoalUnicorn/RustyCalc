//! Pure-function helper for splitting a formula string into colored ref
//! tokens and neutral gaps. Drives the `<FormulaOverlay>` component.
//!
//! ## Encoding note: byte offsets vs UTF-16 code units
//!
//! `ActiveRef.span` uses byte offsets into the Rust `&str` (from the lexer).
//! All segments are byte ranges that the view function slices from the
//! formula text — no offset crosses the JS/Rust boundary, so the
//! UTF-16-vs-bytes mismatch never arises. The lexer guarantees char-boundary
//! spans, so non-ASCII sheet names slice safely too.

use crate::coord::ActiveRef;
use std::ops::Range;

/// One segment of formula text destined for the overlay.
///
/// `range` is a byte range into the formula string (held by the caller).
/// `color_idx` is the palette index (pre-modulo'd) for ref tokens,
/// or `None` for neutral gaps. The actual color string is resolved at
/// view time from `FORMULA_REF_COLORS`.
#[derive(Clone, Debug, PartialEq)]
pub struct FormulaSegment {
    pub range: Range<usize>,
    pub color_idx: Option<u8>,
}

/// Split `formula` into colored ref segments and neutral gap segments,
/// in left-to-right order, covering every byte of `formula`.
///
/// `refs` must be sorted by `span.start` (the analyzer emits them in token
/// order). `palette_len` is `FORMULA_REF_COLORS.len()` — used for modulo
/// wrapping of `ActiveRef.color_idx`.
///
/// The caller holds the formula text; segments reference it by byte range.
pub fn split_formula_by_refs(
    formula: &str,
    refs: &[ActiveRef],
    palette_len: usize,
) -> Vec<FormulaSegment> {
    if refs.is_empty() {
        return vec![FormulaSegment {
            range: 0..formula.len(),
            color_idx: None,
        }];
    }

    let mut segments = Vec::with_capacity(refs.len() * 2 + 1);
    let mut cursor = 0usize;

    for ar in refs {
        let span = ar.span;
        // debug_assert: catch lexer regressions that produce unsorted spans
        debug_assert!(
            cursor <= span.start,
            "ActiveRef spans must be sorted by start; cursor={cursor}, span.start={}",
            span.start
        );
        if cursor < span.start {
            segments.push(FormulaSegment {
                range: cursor..span.start,
                color_idx: None,
            });
        }
        let idx = (ar.color_idx % palette_len) as u8;
        segments.push(FormulaSegment {
            range: span.start..span.end,
            color_idx: Some(idx),
        });
        cursor = span.end;
    }

    if cursor < formula.len() {
        segments.push(FormulaSegment {
            range: cursor..formula.len(),
            color_idx: None,
        });
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::{Absolute, RefNode, SheetRange, TextRef};
    use iron_canvas_core::types::coord::FormulaRefKind;

    fn make_ref(start: usize, end: usize, color_idx: usize) -> ActiveRef {
        ActiveRef {
            ref_node: RefNode::cell(0, None, 0, 0, Absolute { row: false, column: false }),
            sheet_area: SheetRange::from_cell(0, 0, 0),
            color_idx,
            span: TextRef { start, end },
            kind: FormulaRefKind::Direct,
        }
    }

    const PALETTE_LEN: usize = 3;

    fn reconstruct(formula: &str, segs: &[FormulaSegment]) -> String {
        segs.iter().map(|s| &formula[s.range.clone()]).collect()
    }

    #[test]
    fn no_refs_returns_neutral_full_text() {
        let segs = split_formula_by_refs("=SUM(1,2)", &[], PALETTE_LEN);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].range, 0..9);
        assert!(segs[0].color_idx.is_none());
    }

    #[test]
    fn single_ref() {
        let refs = vec![make_ref(1, 3, 0)];
        let segs = split_formula_by_refs("=A1+5", &refs, PALETTE_LEN);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].range, 0..1);
        assert_eq!(segs[1].range, 1..3);
        assert_eq!(segs[1].color_idx, Some(0));
        assert_eq!(segs[2].range, 3..5);
    }

    #[test]
    fn adjacent_refs() {
        let refs = vec![make_ref(1, 3, 0), make_ref(4, 6, 1)];
        let segs = split_formula_by_refs("=A1+B2", &refs, PALETTE_LEN);
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[0].range, 0..1); // "="
        assert_eq!(segs[1].range, 1..3); // "A1"
        assert_eq!(segs[1].color_idx, Some(0));
        assert_eq!(segs[2].range, 3..4); // "+"
        assert_eq!(segs[3].range, 4..6); // "B2"
        assert_eq!(segs[3].color_idx, Some(1));
    }

    #[test]
    fn same_cell_reuses_color() {
        let refs = vec![make_ref(1, 3, 0), make_ref(4, 6, 0)];
        let segs = split_formula_by_refs("=A1+A1", &refs, PALETTE_LEN);
        assert_eq!(segs[1].color_idx, Some(0));
        assert_eq!(segs[3].color_idx, Some(0));
    }

    #[test]
    fn range_ref_is_one_span() {
        let refs = vec![make_ref(5, 10, 0)];
        let segs = split_formula_by_refs("=SUM(B2:B5)", &refs, PALETTE_LEN);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[1].range, 5..10);
    }

    #[test]
    fn preserves_full_text() {
        let formula = "=A1+SUM(B2:B5)";
        let refs = vec![make_ref(1, 3, 0), make_ref(8, 13, 1)];
        let segs = split_formula_by_refs(formula, &refs, PALETTE_LEN);
        let reconstructed = reconstruct(formula, &segs);
        assert_eq!(reconstructed, formula);
    }

    #[test]
    fn color_idx_wraps_modulo_palette() {
        let refs = vec![make_ref(1, 3, 5)];
        let segs = split_formula_by_refs("=A1", &refs, PALETTE_LEN);
        // 5 % 3 = 2
        assert_eq!(segs[1].color_idx, Some(2));
    }

    #[test]
    fn gap_between_adjacent_refs_is_zero_length_not_emitted() {
        // A1 ends at 3, B2 starts at 3 — no gap
        let refs = vec![make_ref(1, 3, 0), make_ref(3, 5, 1)];
        let segs = split_formula_by_refs("=A1B2", &refs, PALETTE_LEN);
        // Should be: '=', 'A1', 'B2' — no empty gap
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[1].range, 1..3);
        assert_eq!(segs[2].range, 3..5);
    }
}
