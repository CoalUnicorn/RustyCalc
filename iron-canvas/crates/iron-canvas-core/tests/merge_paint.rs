//! Merge paint smoke coverage: the merge pass paints the anchor's content over
//! the merged rectangle, including when the anchor itself is scrolled out of
//! view.

mod common;

use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{CanvasMetrics, CellStyle, Orchestrator, PaintResult, RCRange};
use iron_canvas_recorder::MemSurface;

use common::TestModel;

/// The text of every `FillText` op, in paint order.
fn texts(ops: &[iron_canvas_recorder::DrawOp]) -> Vec<String> {
    ops.iter()
        .filter_map(|op| match op {
            iron_canvas_recorder::DrawOp::FillText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

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

/// A covered cell reports the anchor's committed link, which is what the host
/// activates and what the tooltip exists for: the physical lookup the host used
/// before would miss a link the user can see.
#[test]
fn a_covered_cell_reports_the_anchors_link() {
    use iron_canvas_core::{CellCoord, CellLink, LinkTarget};

    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(1)
            .with_merged_ranges(vec![RCRange {
                r1: 2,
                c1: 2,
                r2: 4,
                c2: 3,
            }])
            .with_sheet_links(vec![CellLink::new(
                RCRange::from_cell(2, 2),
                LinkTarget::External("https://anchor.example".to_string()),
                None,
                false,
                None,
            )]),
    );
    model.set_cell(2, 2, "anchor");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let covered = orch.cell_rect(3, 3).expect("covered cell must be visible");
    let cell = orch
        .display_cell_at(f64::from(covered.left() + 1), f64::from(covered.top() + 1))
        .expect("the covered cell is on the grid");

    assert_eq!(cell.cell, CellCoord { row: 3, col: 3 });
    let link = cell
        .link
        .expect("the anchor's link must travel with the merge");
    assert_eq!(link.target().as_str(), "https://anchor.example");
    assert_eq!(cell.anchor, RCRange::from_cell(2, 2));
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

// ── Offscreen anchor: full logical dimensions ──

/// Only covered cells visible, vertically: the fill covers every visible row of
/// the merge, not just the covered cell the renderer happened to start from.
#[test]
fn a_vertical_offscreen_anchor_uses_the_full_logical_height() {
    use iron_canvas_recorder::DrawOp;

    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(6)
            .with_merged_ranges(vec![RCRange {
                r1: 1,
                c1: 1,
                r2: 9,
                c2: 1,
            }]),
    );
    model.set_cell(1, 1, "anchor");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let row = orch.cell_rect(7, 1).expect("row 7 must be visible");
    let ops = orch.grid_surface().recorder().ops();
    assert!(
        ops.iter()
            .any(|op| matches!(op, DrawOp::RectFill { rect, .. }
            if rect.height > row.height && rect.width == row.width)),
        "the merge fill must span every visible row of the merge"
    );
    assert!(texts(&ops).iter().any(|t| t == "anchor"));
}

/// Horizontal mirror: the fill covers every visible column of the merge.
#[test]
fn a_horizontal_offscreen_anchor_uses_the_full_logical_width() {
    use iron_canvas_recorder::DrawOp;

    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_left_column(6)
            .with_merged_ranges(vec![RCRange {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 9,
            }]),
    );
    model.set_cell(1, 1, "anchor");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let cell = orch.cell_rect(1, 7).expect("column 7 must be visible");
    let ops = orch.grid_surface().recorder().ops();
    assert!(
        ops.iter()
            .any(|op| matches!(op, DrawOp::RectFill { rect, .. }
            if rect.width > cell.width && rect.height == cell.height)),
        "the merge fill must span every visible column of the merge"
    );
    assert!(texts(&ops).iter().any(|t| t == "anchor"));
}

// ── Anchor fill, CF decoration, wrap, neighbours ──

/// The anchor's fill and conditional-formatting decoration paint over the whole
/// merged rectangle, after the interior cell strokes they cover.
#[test]
fn the_anchor_fill_and_cf_decoration_cover_the_merge() {
    use iron_canvas_core::{CellDecoration, DataBarSpec};
    use iron_canvas_recorder::DrawOp;

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
    model.set_cell(2, 2, "x");
    model.set_style(
        2,
        2,
        CellStyle {
            fill_color: Some("#ff0000".to_string()),
            ..CellStyle::default()
        },
    );
    model.set_decoration(
        2,
        2,
        CellDecoration::DataBar(DataBarSpec {
            fraction: 1.0,
            color: "#112233".to_string(),
        }),
    );
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let ops = orch.grid_surface().recorder().ops();
    let fill = ops
        .iter()
        .position(|op| matches!(op, DrawOp::RectFill { color, .. } if color == "#ff0000"))
        .expect("the anchor fill must paint");
    let bar = ops
        .iter()
        .position(|op| matches!(op, DrawOp::RectFill { color, .. } if color == "#112233"))
        .expect("the anchor CF data bar must paint");
    assert!(bar > fill, "the CF decoration paints over the anchor fill");
}

/// A neighbour's explicit border on the shared edge paints in the per-cell
/// pass; the merge re-paints its own perimeter afterwards, so the merge wins
/// the shared pixels. The covered cell's own border is painted too and then
/// covered — that redundancy is deliberate (see the merge-paint comments).
#[test]
fn the_merge_perimeter_paints_after_a_neighbour_border() {
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
    model.set_cell(2, 2, "x");
    // The merge's right perimeter runs along column 3's right edge.
    model.set_style(
        2,
        3,
        CellStyle {
            border: Border {
                right: Some(BorderItem {
                    style: BorderStyle::Thick,
                    color: Some("#00ff00".to_string()),
                }),
                ..Border::default()
            },
            ..CellStyle::default()
        },
    );
    // The neighbour to the right carries its own left border on that edge.
    model.set_style(
        2,
        4,
        CellStyle {
            border: Border {
                left: Some(BorderItem {
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
    let neighbour = ops
        .iter()
        .position(|op| matches!(op, DrawOp::StrokeLine { color, .. } if color == "#0000ff"))
        .expect("the neighbour's left border must paint");
    // The *last* green stroke is the merge pass's perimeter: the covered cell's
    // own right border paints earlier, in the per-cell pass.
    let perimeter = ops
        .iter()
        .rposition(|op| matches!(op, DrawOp::StrokeLine { color, .. } if color == "#00ff00"))
        .expect("the merge's right perimeter must paint");
    let covered_cell_border = ops
        .iter()
        .position(|op| matches!(op, DrawOp::StrokeLine { color, .. } if color == "#00ff00"))
        .expect("the covered cell's own border paints in the per-cell pass");
    assert!(
        covered_cell_border < perimeter,
        "the covered cell's border must paint before the merge re-paints the perimeter"
    );
    assert!(
        perimeter > neighbour,
        "the merge perimeter paints after the neighbour border it shares"
    );
}

/// Wrapped text is laid out against the logical rectangle and clipped to the
/// visible fragment: the clip is the fragment, not a single cell.
#[test]
fn wrapped_merge_text_is_clipped_to_the_fragment() {
    use iron_canvas_recorder::DrawOp;

    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(1)
            .with_merged_ranges(vec![RCRange {
                r1: 2,
                c1: 1,
                r2: 3,
                c2: 4,
            }]),
    );
    model.set_cell(2, 1, "a long wrapped value that needs more than one line");
    model.set_style(
        2,
        1,
        CellStyle {
            alignment: Some(iron_canvas_core::Alignment {
                horizontal: iron_canvas_core::HAlign::General,
                vertical: iron_canvas_core::VAlign::Top,
                wrap_text: true,
            }),
            ..CellStyle::default()
        },
    );
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let cell = orch
        .cell_rect(2, 1)
        .expect("the anchor cell must be visible");
    let ops = orch.grid_surface().recorder().ops();
    let clip = ops
        .iter()
        .find_map(|op| match op {
            DrawOp::PushClip { rect } if rect.width > cell.width => Some(*rect),
            _ => None,
        })
        .expect("wrapped text must clip to the merged fragment");
    assert!(
        clip.height > cell.height,
        "the clip is the whole fragment, not the anchor's own cell"
    );
}

// ── Frozen boundaries ──

/// A merge spanning both frozen boundaries paints once per pane segment, and
/// the frozen separators still paint after the cells (so they stay visible).
#[test]
fn a_merge_crossing_both_frozen_boundaries_paints_per_segment() {
    use iron_canvas_core::painter::GroupClass;
    use iron_canvas_recorder::DrawOp;

    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_frozen(2, 2)
            .with_top_row(3)
            .with_left_column(3)
            .with_merged_ranges(vec![RCRange {
                r1: 1,
                c1: 1,
                r2: 4,
                c2: 4,
            }]),
    );
    model.set_cell(1, 1, "spanning");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let fragments = orch.visible_fragments(RCRange {
        r1: 1,
        c1: 1,
        r2: 4,
        c2: 4,
    });
    assert_eq!(
        fragments.len(),
        4,
        "the merge intersects all four panes: {fragments:?}"
    );

    let ops = orch.grid_surface().recorder().ops();
    let last_merge_fill = ops
        .iter()
        .rposition(|op| matches!(op, DrawOp::FillText { text, .. } if text == "spanning"))
        .expect("the merge text must paint");
    let separator_group = ops
        .iter()
        .position(
            |op| matches!(op, DrawOp::BeginGroup { class } if *class == GroupClass::FrozenSep),
        )
        .expect("the frozen separators must paint");
    assert!(
        separator_group > last_merge_fill,
        "frozen separators must paint after the merge"
    );
}

// ── Transactional failure ──

/// A failed merge preparation holds the attempt: the previous pixels stay and
/// the previous merge hit geometry stays queryable.
#[test]
fn a_failed_merge_preparation_keeps_pixels_and_hit_geometry() {
    // The anchor is above the viewport, so its content comes from the scalar
    // accessors — the read that can fail for a merge with no segment to read it
    // from.
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(6)
            .with_merged_ranges(vec![RCRange {
                r1: 1,
                c1: 1,
                r2: 9,
                c2: 2,
            }]),
    );
    model.set_cell(1, 1, "anchor");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let covered = orch.cell_rect(7, 1).expect("row 7 must be visible");
    let before = orch
        .display_cell_at(f64::from(covered.left() + 1), f64::from(covered.top() + 1))
        .expect("the covered cell resolves to its anchor");
    let ops_before = orch.grid_surface().recorder().ops().len();

    // Fail the offscreen anchor's style read only: geometry and the visible
    // segments still prepare, so this exercises the merge preparation path.
    model.set_style_bridge_fail_at(Some((1, 1)));
    model.set_cell(1, 1, "changed");
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);

    let after = orch
        .display_cell_at(f64::from(covered.left() + 1), f64::from(covered.top() + 1))
        .expect("the held attempt must keep the committed merge geometry");
    assert_eq!(after.anchor, before.anchor);
    assert_eq!(after.merged, before.merged);
    assert_eq!(
        orch.grid_surface().recorder().ops().len(),
        ops_before,
        "a held attempt must not paint"
    );
    assert!(
        !texts(&orch.grid_surface().recorder().ops())
            .iter()
            .any(|t| t == "changed"),
        "the held attempt must not publish the new anchor value either"
    );
}

// ── Merge-table contract ──

/// An out-of-bounds or overlapping merge list holds the attempt instead of
/// painting a fabricated table, and the committed geometry survives.
#[test]
fn an_invalid_merge_list_holds_the_attempt() {
    use iron_canvas_core::FrameInputFailure;

    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(1)
            .with_merged_ranges(vec![RCRange {
                r1: 2,
                c1: 2,
                r2: 3,
                c2: 3,
            }]),
    );
    model.set_cell(2, 2, "anchor");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let covered = orch.cell_rect(3, 3).expect("covered cell must be visible");
    let committed = orch
        .display_cell_at(f64::from(covered.left() + 1), f64::from(covered.top() + 1))
        .expect("the covered cell resolves to its anchor");

    for invalid in [
        vec![RCRange {
            r1: 0,
            c1: 1,
            r2: 1,
            c2: 1,
        }],
        vec![
            RCRange {
                r1: 1,
                c1: 1,
                r2: 3,
                c2: 3,
            },
            RCRange {
                r1: 3,
                c1: 3,
                r2: 5,
                c2: 5,
            },
        ],
    ] {
        model.set_merged_ranges(invalid);
        orch.mark_content_dirty();
        assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
        let held = orch
            .display_cell_at(f64::from(covered.left() + 1), f64::from(covered.top() + 1))
            .expect("the held attempt keeps the committed merge geometry");
        assert_eq!(held.anchor, committed.anchor);
        model.set_merged_ranges(vec![RCRange {
            r1: 2,
            c1: 2,
            r2: 3,
            c2: 3,
        }]);
    }

    // The named failure is reported, so a recording can attribute the hold.
    model.set_capture_fail(Some(FrameInputFailure::MergedRanges));
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(
        orch.last_trace().outcome,
        iron_canvas_core::FrameOutcome::HeldOnInputFailure(FrameInputFailure::MergedRanges)
    );
}

// ── Metadata discovery on an overlay-only wakeup ──

/// A merge the model gained since the last frame must commit even when the host
/// only asks for an overlay repaint: the capture sees the change, so dropping it
/// would leave committed query geometry wrong until some other attempt rebuilt.
#[test]
fn an_overlay_only_repaint_publishes_a_newly_discovered_merge() {
    let model = Rc::new(TestModel::synthetic_grid().with_top_row(1));
    model.set_cell(2, 2, "anchor");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let covered = orch.cell_rect(2, 3).expect("cell must be visible");
    let at = (f64::from(covered.left() + 1), f64::from(covered.top() + 1));
    assert_eq!(
        orch.display_cell_at(at.0, at.1)
            .expect("cell is on the grid")
            .anchor,
        RCRange::from_cell(2, 3),
        "before the merge, the cell is its own anchor"
    );

    // The model gains a merge; the host asks for overlay work only.
    model.set_merged_ranges(vec![RCRange {
        r1: 2,
        c1: 2,
        r2: 3,
        c2: 3,
    }]);
    orch.request_overlay_repaint();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let after = orch
        .display_cell_at(at.0, at.1)
        .expect("cell is on the grid");
    assert_eq!(
        after.anchor,
        RCRange::from_cell(2, 2),
        "the discovered merge must commit, not be dropped"
    );
    assert_eq!(
        after.merged,
        RCRange {
            r1: 2,
            c1: 2,
            r2: 3,
            c2: 3
        }
    );
}

// ── Per-fragment text transform across a frozen boundary ──

type Paint = (Vec<iron_canvas_core::PixelRect>, f64, f64);

/// Every paint of `value` with the clip stack active at the time, in paint
/// order.
///
/// A backend discards text drawn outside its clip, and text drawn *under* a
/// clip that a later op covers is not the final pixel either. Scoping an
/// assertion to the merge pass's own clip block is therefore the only way to
/// ask "did the merge show this text" — the per-cell pass paints the covered
/// anchor's text too, under the anchor cell's clip, before the merge covers it.
fn text_paints(ops: &[iron_canvas_recorder::DrawOp], value: &str) -> Vec<Paint> {
    use iron_canvas_recorder::DrawOp;
    let mut clips: Vec<iron_canvas_core::PixelRect> = Vec::new();
    let mut paints = Vec::new();
    for op in ops {
        match op {
            DrawOp::PushClip { rect } => clips.push(*rect),
            DrawOp::PopClip => {
                clips.pop();
            }
            DrawOp::FillText { text, x, y, .. } if text == value => {
                paints.push((clips.clone(), *x, *y));
            }
            _ => {}
        }
    }
    paints
}

/// How many paints of `value` were drawn under `fragment`'s clip and land
/// inside every clip of their own stack — i.e. are actually shown there.
fn shown_in(paints: &[Paint], fragment: iron_canvas_core::PixelRect) -> usize {
    fn inside(rect: iron_canvas_core::PixelRect, x: f64, y: f64) -> bool {
        x >= f64::from(rect.left()) - 1.0
            && x <= f64::from(rect.right()) + 1.0
            && y >= f64::from(rect.top()) - 1.0
            && y <= f64::from(rect.bottom()) + 1.0
    }
    paints
        .iter()
        .filter(|(stack, x, y)| {
            stack.contains(&fragment) && stack.iter().all(|clip| inside(*clip, *x, *y))
        })
        .count()
}

fn right_aligned(color: &str) -> CellStyle {
    use iron_canvas_core::{Alignment, HAlign, VAlign};
    CellStyle {
        font: iron_canvas_core::FontStyle {
            color: Some(color.to_string()),
            ..Default::default()
        },
        alignment: Some(Alignment {
            horizontal: HAlign::Right,
            vertical: VAlign::Center,
            wrap_text: false,
        }),
        ..CellStyle::default()
    }
}

/// A merge crossing a frozen **column** boundary: the scrolled fragment needs the
/// scrolled band's transform, not the frozen anchor's. Before the translation
/// fix, right-aligned text was painted at the frozen anchor's absolute position
/// under the scrolled fragment's clip and vanished.
#[test]
fn merge_text_uses_each_frozen_column_fragment_transform() {
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_frozen(0, 1)
            .with_left_column(4)
            .with_merged_ranges(vec![RCRange {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 6,
            }]),
    );
    model.set_cell(1, 1, "edge");
    model.set_style(1, 1, right_aligned("#000000"));
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let range = RCRange {
        r1: 1,
        c1: 1,
        r2: 1,
        c2: 6,
    };
    let fragments = orch.visible_fragments(range);
    assert_eq!(
        fragments.len(),
        2,
        "the merge must span the frozen column and the scrolled band: {fragments:?}"
    );

    let ops = orch.grid_surface().recorder().ops();
    let paints = text_paints(&ops, "edge");
    let frozen = fragments[0].1;
    let scrolled = fragments[1].1;
    assert_eq!(
        shown_in(&paints, scrolled),
        1,
        "right-aligned text belongs at the merge's right edge, in the scrolled fragment; fragments: {fragments:?} paints: {paints:?}"
    );
    assert_eq!(
        shown_in(&paints, frozen),
        0,
        "the frozen fragment shows no part of a right-aligned label; fragments: {fragments:?}"
    );
}

/// Column mirror of the above: a merge crossing a frozen **row** boundary with
/// bottom-aligned text.
#[test]
fn merge_text_uses_each_frozen_row_fragment_transform() {
    use iron_canvas_core::{Alignment, HAlign, VAlign};

    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_frozen(1, 0)
            .with_top_row(4)
            .with_merged_ranges(vec![RCRange {
                r1: 1,
                c1: 1,
                r2: 6,
                c2: 1,
            }]),
    );
    model.set_cell(1, 1, "edge");
    model.set_style(
        1,
        1,
        CellStyle {
            font: iron_canvas_core::FontStyle {
                color: Some("#000000".to_string()),
                ..Default::default()
            },
            alignment: Some(Alignment {
                horizontal: HAlign::Center,
                vertical: VAlign::Bottom,
                wrap_text: false,
            }),
            ..CellStyle::default()
        },
    );
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let range = RCRange {
        r1: 1,
        c1: 1,
        r2: 6,
        c2: 1,
    };
    let fragments = orch.visible_fragments(range);
    assert_eq!(
        fragments.len(),
        2,
        "the merge must span the frozen row and the scrolled band: {fragments:?}"
    );

    let ops = orch.grid_surface().recorder().ops();
    let paints = text_paints(&ops, "edge");
    let frozen = fragments[0].1;
    let scrolled = fragments[1].1;
    assert_eq!(
        shown_in(&paints, scrolled),
        1,
        "bottom-aligned text belongs at the merge's bottom edge, in the scrolled fragment; fragments: {fragments:?}"
    );
    assert_eq!(
        shown_in(&paints, frozen),
        0,
        "the frozen fragment shows no part of a bottom-aligned label; fragments: {fragments:?}"
    );
}

// ── Active-cell overlay on a merged cell ──

/// The overlay's active-cell restore must cover the **logical** cell, with the
/// grid's own text geometry. Restoring only the physical anchor would lay a
/// second, smaller label and interior grid edges over pixels the grid painted as
/// one merged cell.
#[test]
fn the_active_cell_overlay_restores_the_whole_merged_cell() {
    use iron_canvas_recorder::DrawOp;

    let range = RCRange {
        r1: 2,
        c1: 2,
        r2: 4,
        c2: 3,
    };
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(1)
            .with_merged_ranges(vec![range])
            // A covered cell is the active cell: the overlay must resolve it to
            // the merge, exactly as a click on it would.
            .with_active(3, 3),
    );
    model.set_cell(2, 2, "merged");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let fragments = orch.visible_fragments(range);
    assert_eq!(
        fragments.len(),
        1,
        "the merge must be visible: {fragments:?}"
    );
    let fragment = fragments[0].1;

    let ops = orch.overlay_surface().recorder().ops();
    assert!(
        ops.iter()
            .any(|op| matches!(op, DrawOp::RectFill { rect, .. } if *rect == fragment)),
        "the overlay's active-cell fill must cover the merged fragment {fragment:?}"
    );
    let paints = text_paints(&ops, "merged");
    assert_eq!(
        shown_in(&paints, fragment),
        1,
        "the overlay must show the merged label once, in the fragment: {paints:?}"
    );
}

/// The same restore when the merge's anchor is scrolled out of view: the
/// overlay resolves the covered cell to the merge from committed geometry, so it
/// still paints the visible fragment rather than nothing.
#[test]
fn the_active_cell_overlay_restores_an_offscreen_anchor_merge() {
    use iron_canvas_recorder::DrawOp;

    let range = RCRange {
        r1: 1,
        c1: 1,
        r2: 12,
        c2: 2,
    };
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(8)
            .with_merged_ranges(vec![range])
            .with_active(9, 1),
    );
    model.set_cell(1, 1, "offscreen");
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let fragments = orch.visible_fragments(range);
    assert!(!fragments.is_empty(), "part of the merge must be visible");
    let ops = orch.overlay_surface().recorder().ops();
    assert!(
        ops.iter()
            .any(|op| matches!(op, DrawOp::RectFill { rect, .. } if *rect == fragments[0].1)),
        "the overlay must restore the visible fragment {fragments:?}"
    );
    let paints = text_paints(&ops, "offscreen");
    assert_eq!(
        shown_in(&paints, fragments[0].1),
        1,
        "the offscreen anchor's label must be restored in the visible fragment: {paints:?}"
    );
}
