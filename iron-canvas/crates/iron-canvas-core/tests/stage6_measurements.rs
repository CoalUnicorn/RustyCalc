//! Stage 6, Task 1 — the fixed-workload traffic matrix.
//!
//! This binary changes no production behaviour. It drives the real
//! `Orchestrator<MemSurface>` through the eleven workloads (W0-W10) of the
//! Stage 6 plan across the full size x frozen x style matrix, and emits one
//! stable CSV row per measured frame for
//! `docs/performance/2026-08-02-stage-6-render-costs.md` to quote.
//!
//! Three quantities are deliberately kept in separate columns, because
//! conflating them is the exact mistake Stage 6 exists to avoid:
//!
//! - `fetched_slots` — `FrameTrace.fetched_cell_slots`: *logical* cell slots
//!   handed to the four bulk content accessors, summed per call. It counts
//!   neither host crossings nor elapsed time; a 1,000-cell pane reads 4,000
//!   whatever verdict the fingerprint planner later reaches.
//! - `bulk_calls` / `observed_cells` / `fetch_ranges` — recorded by
//!   [`ObservedModel`], the test-local wrapper that overrides only the four
//!   bulk methods to note channel, range and cell count before delegating to
//!   the real `TestModel`. This is the actual *fetch shape* the host sees, and
//!   the only reason the wrapper exists: `FrameTrace` alone cannot tell one
//!   1,000-cell fetch from four 250-cell ones.
//! - the grouped `DrawOp` columns — painter traffic, from the `MemSurface`
//!   recorder.
//!
//! W5 is Stage 6's phase-attribution control, and Task 6 re-shaped what it has
//! to control for. Before rotation, a row blit always left the fingerprint tree
//! stale, so the first post-blit content check was always the reseeding `Full`
//! and the second was the `Skip` that reseed enabled — one sequence, two
//! verdicts. Rotation removed that reseed frame: a qualifying row blit now
//! installs a complete tree, so that same sequence Skips twice and the delta
//! vanishes from the run.
//!
//! Task 7 restored the control by naming the two *states* instead of two
//! positions in a sequence — `post_blit_stale_full` (rotation unavailable) and
//! `post_blit_rotated_skip` (rotation applied) — and forcing the unavailable
//! side with the one production mechanism that legitimately produces it: a
//! Damage strip marks history stale, so the blit that follows may not rotate.
//! Both arms still perform the same full-pane fetch, candidate build, shell,
//! header and presentation work and differ only in the cell painter, which is
//! what the control was always for.
//!
//! Timings are not measured here at all — end-to-end elapsed time is the
//! browser probe's job (`iron-canvas-web/tests/render_wasm.rs`), and the
//! private fingerprint A/B lives in `renderer/cell/fingerprint.rs`.

mod common;

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;

use iron_canvas_core::chrome::PaneRegionMask;
use iron_canvas_core::geometry::constants::{CELL_AREA_INSET, FROZEN_SEP};
use iron_canvas_core::{
    Border, BorderItem, BorderStyle, CanvasModel, CanvasSize, CanvasTheme, CanvasView,
    CellContentQuery, CellDecoration, CellKind, CellStyle, DataBarSpec, Fetched, FrameOutcome,
    FrameTrace, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT, Orchestrator, PaintResult, PaneVerdict,
    RCRange, RowSpan,
};
use iron_canvas_recorder::{DrawOp, MemSurface};

use common::TestModel;

// ==============================================================================
// The observed model wrapper
// ==============================================================================
//
// Delegates every scalar and frame method verbatim; overrides only the four
// bulk accessors, and only to record `(channel, range, cells)` before handing
// the call straight to the wrapped `TestModel`. Nothing about fetch semantics,
// buffer layout or `Fetched` propagation changes — the recorded call list is
// pure observation.

#[derive(Clone, Copy, PartialEq, Eq)]
enum BulkChannel {
    Styles,
    Values,
    Types,
    Decorations,
}

impl BulkChannel {
    fn tag(self) -> &'static str {
        match self {
            Self::Styles => "styles",
            Self::Values => "values",
            Self::Types => "types",
            Self::Decorations => "decos",
        }
    }
}

#[derive(Clone, Copy)]
struct BulkCall {
    channel: BulkChannel,
    range: RCRange,
    cells: usize,
}

fn cells_in(range: RCRange) -> usize {
    let rows = (range.r2 - range.r1 + 1).max(0) as usize;
    let cols = (range.c2 - range.c1 + 1).max(0) as usize;
    rows * cols
}

struct ObservedModel {
    inner: TestModel,
    calls: RefCell<Vec<BulkCall>>,
}

impl ObservedModel {
    fn new(inner: TestModel) -> Self {
        Self {
            inner,
            calls: RefCell::new(Vec::new()),
        }
    }

    fn model(&self) -> &TestModel {
        &self.inner
    }

    fn clear_calls(&self) {
        self.calls.borrow_mut().clear();
    }

    fn calls(&self) -> Vec<BulkCall> {
        self.calls.borrow().clone()
    }

    fn record(&self, channel: BulkChannel, range: RCRange) {
        self.calls.borrow_mut().push(BulkCall {
            channel,
            range,
            cells: cells_in(range),
        });
    }
}

/// Forwarding bodies for the methods this wrapper only passes through. Mirrors
/// `model_adapter`'s own private `forward_methods!`, which is not exported.
macro_rules! delegate_to_inner {
    ($(fn $name:ident(&self $(, $arg:ident: $argty:ty)*) $(-> $ret:ty)?;)*) => {
        $(
            fn $name(&self, $($arg: $argty),*) $(-> $ret)? {
                self.inner.$name($($arg),*)
            }
        )*
    };
}

impl CellContentQuery for ObservedModel {
    delegate_to_inner! {
        fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellStyle>;
        fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellKind>;
        fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Fetched<String>;
        fn get_extended_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellDecoration>;
    }

    fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellStyle>>) {
        self.record(BulkChannel::Styles, range);
        self.inner.get_cell_styles_in(sheet, range, out);
    }
    fn get_formatted_cell_values_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Fetched<String>>,
    ) {
        self.record(BulkChannel::Values, range);
        self.inner.get_formatted_cell_values_in(sheet, range, out);
    }
    fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellKind>>) {
        self.record(BulkChannel::Types, range);
        self.inner.get_cell_types_in(sheet, range, out);
    }
    fn get_cell_decorations_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Fetched<CellDecoration>>,
    ) {
        self.record(BulkChannel::Decorations, range);
        self.inner.get_cell_decorations_in(sheet, range, out);
    }
}

impl CanvasModel for ObservedModel {
    delegate_to_inner! {
        fn get_selected_sheet(&self) -> Option<u32>;
        fn get_selected_view(&self) -> Option<CanvasView>;
        fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32>;
        fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32>;
        fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64>;
        fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64>;
        fn get_show_grid_lines(&self, sheet: u32) -> Option<bool>;
        fn get_show_selection(&self) -> bool;
        fn last_row(&self, sheet: u32) -> i32;
        fn last_column(&self, sheet: u32) -> i32;
        fn get_show_row_headers(&self, sheet: u32) -> Option<bool>;
        fn get_show_col_headers(&self, sheet: u32) -> Option<bool>;
        fn get_row_header_text(&self, sheet: u32, row: i32) -> Option<String>;
        fn get_column_header_text(&self, sheet: u32, col: i32) -> Option<String>;
    }
}

// ==============================================================================
// Painter traffic: stable DrawOp groups
// ==============================================================================
//
// Grouped rather than per-variant so the report survives a future painter
// primitive being added or split. `total` is the raw op count, kept only as a
// secondary cross-check on the groups.

#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct OpCounts {
    fills: usize,
    strokes: usize,
    text: usize,
    clips: usize,
    blits: usize,
    invalidations: usize,
    text_resets: usize,
    dpr_transforms: usize,
    group_brackets: usize,
    total: usize,
}

impl OpCounts {
    fn of(ops: &[DrawOp]) -> Self {
        let mut counts = Self {
            total: ops.len(),
            ..Self::default()
        };
        for op in ops {
            match op {
                DrawOp::RectFill { .. } | DrawOp::FillPath { .. } | DrawOp::ClearRect { .. } => {
                    counts.fills += 1;
                }
                DrawOp::RectStroke { .. }
                | DrawOp::RectDashed { .. }
                | DrawOp::StrokeLine { .. }
                | DrawOp::StrokeHLine { .. }
                | DrawOp::StrokeVLine { .. }
                | DrawOp::StrokeTextHLine { .. } => counts.strokes += 1,
                DrawOp::FillText { .. } => counts.text += 1,
                DrawOp::PushClip { .. } | DrawOp::PopClip => counts.clips += 1,
                DrawOp::Blit { .. } => counts.blits += 1,
                DrawOp::InvalidateCache => counts.invalidations += 1,
                DrawOp::ResetTextDefaults => counts.text_resets += 1,
                DrawOp::ApplyDprTransform { .. } => counts.dpr_transforms += 1,
                DrawOp::BeginGroup { .. } | DrawOp::EndGroup => counts.group_brackets += 1,
            }
        }
        counts
    }

    /// Cell-painter traffic proper: the groups a fingerprint `Skip` is
    /// supposed to avoid. Excludes the state ops (`InvalidateCache`,
    /// `ResetTextDefaults`, DPR transforms) and the blit itself, which a Skip
    /// frame still pays.
    fn drawing(self) -> usize {
        self.fills + self.strokes + self.text + self.clips + self.group_brackets
    }
}

// ==============================================================================
// The workload matrix
// ==============================================================================

/// Fixed slot metrics for every shape. `synthetic_grid`'s 20 x 80 grid, so a
/// target pane size maps to an exact canvas size.
const ROW_H: f64 = 20.0;
const COL_W: f64 = 80.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SizeKind {
    /// The production-shaped 29 x 21 / 609-cell pane of the Stage 6 plan, read
    /// as 29 rows x 21 columns to match the fingerprint bench's ROWS x COLS
    /// convention.
    Production,
    /// The plan's 50 x 20 / 1,000-cell stress pane.
    Stress,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrozenKind {
    None,
    TwoByTwo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StyleKind {
    Plain,
    /// Explicit borders plus CF decorations, banded every fifth row.
    Styled,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Shape {
    size: SizeKind,
    frozen: FrozenKind,
    style: StyleKind,
}

impl Shape {
    fn all() -> Vec<Self> {
        let mut shapes = Vec::with_capacity(8);
        for size in [SizeKind::Production, SizeKind::Stress] {
            for frozen in [FrozenKind::None, FrozenKind::TwoByTwo] {
                for style in [StyleKind::Plain, StyleKind::Styled] {
                    shapes.push(Self {
                        size,
                        frozen,
                        style,
                    });
                }
            }
        }
        shapes
    }

    fn production_plain() -> Self {
        Self {
            size: SizeKind::Production,
            frozen: FrozenKind::None,
            style: StyleKind::Plain,
        }
    }

    fn rows(self) -> i32 {
        match self.size {
            SizeKind::Production => 29,
            SizeKind::Stress => 50,
        }
    }

    fn cols(self) -> i32 {
        match self.size {
            SizeKind::Production => 21,
            SizeKind::Stress => 20,
        }
    }

    fn frozen_count(self) -> i32 {
        match self.frozen {
            FrozenKind::None => 0,
            FrozenKind::TwoByTwo => 2,
        }
    }

    /// The scroll band's first legal origin. A frozen sheet clamps any origin
    /// inside the frozen run up to this, so scrolling *from* it is what
    /// `Chrome::classify` sees as movement — asking for row 2 on a 2-row
    /// frozen sheet is no movement at all.
    fn scroll_origin(self) -> i32 {
        self.frozen_count() + 1
    }

    /// One slot past `scroll_origin`: the single-step scroll W4/W5/W6/W7/W9
    /// perform, and W10's column-axis twin.
    fn scrolled_origin(self) -> i32 {
        self.scroll_origin() + 1
    }

    /// Canvas that shows exactly `rows` x `cols` slots: headers, the cell-area
    /// inset, the slot run, and one frozen separator per axis when frozen.
    ///
    /// The run is one slot short on each axis because the visible range always
    /// admits the partially visible trailing slot: `n - 1` full slots plus that
    /// partial one is what actually gets fetched and painted. The emitted
    /// `fetch_ranges` column is the authority on the achieved shape.
    fn canvas(self) -> CanvasSize {
        let sep = if self.frozen_count() > 0 {
            f64::from(FROZEN_SEP)
        } else {
            0.0
        };
        let cols_run = f64::from(self.cols() - 1) * COL_W;
        let rows_run = f64::from(self.rows() - 1) * ROW_H;
        CanvasSize {
            w: f64::from(HEADER_COL_WIDTH) + f64::from(CELL_AREA_INSET) + cols_run + sep,
            h: f64::from(HEADER_ROW_HEIGHT) + f64::from(CELL_AREA_INSET) + rows_run + sep,
        }
    }

    fn size_tag(self) -> &'static str {
        match self.size {
            SizeKind::Production => "prod29x21",
            SizeKind::Stress => "stress50x20",
        }
    }

    fn frozen_tag(self) -> &'static str {
        match self.frozen {
            FrozenKind::None => "unfrozen",
            FrozenKind::TwoByTwo => "frozen2x2",
        }
    }

    fn style_tag(self) -> &'static str {
        match self.style {
            StyleKind::Plain => "plain",
            StyleKind::Styled => "styled",
        }
    }
}

/// The eleven fixed sequences of the Stage 6 plan's workload matrix.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Workload {
    /// Initial Fresh. Cold control — never a ratio denominator.
    W0,
    /// Overlay-only selection move: zero-grid control.
    W1,
    /// One row-addressed content change: the Damage fetch/painter baseline.
    W2,
    /// The same cell edit routed through unrowed (pane-scoped) content work,
    /// so W2/W3 give the Damage-to-SlotsReuse ratio directly.
    W3,
    /// A qualifying one-axis row scroll: the Viewport strip baseline.
    W4,
    /// W4, then two unchanged full-pane content notifications: the
    /// fetch-equivalent post-blit Full/Skip pair.
    W5,
    /// W4, then a borderless overlapping-row edit: the real post-scroll edit.
    W6,
    /// A row scroll whose visible-row extent changes by one, so
    /// `shift_is_safe` rejects the shift as `IncompatibleRange`.
    W7,
    /// Theme change then Fresh: the duplicate-invalidation count.
    W8,
    /// Content plus a real row scroll in one attempt: conservative Fresh.
    /// W9 changes `top_row`, so it is not a stable-geometry commit sample.
    W9,
    /// Horizontal Viewport scroll: the column-axis control.
    W10,
}

impl Workload {
    fn all() -> [Self; 11] {
        [
            Self::W0,
            Self::W1,
            Self::W2,
            Self::W3,
            Self::W4,
            Self::W5,
            Self::W6,
            Self::W7,
            Self::W8,
            Self::W9,
            Self::W10,
        ]
    }

    fn id(self) -> &'static str {
        match self {
            Self::W0 => "W0",
            Self::W1 => "W1",
            Self::W2 => "W2",
            Self::W3 => "W3",
            Self::W4 => "W4",
            Self::W5 => "W5",
            Self::W6 => "W6",
            Self::W7 => "W7",
            Self::W8 => "W8",
            Self::W9 => "W9",
            Self::W10 => "W10",
        }
    }
}

// ==============================================================================
// One measured frame
// ==============================================================================

struct Sample {
    result: PaintResult,
    trace: FrameTrace,
    ops: OpCounts,
    calls: Vec<BulkCall>,
}

impl Sample {
    fn observed_cells(&self) -> usize {
        self.calls.iter().map(|call| call.cells).sum()
    }

    /// `channel:r1-r2xc1-c2@cells`, `|`-joined — CSV-safe (no commas) and
    /// still precise enough to tell one whole-pane fetch from four strips.
    fn fetch_ranges(&self) -> String {
        let mut out = String::new();
        for call in &self.calls {
            if !out.is_empty() {
                out.push('|');
            }
            let range = call.range;
            let _ = write!(
                out,
                "{}:{}-{}x{}-{}@{}",
                call.channel.tag(),
                range.r1,
                range.r2,
                range.c1,
                range.c2,
                call.cells
            );
        }
        out
    }
}

/// Live `Orchestrator` plus the observed model behind it. One probe per
/// workload run: nothing leaks between workloads.
struct Probe {
    model: Rc<ObservedModel>,
    orch: Orchestrator<MemSurface>,
    cursor: usize,
}

impl Probe {
    fn new(shape: Shape) -> Self {
        let frozen = shape.frozen_count();
        let inner = TestModel::synthetic_grid()
            .with_frozen(frozen, frozen)
            .with_data_until(100_000);
        if shape.style == StyleKind::Styled {
            decorate(&inner, shape);
        }
        let model = Rc::new(ObservedModel::new(inner));

        let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
        orch.resize(shape.canvas(), 1.0);
        orch.set_model(Rc::clone(&model) as Rc<dyn CanvasModel>);
        // Construction is not part of any workload, so the first measured
        // window opens after `resize`/`set_model`, not at op zero.
        let cursor = orch.grid_surface().recorder().ops().len();
        Self {
            model,
            orch,
            cursor,
        }
    }

    fn model(&self) -> &TestModel {
        self.model.model()
    }

    /// Paint one attempt and collect everything about it.
    ///
    /// The counted window opens where the previous paint (or construction)
    /// closed it, not at entry: setters a workload invokes between paints emit
    /// painter operations eagerly — `Orchestrator::set_theme` invalidates the
    /// grid paint cache before anything is drawn — and those belong to the
    /// attempt they precede.
    fn paint(&mut self) -> Sample {
        self.model.clear_calls();
        let result = self.orch.paint_if_dirty();
        let ops = {
            let recorded = self.orch.grid_surface().recorder().ops();
            let counted = OpCounts::of(&recorded[self.cursor..]);
            self.cursor = recorded.len();
            counted
        };
        Sample {
            result,
            trace: self.orch.last_trace(),
            ops,
            calls: self.model.calls(),
        }
    }

    /// The cold Fresh paint every workload but W0 starts from, discarded.
    fn warm(&mut self) {
        let _ = self.paint();
    }
}

/// Explicit borders (bottom edge, every fifth row) and CF data bars, the
/// "styled sheet containing explicit borders and CF decorations" half of the
/// matrix. Banded rather than uniform so `has_any_explicit_border` is true for
/// some rows and false for others — the state the row-band safety check reads.
fn decorate(model: &TestModel, shape: Shape) {
    let rows = shape.rows() + 8;
    let cols = shape.cols() + 4;
    for row in 1..=rows {
        for col in 1..=cols {
            if row % 5 == 0 {
                model.set_style(
                    row,
                    col,
                    CellStyle {
                        border: Border {
                            bottom: Some(BorderItem {
                                style: BorderStyle::Thin,
                                color: None,
                            }),
                            ..Border::default()
                        },
                        ..CellStyle::default()
                    },
                );
            }
            if col % 4 == 0 {
                model.set_decoration(
                    row,
                    col,
                    CellDecoration::DataBar(DataBarSpec {
                        fraction: f64::from(row % 10) / 10.0,
                        color: "#3366cc".to_string(),
                    }),
                );
            }
        }
    }
}

/// A row inside the scroll pane that carries no explicit border in either
/// style variant — the "borderless overlapping row" W6 needs. `top_row` is
/// the scroll band's current origin, not the frozen band's.
fn borderless_scroll_row(top_row: i32) -> i32 {
    let mut row = top_row + 3;
    while row % 5 == 0 {
        row += 1;
    }
    row
}

// ==============================================================================
// Workload sequences
// ==============================================================================

struct Row {
    workload: &'static str,
    phase: &'static str,
    shape: Shape,
    sample: Sample,
}

fn run_workload(workload: Workload, shape: Shape) -> Vec<Row> {
    let mut probe = Probe::new(shape);
    let frozen = shape.frozen_count();
    let sheet = 0;
    // Every content edit lands on a column past the frozen band, so the frozen
    // panes are not accidentally the only ones touched.
    let edit_col = frozen + 2;

    let mut rows = Vec::new();
    let mut push = |phase: &'static str, sample: Sample| {
        rows.push(Row {
            workload: workload.id(),
            phase,
            shape,
            sample,
        });
    };

    match workload {
        Workload::W0 => {
            push("cold_fresh", probe.paint());
        }
        Workload::W1 => {
            probe.warm();
            probe.model().set_active(frozen + 3, frozen + 3);
            probe.orch.view_changed();
            push("selection_move", probe.paint());
        }
        Workload::W2 => {
            probe.warm();
            let row = borderless_scroll_row(shape.scroll_origin());
            probe.model().set_cell(row, edit_col, "edited");
            probe
                .orch
                .mark_rows_damaged(sheet, RowSpan { r1: row, r2: row });
            push("row_damage", probe.paint());
        }
        Workload::W3 => {
            probe.warm();
            let row = borderless_scroll_row(shape.scroll_origin());
            probe.model().set_cell(row, edit_col, "edited");
            probe.orch.mark_content_dirty(PaneRegionMask::ALL);
            push("pane_content", probe.paint());
        }
        Workload::W4 => {
            probe.warm();
            probe.model().set_top_row(shape.scrolled_origin());
            probe.orch.view_changed();
            push("row_scroll", probe.paint());
        }
        Workload::W5 => {
            // The phase-attribution control, re-shaped for Task 7. Both frames
            // below are the SAME interaction — an unchanged-content
            // notification arriving straight after a qualifying row blit — and
            // both therefore pay the identical full-pane fetch. What differs is
            // whether the blit was able to carry its fingerprint history across
            // the shift, which is the only thing Tasks 5-7 changed.
            //
            // The `stale` arm forces "could not" with the one production
            // mechanism that legitimately produces it: a Damage strip commits
            // `MarkStale` unconditionally, so the following blit finds history
            // it may not rotate and the content check falls back to the
            // five-pass walk. That is what EVERY post-blit content check cost
            // before Task 6, and what a Damage-then-scroll sequence still costs
            // today. The `rotated` arm is the same sequence without the Damage
            // strip, which is now the common case.
            probe.warm();
            let damaged = borderless_scroll_row(shape.scroll_origin());

            probe.orch.mark_rows_damaged(
                sheet,
                RowSpan {
                    r1: damaged,
                    r2: damaged,
                },
            );
            probe.warm();
            probe.model().set_top_row(shape.scrolled_origin());
            probe.orch.view_changed();
            probe.warm();
            probe.orch.mark_content_dirty(PaneRegionMask::ALL);
            push("post_blit_stale_full", probe.paint());

            // That Full reseeded exact history, so scrolling home and back out
            // again gives the second arm a blit that may rotate.
            probe.model().set_top_row(shape.scroll_origin());
            probe.orch.view_changed();
            probe.warm();
            probe.model().set_top_row(shape.scrolled_origin());
            probe.orch.view_changed();
            probe.warm();
            probe.orch.mark_content_dirty(PaneRegionMask::ALL);
            push("post_blit_rotated_skip", probe.paint());
        }
        Workload::W6 => {
            probe.warm();
            probe.model().set_top_row(shape.scrolled_origin());
            probe.orch.view_changed();
            probe.warm();
            let row = borderless_scroll_row(shape.scrolled_origin());
            probe.model().set_cell(row, edit_col, "edited");
            probe.orch.mark_content_dirty(PaneRegionMask::ALL);
            push("post_blit_edit", probe.paint());
        }
        Workload::W7 => {
            // The scroll band's first row is taller than the rest, so scrolling
            // it out of view frees enough pixels for one extra row: the
            // scroll-axis extent changes by one and `shift_is_safe` must reject
            // the shift as `IncompatibleRange`.
            probe
                .model()
                .set_row_height(shape.scroll_origin(), ROW_H * 2.0 + 5.0);
            probe.warm();
            probe.model().set_top_row(shape.scrolled_origin());
            probe.orch.view_changed();
            push("edge_row_extent", probe.paint());
        }
        Workload::W8 => {
            probe.warm();
            probe.orch.set_theme(CanvasTheme::dark());
            push("theme_fresh", probe.paint());
        }
        Workload::W9 => {
            probe.warm();
            let row = borderless_scroll_row(shape.scroll_origin());
            probe.model().set_cell(row, edit_col, "edited");
            probe.orch.mark_content_dirty(PaneRegionMask::ALL);
            probe.model().set_top_row(shape.scrolled_origin());
            probe.orch.view_changed();
            push("content_plus_view", probe.paint());
        }
        Workload::W10 => {
            probe.warm();
            probe.model().set_left_column(shape.scrolled_origin());
            probe.orch.view_changed();
            push("column_scroll", probe.paint());
        }
    }
    rows
}

// ==============================================================================
// CSV emission
// ==============================================================================

const CSV_HEADER: &str = "workload,phase,size,frozen,style,result,regime,effective,work,outcome,\
tl,tr,bl,br,blit_fallback,fetched_slots,bulk_calls,observed_cells,fetch_ranges,\
fills,strokes,text,clips,blits,invalidations,text_resets,dpr_transforms,group_brackets,ops_total,\
drawing_ops";

fn verdict_tag(verdict: Option<PaneVerdict>) -> String {
    match verdict {
        Some(v) => v.to_string(),
        None => "-".to_string(),
    }
}

fn outcome_tag(outcome: FrameOutcome) -> String {
    match outcome {
        FrameOutcome::Painted => "painted".to_string(),
        FrameOutcome::PartialCommit(mask) => format!("partial({mask:?})"),
        FrameOutcome::HeldOnBridgeFailure(pane) => format!("held_bridge({pane:?})"),
        FrameOutcome::HeldOnInputFailure(failure) => format!("held_input({failure:?})"),
    }
}

fn fallback_tag(trace: &FrameTrace) -> String {
    match trace.blit_fallback {
        Some(fb) => {
            let why = if fb.cold_cache { "cold" } else { "range" };
            format!("{:?}/{why}", fb.pane)
        }
        None => "-".to_string(),
    }
}

fn csv_row(row: &Row) -> String {
    let Row {
        workload,
        phase,
        shape,
        sample,
    } = row;
    let trace = &sample.trace;
    let ops = sample.ops;
    format!(
        "{workload},{phase},{size},{frozen},{style},{result:?},{regime},{effective},{work},\
{outcome},{tl},{tr},{bl},{br},{fallback},{fetched},{bulk_calls},{observed_cells},{ranges},\
{fills},{strokes},{text},{clips},{blits},{invalidations},{resets},{dpr},{groups},{total},{drawing}",
        size = shape.size_tag(),
        frozen = shape.frozen_tag(),
        style = shape.style_tag(),
        result = sample.result,
        regime = trace
            .regime
            .map_or_else(|| "-".to_string(), |r| format!("{r:?}")),
        effective = trace
            .effective
            .map_or_else(|| "-".to_string(), |r| format!("{r:?}")),
        work = format!("{:?}", trace.work).replace(',', ";"),
        outcome = outcome_tag(trace.outcome),
        tl = verdict_tag(trace.panes[0]),
        tr = verdict_tag(trace.panes[1]),
        bl = verdict_tag(trace.panes[2]),
        br = verdict_tag(trace.panes[3]),
        fallback = fallback_tag(trace),
        fetched = trace.fetched_cell_slots,
        bulk_calls = sample.calls.len(),
        observed_cells = sample.observed_cells(),
        ranges = sample.fetch_ranges(),
        fills = ops.fills,
        strokes = ops.strokes,
        text = ops.text,
        clips = ops.clips,
        blits = ops.blits,
        invalidations = ops.invalidations,
        resets = ops.text_resets,
        dpr = ops.dpr_transforms,
        groups = ops.group_brackets,
        total = ops.total,
        drawing = ops.drawing(),
    )
}

/// Stage 6 Task 2 runs this and pastes the CSV block into the report. Ignored:
/// it is an evidence artifact, not an assertion, and its only meaningful form
/// is a release build (`--release --ignored --nocapture --test-threads=1`).
#[test]
#[ignore = "Stage 6 manual measurement probe: emits the W0-W10 traffic CSV; run with --release --ignored --nocapture --test-threads=1"]
fn stage6_emit_traffic_matrix_csv() {
    println!("# stage6-traffic-matrix v1");
    println!("{CSV_HEADER}");
    for shape in Shape::all() {
        for workload in Workload::all() {
            for row in run_workload(workload, shape) {
                println!("{}", csv_row(&row));
            }
        }
    }
}

/// Task 1's acceptance criterion 2, restated against the Task 7 pair: the two
/// W5 arms must pay **equal fetch traffic** and **unequal painter traffic**.
///
/// Equal fetch is the structural half, and Task 6 did not change it — a
/// fingerprint verdict never moves model traffic, because
/// `FetchedCells::fetch_into` runs before the candidate is built.
///
/// Unequal painter traffic is what the arms are named for, and it is the native
/// counterpart of Gate C's browser millisecond: the arm whose blit could not
/// carry its history walks all five passes over the pane, and the arm whose
/// blit could does not. Asserting the *direction* (stale strictly greater)
/// rather than an exact op count is deliberate — the count is a measurement the
/// CSV reports, while the ordering is the invariant rotation exists to create.
/// A regression that stopped rotating, or one that started certifying a Damage
/// strip's history, collapses this inequality in one direction or the other.
#[test]
fn w5_post_blit_arms_pay_equal_fetch_and_unequal_painter_traffic() {
    let rows = run_workload(Workload::W5, Shape::production_plain());
    let [stale, rotated] = rows.as_slice() else {
        panic!("W5 must produce exactly the two post-blit content frames");
    };

    assert_eq!(stale.phase, "post_blit_stale_full");
    assert_eq!(rotated.phase, "post_blit_rotated_skip");

    assert_eq!(
        stale.sample.trace.fetched_cell_slots, rotated.sample.trace.fetched_cell_slots,
        "both arms of W5 fetch the same logical cell slots — a fingerprint Skip \
         avoids the painter, never the fetch"
    );
    assert_eq!(
        stale.sample.observed_cells(),
        rotated.sample.observed_cells(),
        "the observed host fetch shape must match too, not just the logical slot sum"
    );

    assert_eq!(
        stale.sample.trace.panes,
        [None, None, None, Some(PaneVerdict::Full)],
        "the stale arm's blit must have refused to rotate, leaving the content check \
         a whole-pane repaint"
    );
    assert_eq!(
        rotated.sample.trace.panes,
        [None, None, None, Some(PaneVerdict::Skip)],
        "the rotated arm's blit must have installed history the content check can match"
    );
    assert!(
        stale.sample.ops.drawing() > rotated.sample.ops.drawing(),
        "rotation must remove painter work at identical fetch traffic: \
         stale={} rotated={}",
        stale.sample.ops.drawing(),
        rotated.sample.ops.drawing()
    );
}
