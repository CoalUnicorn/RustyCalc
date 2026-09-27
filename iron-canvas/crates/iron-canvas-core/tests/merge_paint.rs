//! Merge paint smoke coverage: the merge pass paints the anchor's content over
//! the merged rectangle, including when the anchor itself is scrolled out of
//! view.

mod common;

use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{CanvasMetrics, CellStyle, Orchestrator, PaintResult, RCRange};
use iron_canvas_recorder::MemSurface;

use common::TestModel;

fn build(model: Rc<TestModel>) -> Orchestrator<MemSurface> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(
        CanvasMetrics::new(CanvasSize { w: 400.0, h: 300.0 }, 1.0)
            .expect("test canvas metrics are valid"),
    );
    orch.set_model(model);
    orch
}

/// The merge pass paints the anchor's value and a fill covering the whole
/// merged rectangle, not just a single cell.
#[test]
fn a_visible_merge_paints_the_anchor_value_over_the_range() {
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(1)
            .with_merged_ranges(vec![RCRange {
                r1: 2,
                c1: 2,
                r2: 4,
                c2: 3,
            }]),
    );
    model.set_cell(2, 2, "merged");
    let mut orch = build(Rc::clone(&model));

    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let ops = orch.grid_surface().recorder().ops();
    // A fill covering the merge: wider than one column, taller than one row.
    let merge_fill = ops.iter().find_map(|op| match op {
        iron_canvas_recorder::DrawOp::RectFill { rect, .. }
            if rect.width > 80 && rect.height > 40 =>
        {
            Some(*rect)
        }
        _ => None,
    });
    assert!(
        merge_fill.is_some(),
        "the merged rectangle must be filled as one cell"
    );
    let texts: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            iron_canvas_recorder::DrawOp::FillText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        texts.iter().any(|t| t == "merged"),
        "the anchor value must paint, got {texts:?}"
    );
}

/// An anchor scrolled out of view still supplies the merge's value: the
/// covered cells visible in the frame must render the anchor, not their own
/// content.
#[test]
fn an_offscreen_anchor_still_supplies_the_merge_value() {
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(10)
            .with_merged_ranges(vec![RCRange {
                r1: 1,
                c1: 1,
                r2: 30,
                c2: 2,
            }]),
    );
    model.set_cell(1, 1, "anchor-value");
    let mut orch = build(Rc::clone(&model));

    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let ops = orch.grid_surface().recorder().ops();
    let texts: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            iron_canvas_recorder::DrawOp::FillText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        texts.iter().any(|t| t == "anchor-value"),
        "an offscreen anchor's value must still paint, got {texts:?}"
    );
}

/// A merge whose visible top edge mixes two border styles produces two
/// strokes with their own width/color; a uniform edge produces one. IronCalc
/// spreads the source style across the range and keeps each side only on the
/// perimeter, so the anchor alone does not hold the merged border.
#[test]
fn a_mixed_perimeter_edge_paints_one_stroke_per_style_run() {
    use iron_canvas_core::{Border, BorderItem, BorderStyle};
    use iron_canvas_recorder::DrawOp;

    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(1)
            .with_merged_ranges(vec![RCRange {
                r1: 2,
                c1: 2,
                r2: 2,
                c2: 3,
            }]),
    );
    model.set_style(
        2,
        2,
        CellStyle {
            border: Border {
                top: Some(BorderItem {
                    style: BorderStyle::Thick,
                    color: Some("#ff0000".to_string()),
                }),
                ..Border::default()
            },
            ..CellStyle::default()
        },
    );
    model.set_style(
        2,
        3,
        CellStyle {
            border: Border {
                top: Some(BorderItem {
                    style: BorderStyle::Thin,
                    color: Some("#0000ff".to_string()),
                }),
                ..Border::default()
            },
            ..CellStyle::default()
        },
    );
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let ops = orch.grid_surface().recorder().ops();
    let colors: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::StrokeLine { color, .. } => Some(color.clone()),
            _ => None,
        })
        .collect();
    assert!(
        colors.iter().any(|c| c == "#ff0000") && colors.iter().any(|c| c == "#0000ff"),
        "both style runs must paint their own stroke, got {colors:?}"
    );
}

// ── Display-cell queries (Task 10) ──

/// A covered physical cell resolves to its merge's anchor and full range, even
/// when the anchor itself is scrolled out of view, and the reported fragment
/// lies inside the canvas.
#[test]
fn a_covered_cell_resolves_to_its_offscreen_anchor() {
    use iron_canvas_core::CellCoord;

    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(10)
            .with_merged_ranges(vec![RCRange {
                r1: 1,
                c1: 1,
                r2: 30,
                c2: 2,
            }]),
    );
    model.set_cell(1, 1, "anchor-value");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let rect = orch.cell_rect(12, 1).expect("row 12 must be visible");
    let cell = orch
        .display_cell_at(f64::from(rect.left() + 1), f64::from(rect.top() + 1))
        .expect("the point is on the grid");

    assert_eq!(cell.cell, CellCoord { row: 12, col: 1 });
    assert_eq!(cell.anchor, RCRange::from_cell(1, 1));
    assert_eq!(
        cell.merged,
        RCRange {
            r1: 1,
            c1: 1,
            r2: 30,
            c2: 2
        }
    );
    assert!(cell.link.is_none());
    // The visible fragment must lie inside the 400x300 canvas.
    assert!(cell.fragment.left() >= 0 && cell.fragment.top() >= 0);
    assert!(cell.fragment.right() <= 400 && cell.fragment.bottom() <= 300);
}

/// An unmerged cell reports itself as its own anchor and range.
#[test]
fn an_unmerged_cell_reports_itself() {
    use iron_canvas_core::CellCoord;

    let model = Rc::new(TestModel::synthetic_grid().with_top_row(1));
    model.set_cell(3, 4, "plain");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let rect = orch.cell_rect(3, 4).expect("cell must be visible");
    let cell = orch
        .display_cell_at(f64::from(rect.left() + 1), f64::from(rect.top() + 1))
        .expect("the point is on the grid");

    assert_eq!(cell.cell, CellCoord { row: 3, col: 4 });
    assert_eq!(cell.anchor, RCRange::from_cell(3, 4));
    assert_eq!(cell.merged, RCRange::from_cell(3, 4));
    assert_eq!(cell.fragment, rect);
}
