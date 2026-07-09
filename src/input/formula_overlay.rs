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
