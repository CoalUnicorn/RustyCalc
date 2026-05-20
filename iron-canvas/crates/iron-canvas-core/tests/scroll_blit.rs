//! Stage 2.4 — recorder-driven proof that the scroll-blit fast-path
//! activates when it should and stays disabled when it shouldn't.
//!
//! These tests drive `RendererCore` directly (one `RecorderPainter`,
//! one `RendererCore` across both frames) so the cross-frame pane cache
//! survives between paints — that's the state the Stage 3.3a strip-fetch
//! path depends on.

use std::cell::Cell;

use ironcalc_base::types::{CellType, Style};

use iron_canvas_core::chrome::{ActiveCellSnapshot, Chrome, FramePath};
use iron_canvas_core::painter::BlitPainter;
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_recorder::{DrawOp, RecorderPainter};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CanvasModel, CanvasSize, CanvasView, RCRange};

/// Capture an `ActiveCellSnapshot` from the model's view. After B2
/// `screen_for_blit` takes the snapshot as a parameter; tests source it
/// here instead of reading it off `Chrome`.
fn snap(m: &ScrollModel) -> ActiveCellSnapshot {
    let view = m.get_selected_view().expect("scroll model has view");
    ActiveCellSnapshot::capture(m, m.get_selected_sheet(), view.row, view.column)
}

/// Scrollable model. `top_row` / `left_column` live in `Cell`s so a
/// single test can rebuild `Chrome::next` after a scroll without
/// rebuilding the model. `row5_height` lets a test mutate row 5's
/// height between frames to drive the overlap-row-height guard in
/// `screen_for_blit`. Cell content + styles stay default — the tests are
/// about paint-path activation, not visual fidelity.
struct ScrollModel {
    top_row: Cell<i32>,
    left_column: Cell<i32>,
    row5_height: Cell<f64>,
    /// Rows `1..=data_until` return non-empty formatted values
    /// (`"R{row}"`); rows past that return `""`. Default 0 = empty
    /// sheet (existing tests).
    data_until: Cell<i32>,
}

impl ScrollModel {
    fn new() -> Self {
        Self {
            top_row: Cell::new(1),
            left_column: Cell::new(1),
            row5_height: Cell::new(20.0),
            data_until: Cell::new(0),
        }
    }
    fn set_top_row(&self, row: i32) {
        self.top_row.set(row);
    }
    fn set_left_column(&self, col: i32) {
        self.left_column.set(col);
    }
    fn set_row5_height(&self, h: f64) {
        self.row5_height.set(h);
    }
    fn set_data_until(&self, row: i32) {
        self.data_until.set(row);
    }
}

impl CanvasModel for ScrollModel {
    fn get_selected_sheet(&self) -> u32 {
        0
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: 0,
            row: 1,
            column: 1,
            selection: RCRange::from([1, 1, 1, 1]),
            top_row: self.top_row.get(),
            left_column: self.left_column.get(),
        })
    }
    fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_row_height(&self, _: u32, row: i32) -> Option<f64> {
        Some(if row == 5 {
            self.row5_height.get()
        } else {
            20.0
        })
    }
    fn get_column_width(&self, _: u32, _: i32) -> Option<f64> {
        Some(80.0)
    }
    fn get_show_grid_lines(&self, _: u32) -> Option<bool> {
        Some(true)
    }
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Option<Style> {
        Some(Style::default())
    }
    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Option<CellType> {
        Some(CellType::Text)
    }
    fn get_formatted_cell_value(&self, _: u32, row: i32, _col: i32) -> Option<String> {
        if row >= 1 && row <= self.data_until.get() {
            Some(format!("R{row}"))
        } else {
            Some(String::new())
        }
    }
}

fn canvas() -> CanvasSize {
    CanvasSize { w: 600.0, h: 400.0 }
}

fn count_blits(ops: &[DrawOp]) -> usize {
    ops.iter()
        .filter(|op| matches!(op, DrawOp::Blit { .. }))
        .count()
}

fn issue_blits<P: BlitPainter>(painter: &P, plan: &iron_canvas_core::chrome::BlitPlan) {
    for s in &plan.shifts {
        painter.blit(s.src, s.dst);
    }
}

fn count_rect_fills(ops: &[DrawOp]) -> usize {
    ops.iter()
        .filter(|op| matches!(op, DrawOp::RectFill { .. }))
        .count()
}

#[test]
fn scroll_by_one_row_emits_exactly_one_blit_op() {
    let m = ScrollModel::new();
    let theme = CanvasTheme::light();
    let canvas = canvas();

    // Frame 0 at top_row=1.
    let frame0 = Chrome::next(None, &m, canvas, &theme, iron_canvas_core::chrome::FramePath::Fresh);
    let core = RendererCore::for_layer(RecorderPainter::new());
    core.render_grid(&m, &frame0);

    let baseline_ops = core.painter().ops().len();

    // Scroll by 1 row → top_row=2.
    m.set_top_row(2);

    let plan = frame0
        .screen_for_blit(&m, canvas, &theme, &snap(&m))
        .expect("single-row scroll must qualify for blit");

    // Simulate the orchestrator's blit fast-path on the same core so
    // the pane cache state carries across frames.
    let frame1 = Chrome::next(
        Some(frame0),
        &m,
        canvas,
        &theme,
        iron_canvas_core::chrome::FramePath::Blit(&plan),
    );
    issue_blits(core.painter(), &plan);
    core.render_grid_blit(&m, &frame1, &plan);

    let blit_phase_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect();

    assert_eq!(
        count_blits(&blit_phase_ops),
        1,
        "blit fast-path must emit exactly one DrawOp::Blit, got {:#?}",
        blit_phase_ops,
    );

    // The recorded Blit must match the plan's src/dst exactly — the
    // painter shouldn't be reinterpreting coordinates.
    let blit_op = blit_phase_ops
        .iter()
        .find(|op| matches!(op, DrawOp::Blit { .. }))
        .expect("blit op present per earlier assertion");
    let primary = match plan.shifts.first() {
        Some(s) => s,
        None => panic!("plan must carry at least one shift"),
    };
    match blit_op {
        DrawOp::Blit { src, dst } => {
            assert_eq!(*src, primary.src, "blit src must match plan");
            assert_eq!(*dst, primary.dst, "blit dst must match plan");
        }
        _ => unreachable!(),
    }
}

#[test]
fn scroll_past_viewport_disqualifies_blit() {
    let m = ScrollModel::new();
    let theme = CanvasTheme::light();
    let canvas = canvas();

    let frame0 = Chrome::next(None, &m, canvas, &theme, iron_canvas_core::chrome::FramePath::Fresh);

    // Canvas is 400 px tall, rows are 20 px → ~20 visible rows. Scroll
    // by 100 rows → no overlap with prev viewport → screen_for_blit must bail.
    m.set_top_row(101);

    let plan = frame0.screen_for_blit(&m, canvas, &theme, &snap(&m));
    assert!(
        plan.is_none(),
        "scroll past viewport extent must not qualify for blit",
    );
}

#[test]
fn scroll_by_one_column_emits_exactly_one_blit_op() {
    let m = ScrollModel::new();
    let theme = CanvasTheme::light();
    let canvas = canvas();

    let frame0 = Chrome::next(None, &m, canvas, &theme, iron_canvas_core::chrome::FramePath::Fresh);
    let core = RendererCore::for_layer(RecorderPainter::new());
    core.render_grid(&m, &frame0);
    let baseline_ops = core.painter().ops().len();

    // Pure horizontal scroll by 1 column.
    m.set_left_column(2);

    let plan = frame0
        .screen_for_blit(&m, canvas, &theme, &snap(&m))
        .expect("single-column scroll must qualify for blit");

    let frame1 = Chrome::next(Some(frame0), &m, canvas, &theme, FramePath::Blit(&plan));
    issue_blits(core.painter(), &plan);
    core.render_grid_blit(&m, &frame1, &plan);

    let blit_phase_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect();

    assert_eq!(
        count_blits(&blit_phase_ops),
        1,
        "column-scroll blit fast-path must emit exactly one DrawOp::Blit, got {:#?}",
        blit_phase_ops,
    );
}

/// Regression for the strip-fetch path: a 1-row scroll must paint only
/// the freshly-revealed strip, not the kept band. `apply_blit_shift`
/// rotates kept-band entries into their new pane indices (still `Some`),
/// so an unqualified full-pane walk would re-take them and emit cell-bg
/// rect_fills on top of pixels the painter blit already placed. The fix
/// narrows iteration to the strip via `PaneCells::for_strip`; this test
/// locks that contract in by comparing post-blit cell paint volume
/// against the full-pane baseline.
#[test]
fn scroll_by_one_row_paints_only_strip_cells() {
    let m = ScrollModel::new();
    let theme = CanvasTheme::light();
    let canvas = canvas();

    let frame0 = Chrome::next(None, &m, canvas, &theme, iron_canvas_core::chrome::FramePath::Fresh);
    let core = RendererCore::for_layer(RecorderPainter::new());
    core.render_grid(&m, &frame0);

    let baseline_ops: Vec<DrawOp> = core.painter().ops().iter().cloned().collect();
    let baseline_rect_fills = count_rect_fills(&baseline_ops);

    m.set_top_row(2);
    let plan = frame0
        .screen_for_blit(&m, canvas, &theme, &snap(&m))
        .expect("single-row scroll must qualify for blit");
    let frame1 = Chrome::next(Some(frame0), &m, canvas, &theme, FramePath::Blit(&plan));
    issue_blits(core.painter(), &plan);
    core.render_grid_blit(&m, &frame1, &plan);

    let blit_phase_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops.len())
        .cloned()
        .collect();
    let blit_phase_rect_fills = count_rect_fills(&blit_phase_ops);

    // Strip = 2 rows of cells (prev's overflow row + new's overflow row,
    // see `compute_strip` for why both) + 1 strip-bg fill + the row-header
    // strip (scroll-axis header always repaints) + corner box. Full-pane
    // repaint is O(visible_rows × visible_cols). With ~19 rows × 7 cols
    // visible, a buggy strip path that walks the full pane emits roughly
    // the same cell rect_fills as the baseline; the strip-only path emits
    // a small constant + headers. `×3 <` keeps catching a kept-band leak
    // (which would push past the full baseline) while tolerating the
    // 2-row strip shape.
    assert!(
        blit_phase_rect_fills * 3 < baseline_rect_fills,
        "1-row strip path emitted {} rect_fills; full-pane baseline was {}. \
         the strip path must not re-paint the kept band",
        blit_phase_rect_fills,
        baseline_rect_fills,
    );
}

/// Regression for the edit-then-scroll bug: typing into a cell and
/// pressing Enter scrolls by one row. If the consumer forgets to call
/// `markContentDirty`, the CONTENT-veto in `decide()` doesn't fire, so
/// the geometric `screen_for_blit` would otherwise succeed and the blit's kept
/// band would shift pre-edit pixels (the just-edited cell renders blank).
/// The defensive content check inside `screen_for_blit` must catch this by
/// re-hashing the prev frame's active-cell value against the live model.
#[test]
fn active_cell_value_change_disqualifies_blit() {
    let m = ScrollModel::new();
    let theme = CanvasTheme::light();
    let canvas = canvas();
    // Frame 0 paints with row 1 ("R1" coords) returning "".
    let frame0 = Chrome::next(None, &m, canvas, &theme, FramePath::Fresh);
    // Snapshot captured at paint time, as SelectionLayer would. The
    // defensive check in `screen_for_blit` compares this snapshot
    // against the live model on the *next* blit attempt.
    let active_at_paint = snap(&m);

    // Simulate the bug: row 1's value flips "" → "R1" (proxy for an edit
    // committed at the active cell) AND viewport scrolls by one row.
    m.set_data_until(5);
    m.set_top_row(2);

    let plan = frame0.screen_for_blit(&m, canvas, &theme, &active_at_paint);
    assert!(
        plan.is_none(),
        "edit-then-scroll must disqualify the blit when the active cell value changed",
    );
}

/// Contrapositive: a pure single-row scroll with the active-cell value
/// unchanged still qualifies for the blit fast path. Pins the defensive
/// check to mismatch-only behavior — it must not over-reject.
#[test]
fn active_cell_value_unchanged_allows_blit() {
    let m = ScrollModel::new();
    m.set_data_until(20); // row 1's value is "R1" in both frames.
    let theme = CanvasTheme::light();
    let canvas = canvas();
    let frame0 = Chrome::next(None, &m, canvas, &theme, FramePath::Fresh);

    m.set_top_row(2);

    let plan = frame0.screen_for_blit(&m, canvas, &theme, &snap(&m));
    assert!(
        plan.is_some(),
        "pure scroll with unchanged active-cell value must qualify for the blit",
    );
}

#[test]
fn overlap_row_height_change_disqualifies_blit() {
    let m = ScrollModel::new();
    let theme = CanvasTheme::light();
    let canvas = canvas();

    // Frame 0 sees row 5 at the default 20 px height.
    let frame0 = Chrome::next(None, &m, canvas, &theme, FramePath::Fresh);

    // Resize row 5 between frames AND scroll. Row 5 sits inside the
    // overlap band of a 1-row scroll, so `screen_for_blit`'s row-height guard
    // must fire and the fast-path must bail to a full repaint.
    m.set_row5_height(40.0);
    m.set_top_row(2);

    let plan = frame0.screen_for_blit(&m, canvas, &theme, &snap(&m));
    assert!(
        plan.is_none(),
        "row-height mutation inside the kept band must disqualify the blit",
    );
}

/// Regression for the smearing bug seen in the browser: data ends inside
/// the viewport (rows 1..=15 have data, 16+ empty), user scrolls by one
/// row. The strip is row 21 (newly revealed, empty); the kept band rows
/// 2..=20 had their pixels preserved by `Painter::blit`. The strip-fetch
/// path must not emit `FillText` ops for the kept band — doing so would
/// re-paint cells the blit already placed correctly, and (visually) drag
/// the last data row's text into rows below it.
///
/// Assertion: after the scroll-blit, no `FillText` op carries a `"R{n}"`
/// data-cell text. Strip cells are empty so they emit no text; kept-band
/// cells were preserved so they emit no text. The total post-scroll
/// FillText count for data-shaped strings must be zero.
#[test]
fn scroll_blit_does_not_smear_last_data_row_into_strip() {
    let m = ScrollModel::new();
    m.set_data_until(15);
    let theme = CanvasTheme::light();
    let canvas = canvas();

    let frame0 = Chrome::next(None, &m, canvas, &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(RecorderPainter::new());
    core.render_grid(&m, &frame0);
    let baseline_ops = core.painter().ops().len();

    m.set_top_row(2);
    let plan = frame0
        .screen_for_blit(&m, canvas, &theme, &snap(&m))
        .expect("single-row scroll must qualify for blit");

    let frame1 = Chrome::next(Some(frame0), &m, canvas, &theme, FramePath::Blit(&plan));
    issue_blits(core.painter(), &plan);
    core.render_grid_blit(&m, &frame1, &plan);

    let post_scroll_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect();

    let data_text_ops: Vec<&DrawOp> = post_scroll_ops
        .iter()
        .filter(|op| match op {
            DrawOp::FillText { text, .. } => text.starts_with('R'),
            _ => false,
        })
        .collect();

    assert!(
        data_text_ops.is_empty(),
        "scroll-blit must not re-paint kept-band cells; got {} FillText ops with data text: {:#?}",
        data_text_ops.len(),
        data_text_ops,
    );
}

/// Variant: 5-row scroll where data ends right at the last visible row
/// of the *initial* frame (canvas shows 20 rows, data_until = 20). After
/// the scroll the last data row is mid-viewport and strip rows 21..=25
/// reveal newly-visible empty cells. Mirrors the screenshot scenario:
/// last visible row had data pre-scroll, new strip below is empty.
#[test]
fn scroll_blit_does_not_smear_when_data_ends_at_initial_last_visible_row() {
    let m = ScrollModel::new();
    m.set_data_until(20);
    let theme = CanvasTheme::light();
    let canvas = canvas();

    let frame0 = Chrome::next(None, &m, canvas, &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(RecorderPainter::new());
    core.render_grid(&m, &frame0);
    let baseline_ops = core.painter().ops().len();

    m.set_top_row(6);
    let plan = frame0
        .screen_for_blit(&m, canvas, &theme, &snap(&m))
        .expect("5-row scroll must qualify for blit");

    let frame1 = Chrome::next(Some(frame0), &m, canvas, &theme, FramePath::Blit(&plan));
    issue_blits(core.painter(), &plan);
    core.render_grid_blit(&m, &frame1, &plan);

    let post_scroll_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect();

    // Non-aligned axis: (400 − 28) / 20 = 18.6, so prev had two transition
    // rows. R20 was the overflow (top past canvas bottom) and becomes fully
    // visible in new — never blitted, must repaint. R19 was prev's partial
    // (12 px visible) and is partial again in new at a different fraction
    // (8 px overlap the strip clip), so its bottom band needs new pixels
    // too. Any *other* R-text is a kept-band smear.
    let smeared_text_ops: Vec<&DrawOp> = post_scroll_ops
        .iter()
        .filter(|op| match op {
            DrawOp::FillText { text, .. } => {
                text.starts_with('R') && text != "R19" && text != "R20"
            }
            _ => false,
        })
        .collect();

    assert!(
        smeared_text_ops.is_empty(),
        "5-row scroll-blit must not re-paint kept-band data cells; got {} ops: {:#?}",
        smeared_text_ops.len(),
        smeared_text_ops,
    );
}
