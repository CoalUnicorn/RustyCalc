use crate::coord::ActiveRef;
use crate::coord::{Absolute, RefNode, SheetRange, TextRef};
use crate::input::formula_overlay::*;
use iron_canvas_core::types::coord::FormulaRefKind;

fn make_ref(start: usize, end: usize, color_idx: usize) -> ActiveRef {
    ActiveRef {
        ref_node: RefNode::cell(
            0,
            None,
            0,
            0,
            Absolute {
                row: false,
                column: false,
            },
        ),
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
