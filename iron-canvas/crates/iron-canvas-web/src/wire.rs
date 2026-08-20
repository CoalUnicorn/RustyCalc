//! JS-facing wire shapes for the `IronCanvas` API — both directions.
//!
//! The engine enums in `iron-canvas-core` use tuple variants for their
//! ergonomic in-Rust shape (`RowHeader(i32)`, `Edge(Side)`,
//! `ResizeTarget::Column(i32)`). Serde's internal tagging (`tag = "kind"`)
//! rejects tuple variants, and the engine crates are deliberately kept
//! free of `wasm-bindgen` / `serde-wasm-bindgen` concerns. So the JS shapes
//! are materialized here:
//!
//! - Outbound (query API): struct-formed mirrors that derive `Serialize`,
//!   plus `From<Engine>` impls (`HitTestWire`, `ResizeTargetWire`, ...).
//! - Inbound (setter API): `Deserialize`-only shapes with `From<Wire> for
//!   Engine` impls — overlay setter inputs and theme setter inputs.

// The query/setter shapes above are consumed only by the wasm32 JS bridge.
// On host dev-tools builds — where this module is compiled for the native
// wire-shape test below — they are intentionally unused, so the dead-code
// lint is relaxed only off-wasm32 (the wasm32 build still catches genuinely
// orphaned shapes).
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde::{Deserialize, Serialize};

use std::borrow::Cow;

use iron_canvas_core::{
    AutofillTarget, CanvasTheme, FormulaRef, FormulaRefKind, HitTest, RCRange, RectCorner, RefZone,
    RenderOverlays, ResizeTarget, SheetArea, Side, ThemeVariables, geometry::CanvasSize,
};

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum HitTestWire {
    Cell {
        row: i32,
        column: i32,
    },
    RowHeader {
        row: i32,
    },
    ColumnHeader {
        column: i32,
    },
    Corner,
    AutofillHandle {
        row: i32,
        column: i32,
    },
    FormulaRef {
        ref_idx: usize,
        zone: RefZoneWire,
        grab_row: i32,
        grab_column: i32,
    },
    Outside,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum RefZoneWire {
    Body,
    Edge { side: SideWire },
    Corner { corner: CornerWire },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SideWire {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CornerWire {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum ResizeTargetWire {
    Row { row: i32 },
    Column { column: i32 },
}

/// Mirror of `CanvasSize`. Kept here so the engine type stays serde-free
/// (the `.icr` schema inlines `canvas_w`/`canvas_h` for the same reason).
#[derive(Serialize)]
pub(crate) struct CanvasSizeWire {
    pub w: f64,
    pub h: f64,
}

/// `Option<(row, col)>` from `pixel_to_cell` becomes a `{row, column}`
/// object when present, matching the rest of the API (named fields, not
/// positional tuples).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CellCoordWire {
    pub row: i32,
    pub column: i32,
}

impl From<Side> for SideWire {
    fn from(s: Side) -> Self {
        match s {
            Side::Top => SideWire::Top,
            Side::Right => SideWire::Right,
            Side::Bottom => SideWire::Bottom,
            Side::Left => SideWire::Left,
        }
    }
}

impl From<RectCorner> for CornerWire {
    fn from(c: RectCorner) -> Self {
        match c {
            RectCorner::TopLeft => CornerWire::TopLeft,
            RectCorner::TopRight => CornerWire::TopRight,
            RectCorner::BottomLeft => CornerWire::BottomLeft,
            RectCorner::BottomRight => CornerWire::BottomRight,
        }
    }
}

impl From<RefZone> for RefZoneWire {
    fn from(z: RefZone) -> Self {
        match z {
            RefZone::Body => RefZoneWire::Body,
            RefZone::Edge(side) => RefZoneWire::Edge { side: side.into() },
            RefZone::Corner(corner) => RefZoneWire::Corner {
                corner: corner.into(),
            },
        }
    }
}

impl From<HitTest> for HitTestWire {
    fn from(h: HitTest) -> Self {
        match h {
            HitTest::Cell { row, column } => HitTestWire::Cell { row, column },
            HitTest::RowHeader(row) => HitTestWire::RowHeader { row },
            HitTest::ColumnHeader(column) => HitTestWire::ColumnHeader { column },
            HitTest::Corner => HitTestWire::Corner,
            HitTest::AutofillHandle { row, column } => HitTestWire::AutofillHandle { row, column },
            HitTest::FormulaRef {
                ref_idx,
                zone,
                grab_row,
                grab_column,
            } => HitTestWire::FormulaRef {
                ref_idx,
                zone: zone.into(),
                grab_row,
                grab_column,
            },
            HitTest::Outside => HitTestWire::Outside,
        }
    }
}

impl From<ResizeTarget> for ResizeTargetWire {
    fn from(r: ResizeTarget) -> Self {
        match r {
            ResizeTarget::RowEdge(row) => ResizeTargetWire::Row { row },
            ResizeTarget::ColumnEdge(column) => ResizeTargetWire::Column { column },
        }
    }
}

impl From<CanvasSize> for CanvasSizeWire {
    fn from(s: CanvasSize) -> Self {
        CanvasSizeWire { w: s.w, h: s.h }
    }
}

// `PixelRect` and `Point` already derive `Serialize` in `iron-canvas-core`
// (they ride the `.icr` schema), so no wire-shape mirror is needed for
// them — call sites serialize them directly.

// =============================================================================
// Inbound setters (JS->Rust, Deserialize): overlay inputs.
// =============================================================================
//
// JS pushes these as plain objects via `serde_wasm_bindgen::from_value`. We
// validate by `try_into()` at the call site so a malformed payload surfaces
// as a `JsError`, not a panic.

#[derive(Deserialize)]
pub(crate) struct RCRangeWire {
    pub r1: i32,
    pub c1: i32,
    pub r2: i32,
    pub c2: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AutofillTargetWire {
    pub row: i32,
    /// JS-side name. The engine field is `col`; we expose `column` for
    /// consistency with the rest of the JS API (HitTest, CellRect, etc.).
    pub column: i32,
}

#[derive(Deserialize)]
pub(crate) struct SheetAreaWire {
    pub sheet: u32,
    pub range: RCRangeWire,
}

/// Mirror of the engine `FormulaRefKind`. All variants are unit today; the
/// plan's TS shape carries `name` / `text` payloads, but the engine doesn't
/// model them yet, so the wire shape matches what the renderer actually
/// consumes.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum FormulaRefKindWire {
    Direct,
    DefinedName,
    Unresolved,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FormulaRefWire {
    pub sheet_area: SheetAreaWire,
    pub color_idx: usize,
    pub kind: FormulaRefKindWire,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct RenderOverlaysWire {
    pub extend_to: Option<AutofillTargetWire>,
    pub clipboard: Option<SheetAreaWire>,
    pub point_range: Option<RCRangeWire>,
    pub formula_refs: Vec<FormulaRefWire>,
}

impl From<RCRangeWire> for RCRange {
    fn from(r: RCRangeWire) -> Self {
        RCRange {
            r1: r.r1,
            c1: r.c1,
            r2: r.r2,
            c2: r.c2,
        }
    }
}

impl From<AutofillTargetWire> for AutofillTarget {
    fn from(a: AutofillTargetWire) -> Self {
        AutofillTarget {
            row: a.row,
            col: a.column,
        }
    }
}

impl From<SheetAreaWire> for SheetArea {
    fn from(s: SheetAreaWire) -> Self {
        SheetArea {
            sheet: s.sheet,
            range: s.range.into(),
        }
    }
}

impl From<FormulaRefKindWire> for FormulaRefKind {
    fn from(k: FormulaRefKindWire) -> Self {
        match k {
            FormulaRefKindWire::Direct => FormulaRefKind::Direct,
            FormulaRefKindWire::DefinedName => FormulaRefKind::DefinedName,
            FormulaRefKindWire::Unresolved => FormulaRefKind::Unresolved,
        }
    }
}

impl From<FormulaRefWire> for FormulaRef {
    fn from(f: FormulaRefWire) -> Self {
        FormulaRef {
            sheet_area: f.sheet_area.into(),
            color_idx: f.color_idx,
            kind: f.kind.into(),
        }
    }
}

// =============================================================================
// Inbound setters (JS->Rust, Deserialize): theme inputs.
// =============================================================================
//
// `CanvasThemeWire` requires every field — semantically a full push, no light
// fallback. `ThemeVariablesWire` is the partial-override shape; missing
// fields default to `None` and resolve to LIGHT inside the engine's
// `From<ThemeVariables> for CanvasTheme` impl.

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CanvasThemeWire {
    pub grid_color: String,
    pub grid_separator_color: String,
    pub header_bg: String,
    pub header_border_color: String,
    pub header_text_color: String,
    pub header_selected_bg: String,
    pub header_selected_color: String,
    pub default_text_color: String,
    pub error_text_color: String,
    pub selection_color: String,
    pub cell_bg: String,
    pub pointing: String,
    pub selection_fill: String,
    pub pointing_tint: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ThemeVariablesWire {
    pub grid_color: Option<String>,
    pub grid_separator_color: Option<String>,
    pub header_bg: Option<String>,
    pub header_border_color: Option<String>,
    pub header_text_color: Option<String>,
    pub header_selected_bg: Option<String>,
    pub header_selected_color: Option<String>,
    pub default_text_color: Option<String>,
    pub error_text_color: Option<String>,
    pub selection_color: Option<String>,
    pub cell_bg: Option<String>,
    pub pointing: Option<String>,
    pub selection_fill: Option<String>,
    pub pointing_tint: Option<String>,
}

impl From<CanvasThemeWire> for CanvasTheme {
    fn from(t: CanvasThemeWire) -> Self {
        CanvasTheme {
            grid_color: Cow::Owned(t.grid_color),
            grid_separator_color: Cow::Owned(t.grid_separator_color),
            header_bg: Cow::Owned(t.header_bg),
            header_border_color: Cow::Owned(t.header_border_color),
            header_text_color: Cow::Owned(t.header_text_color),
            header_selected_bg: Cow::Owned(t.header_selected_bg),
            header_selected_color: Cow::Owned(t.header_selected_color),
            default_text_color: Cow::Owned(t.default_text_color),
            error_text_color: Cow::Owned(t.error_text_color),
            selection_color: Cow::Owned(t.selection_color),
            cell_bg: Cow::Owned(t.cell_bg),
            pointing: Cow::Owned(t.pointing),
            selection_fill: Cow::Owned(t.selection_fill),
            pointing_tint: Cow::Owned(t.pointing_tint),
        }
    }
}

impl From<ThemeVariablesWire> for ThemeVariables {
    fn from(v: ThemeVariablesWire) -> Self {
        ThemeVariables {
            grid_color: v.grid_color,
            grid_separator_color: v.grid_separator_color,
            header_bg: v.header_bg,
            header_border_color: v.header_border_color,
            header_text_color: v.header_text_color,
            header_selected_bg: v.header_selected_bg,
            header_selected_color: v.header_selected_color,
            default_text_color: v.default_text_color,
            error_text_color: v.error_text_color,
            selection_color: v.selection_color,
            cell_bg: v.cell_bg,
            pointing: v.pointing,
            selection_fill: v.selection_fill,
            pointing_tint: v.pointing_tint,
        }
    }
}

impl RenderOverlaysWire {
    /// Convert to the engine `RenderOverlays`. Currently infallible; the
    /// `Result` is preserved so future boundary invariants can surface as
    /// a `JsError` without rippling through the call sites.
    pub(crate) fn into_engine(self) -> Result<RenderOverlays, String> {
        Ok(RenderOverlays {
            extend_to: self.extend_to.map(Into::into),
            clipboard: self.clipboard.map(Into::into),
            point_range: self.point_range.map(Into::into),
            formula_refs: self.formula_refs.into_iter().map(Into::into).collect(),
        })
    }
}

#[cfg(feature = "dev-tools")]
mod dev_wire {
    // =============================================================================
    // Outbound (Rust->JS, Serialize): dev-only frame diagnostics projection.
    // =============================================================================
    //
    // The engine's `FrameDiagnostics` stays serde-free; this projection is the
    // only place that decides wire names and tagging. Versioned by
    // `schemaVersion` (DIAG_SCHEMA_VERSION) so a later recorder embedding can
    // migrate. Field names are asserted by the native conversion test above.

    use super::CanvasSizeWire;
    use serde::Serialize;

    use iron_canvas_core::chrome::{GridShape, PaneRegion};
    use iron_canvas_core::renderer::diag::{
        DiagBlit, DiagBlitResultTag, DiagBufferTruth, DiagCache, DiagCacheActionTag,
        DiagCacheResolution, DiagCacheTruth, DiagDeltaKind, DiagFetch, DiagFetchPurpose,
        DiagFetchRequest, DiagFingerprintActionTag, DiagFingerprintTruth, DiagGeometry,
        DiagPaintCounts, DiagPaintedLayers, DiagRepaint, DiagRepaintReason, DiagRevealedStrip,
        DiagSegment, FrameDiagnostics,
    };
    use iron_canvas_core::{
        FrameInputFailure, GridVerdict, PaintRegimeTag, RCRange, RebuildReason, RowSpan, WorkFlags,
    };

    /// camelCase mirror of `PaintRegimeTag`. The engine tag rides the `.icr`
    /// recorder schema with `snake_case` names; this projection re-tags it
    /// camelCase to match the rest of the diagnostics wire.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum PaintRegimeTagWire {
        Overlay,
        Viewport,
        SlotsReuse,
        Fresh,
        Damage,
    }

    impl From<PaintRegimeTag> for PaintRegimeTagWire {
        fn from(tag: PaintRegimeTag) -> Self {
            match tag {
                PaintRegimeTag::Overlay => Self::Overlay,
                PaintRegimeTag::Viewport => Self::Viewport,
                PaintRegimeTag::SlotsReuse => Self::SlotsReuse,
                PaintRegimeTag::Fresh => Self::Fresh,
                PaintRegimeTag::Damage => Self::Damage,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct FrameDiagnosticsWire {
        pub schema_version: u8,
        pub attempt_seq: u64,
        pub committed_seq: Option<u64>,
        pub selected: Option<PaintRegimeTagWire>,
        pub effective: Option<PaintRegimeTagWire>,
        pub work: Vec<&'static str>,
        pub delta: Option<DiagDeltaKindWire>,
        pub rebuild_reason: Option<RebuildReasonWire>,
        pub outcome: FrameOutcomeWire,
        pub painted_layers: DiagPaintedLayersWire,
        pub probe: Option<RCRangeWireOut>,
        pub probe_segments: Vec<PaneRegionWire>,
        pub geometry: Option<DiagGeometryWire>,
        pub fetch: DiagFetchWire,
        pub repaint: DiagRepaintWire,
        pub cache: DiagCacheWire,
        pub blit: Option<DiagBlitWire>,
        pub paint_counts: DiagPaintCountsWire,
    }

    impl From<&FrameDiagnostics> for FrameDiagnosticsWire {
        fn from(diag: &FrameDiagnostics) -> Self {
            let mut work = Vec::new();
            if diag.work.contains(WorkFlags::VIEW) {
                work.push("view");
            }
            if diag.work.contains(WorkFlags::CONTENT) {
                work.push("content");
            }
            if diag.work.contains(WorkFlags::GEOMETRY) {
                work.push("geometry");
            }
            if diag.work.contains(WorkFlags::OVERLAY) {
                work.push("overlay");
            }
            Self {
                schema_version: diag.schema_version,
                attempt_seq: diag.attempt_seq,
                committed_seq: diag.committed_seq,
                selected: diag.selected.map(PaintRegimeTagWire::from),
                effective: diag.effective.map(PaintRegimeTagWire::from),
                work,
                delta: diag.delta.map(DiagDeltaKindWire::from),
                rebuild_reason: diag.rebuild_reason.map(RebuildReasonWire::from),
                outcome: FrameOutcomeWire::from(diag.outcome),
                painted_layers: DiagPaintedLayersWire {
                    grid: diag.painted_layers.grid,
                    overlay: diag.painted_layers.overlay,
                },
                probe: diag.probe.map(RCRangeWireOut::from),
                probe_segments: diag
                    .probe_segments
                    .iter()
                    .copied()
                    .map(PaneRegionWire::from)
                    .collect(),
                geometry: diag.geometry.as_ref().map(DiagGeometryWire::from),
                fetch: DiagFetchWire::from(&diag.fetch),
                repaint: DiagRepaintWire::from(&diag.repaint),
                cache: DiagCacheWire::from(&diag.cache),
                blit: diag.blit.as_ref().map(DiagBlitWire::from),
                paint_counts: DiagPaintCountsWire {
                    rows: diag.paint_counts.rows,
                    cells: diag.paint_counts.cells,
                },
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum DiagDeltaKindWire {
        Stable,
        Scroll,
        Rebuild,
    }

    impl From<DiagDeltaKind> for DiagDeltaKindWire {
        fn from(kind: DiagDeltaKind) -> Self {
            match kind {
                DiagDeltaKind::Stable => Self::Stable,
                DiagDeltaKind::Scroll => Self::Scroll,
                DiagDeltaKind::Rebuild => Self::Rebuild,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum RebuildReasonWire {
        NoCommittedFrame,
        Size,
        Dpr,
        Theme,
        Model,
        Sheet,
        Freeze,
        Headers,
        TwoAxisScroll,
        MissingActiveSnapshot,
        ActiveCellChangedOrUnknown,
        IncompatibleScrollOverlap,
    }

    impl From<RebuildReason> for RebuildReasonWire {
        fn from(reason: RebuildReason) -> Self {
            match reason {
                RebuildReason::NoCommittedFrame => Self::NoCommittedFrame,
                RebuildReason::Size => Self::Size,
                RebuildReason::Dpr => Self::Dpr,
                RebuildReason::Theme => Self::Theme,
                RebuildReason::Model => Self::Model,
                RebuildReason::Sheet => Self::Sheet,
                RebuildReason::Freeze => Self::Freeze,
                RebuildReason::Headers => Self::Headers,
                RebuildReason::TwoAxisScroll => Self::TwoAxisScroll,
                RebuildReason::MissingActiveSnapshot => Self::MissingActiveSnapshot,
                RebuildReason::ActiveCellChangedOrUnknown => Self::ActiveCellChangedOrUnknown,
                RebuildReason::IncompatibleScrollOverlap => Self::IncompatibleScrollOverlap,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "camelCase")]
    pub(crate) enum FrameOutcomeWire {
        Painted,
        HeldOnBridgeFailure,
        HeldOnInputFailure { input: FrameInputFailureWire },
    }

    impl From<iron_canvas_core::FrameOutcome> for FrameOutcomeWire {
        fn from(outcome: iron_canvas_core::FrameOutcome) -> Self {
            match outcome {
                iron_canvas_core::FrameOutcome::Painted => Self::Painted,
                iron_canvas_core::FrameOutcome::HeldOnBridgeFailure => Self::HeldOnBridgeFailure,
                iron_canvas_core::FrameOutcome::HeldOnInputFailure(input) => {
                    Self::HeldOnInputFailure {
                        input: FrameInputFailureWire::from(input),
                    }
                }
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum FrameInputFailureWire {
        SelectedSheet,
        SelectedView,
        SheetMismatch,
        FrozenRows,
        FrozenColumns,
        RowHeaderVisibility,
        ColumnHeaderVisibility,
    }

    impl From<FrameInputFailure> for FrameInputFailureWire {
        fn from(failure: FrameInputFailure) -> Self {
            match failure {
                FrameInputFailure::SelectedSheet => Self::SelectedSheet,
                FrameInputFailure::SelectedView => Self::SelectedView,
                FrameInputFailure::SheetMismatch => Self::SheetMismatch,
                FrameInputFailure::FrozenRows => Self::FrozenRows,
                FrameInputFailure::FrozenColumns => Self::FrozenColumns,
                FrameInputFailure::RowHeaderVisibility => Self::RowHeaderVisibility,
                FrameInputFailure::ColumnHeaderVisibility => Self::ColumnHeaderVisibility,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagPaintedLayersWire {
        pub grid: bool,
        pub overlay: bool,
    }

    impl From<DiagPaintedLayers> for DiagPaintedLayersWire {
        fn from(layers: DiagPaintedLayers) -> Self {
            Self {
                grid: layers.grid,
                overlay: layers.overlay,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct RCRangeWireOut {
        pub r1: i32,
        pub c1: i32,
        pub r2: i32,
        pub c2: i32,
    }

    impl From<RCRange> for RCRangeWireOut {
        fn from(range: RCRange) -> Self {
            Self {
                r1: range.r1,
                c1: range.c1,
                r2: range.r2,
                c2: range.c2,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum PaneRegionWire {
        TopLeft,
        TopRight,
        BottomLeft,
        BottomRight,
    }

    impl From<PaneRegion> for PaneRegionWire {
        fn from(region: PaneRegion) -> Self {
            match region {
                PaneRegion::TopLeft => Self::TopLeft,
                PaneRegion::TopRight => Self::TopRight,
                PaneRegion::BottomLeft => Self::BottomLeft,
                PaneRegion::BottomRight => Self::BottomRight,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagSegmentWire {
        pub region: PaneRegionWire,
        pub range: RCRangeWireOut,
        pub cells: usize,
    }

    impl From<&DiagSegment> for DiagSegmentWire {
        fn from(segment: &DiagSegment) -> Self {
            Self {
                region: PaneRegionWire::from(segment.region),
                range: RCRangeWireOut::from(segment.range),
                cells: segment.cells,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct GridShapeWire {
        pub row_lens: [usize; 2],
        pub col_lens: [usize; 2],
        pub frozen_rows: i32,
        pub frozen_cols: i32,
    }

    impl From<GridShape> for GridShapeWire {
        fn from(shape: GridShape) -> Self {
            Self {
                row_lens: shape.row_lens(),
                col_lens: shape.col_lens(),
                frozen_rows: shape.frozen_rows(),
                frozen_cols: shape.frozen_cols(),
            }
        }
    }

    /// Geometry: the design's example carries frozen counts at the geometry
    /// root (topRow/leftColumn/frozenRows/frozenColumns); the shape object
    /// repeats them alongside the exact slot lengths. `cssSize` is the
    /// logical CSS size the grid planned against; `backingSize` is the
    /// physical backing-store size — core derives it from CSS x DPR, and
    /// the facade overwrites it with the actual canvas backing store
    /// before serialization so CSS/backing mismatches are visible.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagGeometryWire {
        pub css_size: CanvasSizeWire,
        pub backing_size: BackingSizeWire,
        pub dpr: f64,
        pub sheet: u32,
        pub top_row: i32,
        pub left_column: i32,
        pub frozen_rows: i32,
        pub frozen_cols: i32,
        pub row_header_thickness: i32,
        pub col_header_thickness: i32,
        pub show_row_headers: bool,
        pub show_col_headers: bool,
        pub shape: GridShapeWire,
        pub segments: Vec<DiagSegmentWire>,
    }

    #[derive(Serialize, Clone, Copy)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct BackingSizeWire {
        pub w: u32,
        pub h: u32,
    }

    impl From<&DiagGeometry> for DiagGeometryWire {
        fn from(geometry: &DiagGeometry) -> Self {
            let (w, h) = geometry.backing_size;
            Self {
                css_size: CanvasSizeWire::from(geometry.canvas),
                backing_size: BackingSizeWire { w, h },
                dpr: geometry.dpr,
                sheet: geometry.sheet,
                top_row: geometry.top_row,
                left_column: geometry.left_column,
                frozen_rows: geometry.shape.frozen_rows(),
                frozen_cols: geometry.shape.frozen_cols(),
                row_header_thickness: geometry.row_header_thickness,
                col_header_thickness: geometry.col_header_thickness,
                show_row_headers: geometry.show_row_headers,
                show_col_headers: geometry.show_col_headers,
                shape: GridShapeWire::from(geometry.shape),
                segments: geometry
                    .segments
                    .iter()
                    .map(DiagSegmentWire::from)
                    .collect(),
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum DiagFetchPurposeWire {
        FullSegment,
        DamageStrip,
        BlitReveal,
    }

    impl From<DiagFetchPurpose> for DiagFetchPurposeWire {
        fn from(purpose: DiagFetchPurpose) -> Self {
            match purpose {
                DiagFetchPurpose::FullSegment => Self::FullSegment,
                DiagFetchPurpose::DamageStrip => Self::DamageStrip,
                DiagFetchPurpose::BlitReveal => Self::BlitReveal,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagFetchRequestWire {
        pub purpose: DiagFetchPurposeWire,
        pub region: Option<PaneRegionWire>,
        pub range: RCRangeWireOut,
        pub cells: usize,
        pub slots: usize,
    }

    impl From<&DiagFetchRequest> for DiagFetchRequestWire {
        fn from(request: &DiagFetchRequest) -> Self {
            Self {
                purpose: DiagFetchPurposeWire::from(request.purpose),
                region: request.region.map(PaneRegionWire::from),
                range: RCRangeWireOut::from(request.range),
                cells: request.cells,
                slots: request.slots,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagFetchWire {
        pub batches: usize,
        pub addressed_cells: usize,
        pub logical_slots: usize,
        pub requests: Vec<DiagFetchRequestWire>,
    }

    impl From<&DiagFetch> for DiagFetchWire {
        fn from(fetch: &DiagFetch) -> Self {
            Self {
                batches: fetch.batches,
                addressed_cells: fetch.addressed_cells,
                logical_slots: fetch.logical_slots,
                requests: fetch
                    .requests
                    .iter()
                    .map(DiagFetchRequestWire::from)
                    .collect(),
            }
        }
    }

    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "camelCase")]
    pub(crate) enum GridVerdictWire {
        Skip,
        Rows { spans: u8, rows: u16 },
        Full,
        Strip,
        Held,
    }

    impl From<GridVerdict> for GridVerdictWire {
        fn from(verdict: GridVerdict) -> Self {
            match verdict {
                GridVerdict::Skip => Self::Skip,
                GridVerdict::Rows { spans, rows } => Self::Rows { spans, rows },
                GridVerdict::Full => Self::Full,
                GridVerdict::Strip => Self::Strip,
                GridVerdict::Held => Self::Held,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum DiagRepaintReasonWire {
        NoPaintedHistory,
        LayoutMismatch,
        RowAddressMismatch,
        SpanCapExceeded,
        BorderSafety,
        FingerprintsEqual,
        ChangedRows,
    }

    impl From<DiagRepaintReason> for DiagRepaintReasonWire {
        fn from(reason: DiagRepaintReason) -> Self {
            match reason {
                DiagRepaintReason::NoPaintedHistory => Self::NoPaintedHistory,
                DiagRepaintReason::LayoutMismatch => Self::LayoutMismatch,
                DiagRepaintReason::RowAddressMismatch => Self::RowAddressMismatch,
                DiagRepaintReason::SpanCapExceeded => Self::SpanCapExceeded,
                DiagRepaintReason::BorderSafety => Self::BorderSafety,
                DiagRepaintReason::FingerprintsEqual => Self::FingerprintsEqual,
                DiagRepaintReason::ChangedRows => Self::ChangedRows,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct RowSpanWire {
        pub r1: i32,
        pub r2: i32,
    }

    impl From<RowSpan> for RowSpanWire {
        fn from(span: RowSpan) -> Self {
            Self {
                r1: span.r1,
                r2: span.r2,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagRepaintWire {
        pub verdict: Option<GridVerdictWire>,
        pub reason: Option<DiagRepaintReasonWire>,
        pub changed_rows: Vec<RowSpanWire>,
    }

    impl From<&DiagRepaint> for DiagRepaintWire {
        fn from(repaint: &DiagRepaint) -> Self {
            Self {
                verdict: repaint.verdict.map(GridVerdictWire::from),
                reason: repaint.reason.map(DiagRepaintReasonWire::from),
                changed_rows: repaint
                    .changed_rows
                    .iter()
                    .copied()
                    .map(RowSpanWire::from)
                    .collect(),
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum DiagCacheActionTagWire {
        None,
        Replace,
        Splice,
        Shift,
        Reset,
    }

    impl From<DiagCacheActionTag> for DiagCacheActionTagWire {
        fn from(tag: DiagCacheActionTag) -> Self {
            match tag {
                DiagCacheActionTag::None => Self::None,
                DiagCacheActionTag::Replace => Self::Replace,
                DiagCacheActionTag::Splice => Self::Splice,
                DiagCacheActionTag::Shift => Self::Shift,
                DiagCacheActionTag::Reset => Self::Reset,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum DiagFingerprintActionTagWire {
        Install,
        MarkStale,
        Reset,
    }

    impl From<DiagFingerprintActionTag> for DiagFingerprintActionTagWire {
        fn from(tag: DiagFingerprintActionTag) -> Self {
            match tag {
                DiagFingerprintActionTag::Install => Self::Install,
                DiagFingerprintActionTag::MarkStale => Self::MarkStale,
                DiagFingerprintActionTag::Reset => Self::Reset,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagCacheTruthWire {
        pub layout: Option<DiagLayoutWire>,
        pub buffer_truth: String,
        pub fingerprint_truth: String,
    }

    impl From<&DiagCacheTruth> for DiagCacheTruthWire {
        fn from(truth: &DiagCacheTruth) -> Self {
            Self {
                layout: truth.layout.map(DiagLayoutWire::from),
                buffer_truth: match truth.buffer_truth {
                    DiagBufferTruth::Valid => "valid".to_string(),
                    DiagBufferTruth::Stale => "stale".to_string(),
                },
                fingerprint_truth: match truth.fingerprint_truth {
                    DiagFingerprintTruth::Exact => "exact".to_string(),
                    DiagFingerprintTruth::Stale => "stale".to_string(),
                },
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagLayoutWire {
        pub shape: GridShapeWire,
        pub segments: Vec<DiagSegmentWire>,
    }

    impl From<iron_canvas_core::chrome::GridLayout> for DiagLayoutWire {
        fn from(layout: iron_canvas_core::chrome::GridLayout) -> Self {
            Self {
                shape: GridShapeWire::from(layout.shape()),
                segments: layout
                    .segments()
                    .map(|segment| DiagSegmentWire {
                        region: PaneRegionWire::from(segment.region()),
                        range: RCRangeWireOut::from(segment.range()),
                        cells: (segment.range().r2 - segment.range().r1 + 1).max(0) as usize
                            * (segment.range().c2 - segment.range().c1 + 1).max(0) as usize,
                    })
                    .collect(),
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum DiagCacheResolutionWire {
        Committed,
        HeldForRetry,
    }

    impl From<DiagCacheResolution> for DiagCacheResolutionWire {
        fn from(resolution: DiagCacheResolution) -> Self {
            match resolution {
                DiagCacheResolution::Committed => Self::Committed,
                DiagCacheResolution::HeldForRetry => Self::HeldForRetry,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagCacheWire {
        pub planned_action: Option<DiagCacheActionTagWire>,
        pub fingerprint_action: Option<DiagFingerprintActionTagWire>,
        pub committed_before: Option<DiagCacheTruthWire>,
        pub resolution: DiagCacheResolutionWire,
        pub committed_after: DiagCacheTruthWire,
    }

    impl From<&DiagCache> for DiagCacheWire {
        fn from(cache: &DiagCache) -> Self {
            Self {
                planned_action: cache.planned_action.map(DiagCacheActionTagWire::from),
                fingerprint_action: cache
                    .fingerprint_action
                    .map(DiagFingerprintActionTagWire::from),
                committed_before: cache
                    .committed_before
                    .as_ref()
                    .map(DiagCacheTruthWire::from),
                resolution: DiagCacheResolutionWire::from(cache.resolution),
                committed_after: DiagCacheTruthWire::from(&cache.committed_after),
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum AxisWire {
        Row,
        Column,
    }

    impl From<iron_canvas_core::geometry::prim::Axis> for AxisWire {
        fn from(axis: iron_canvas_core::geometry::prim::Axis) -> Self {
            match axis {
                iron_canvas_core::geometry::prim::Axis::Row => Self::Row,
                iron_canvas_core::geometry::prim::Axis::Column => Self::Column,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) enum DiagBlitResultTagWire {
        Shifted,
        HeldPreflight,
        GridFallback,
        FreshFallback,
    }

    impl From<DiagBlitResultTag> for DiagBlitResultTagWire {
        fn from(tag: DiagBlitResultTag) -> Self {
            match tag {
                DiagBlitResultTag::Shifted => Self::Shifted,
                DiagBlitResultTag::HeldPreflight => Self::HeldPreflight,
                DiagBlitResultTag::GridFallback => Self::GridFallback,
                DiagBlitResultTag::FreshFallback => Self::FreshFallback,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagRevealedStripWire {
        pub region: PaneRegionWire,
        pub range: RCRangeWireOut,
    }

    impl From<&DiagRevealedStrip> for DiagRevealedStripWire {
        fn from(strip: &DiagRevealedStrip) -> Self {
            Self {
                region: PaneRegionWire::from(strip.region),
                range: RCRangeWireOut::from(strip.range),
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagBlitWire {
        pub axis: AxisWire,
        pub delta: i32,
        // `PixelRect`/`Point` already derive Serialize in core (they ride the
        // .icr schema) — serialize them directly, as wire.rs already does.
        pub src: iron_canvas_core::geometry::pixel_rect::PixelRect,
        pub dst: iron_canvas_core::geometry::pixel_rect::PixelRect,
        // `null` when execution never reached `push_clip` (held and both
        // fallback outcomes) — never a fabricated zero rectangle.
        pub clip: Option<iron_canvas_core::geometry::pixel_rect::PixelRect>,
        pub strip: iron_canvas_core::geometry::pixel_rect::PixelRect,
        pub revealed: Vec<DiagRevealedStripWire>,
        pub result: DiagBlitResultTagWire,
        pub cold_cache: Option<bool>,
    }

    impl From<&DiagBlit> for DiagBlitWire {
        fn from(blit: &DiagBlit) -> Self {
            Self {
                axis: AxisWire::from(blit.axis),
                delta: blit.delta,
                src: blit.src,
                dst: blit.dst,
                clip: blit.clip,
                strip: blit.strip,
                revealed: blit
                    .revealed
                    .iter()
                    .map(DiagRevealedStripWire::from)
                    .collect(),
                result: DiagBlitResultTagWire::from(blit.result),
                cold_cache: blit.cold_cache,
            }
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct DiagPaintCountsWire {
        pub rows: usize,
        pub cells: usize,
    }

    impl From<DiagPaintCounts> for DiagPaintCountsWire {
        fn from(counts: DiagPaintCounts) -> Self {
            Self {
                rows: counts.rows,
                cells: counts.cells,
            }
        }
    }
}

#[cfg(feature = "dev-tools")]
pub(crate) use dev_wire::*;

#[cfg(all(test, feature = "dev-tools"))]
mod tests {
    use super::*;
    use iron_canvas_core::chrome::PaneRegion;
    use iron_canvas_core::{
        DiagBufferTruth, DiagCacheActionTag, DiagCacheResolution, DiagCacheTruth, DiagDeltaKind,
        DiagFingerprintActionTag, DiagFingerprintTruth, DiagPaintCounts, DiagPaintedLayers,
        DiagRepaintReason, FrameDiagnostics, FrameOutcome, GridVerdict, RCRange, RebuildReason,
        RowSpan,
    };

    /// The wire shape is the contract the browser mirrors parse. Prove the
    /// exact field names here, natively, before any browser test relies on
    /// them.
    #[test]
    fn frame_diagnostics_wire_matches_declared_shape() {
        let diag = FrameDiagnostics {
            schema_version: 1,
            attempt_seq: 7,
            committed_seq: Some(6),
            selected: Some(iron_canvas_core::PaintRegimeTag::SlotsReuse),
            effective: Some(iron_canvas_core::PaintRegimeTag::SlotsReuse),
            work: iron_canvas_core::WorkFlags::CONTENT,
            delta: Some(DiagDeltaKind::Stable),
            rebuild_reason: Some(RebuildReason::Freeze),
            outcome: FrameOutcome::Painted,
            painted_layers: DiagPaintedLayers {
                grid: true,
                overlay: false,
            },
            probe: Some(RCRange {
                r1: 5,
                c1: 4,
                r2: 5,
                c2: 4,
            }),
            probe_segments: vec![PaneRegion::BottomLeft],
            geometry: None,
            fetch: Default::default(),
            repaint: iron_canvas_core::DiagRepaint {
                verdict: Some(GridVerdict::Rows { spans: 1, rows: 1 }),
                reason: Some(DiagRepaintReason::ChangedRows),
                changed_rows: vec![RowSpan { r1: 5, r2: 5 }],
            },
            cache: iron_canvas_core::DiagCache {
                planned_action: Some(DiagCacheActionTag::Replace),
                fingerprint_action: Some(DiagFingerprintActionTag::Install),
                committed_before: Some(DiagCacheTruth {
                    layout: None,
                    buffer_truth: DiagBufferTruth::Stale,
                    fingerprint_truth: DiagFingerprintTruth::Stale,
                }),
                resolution: DiagCacheResolution::Committed,
                committed_after: DiagCacheTruth {
                    layout: None,
                    buffer_truth: DiagBufferTruth::Valid,
                    fingerprint_truth: DiagFingerprintTruth::Exact,
                },
            },
            // A fallback/held blit never reaches `push_clip`: the wire
            // must carry `clip: null`, never a fabricated zero rect.
            blit: Some(iron_canvas_core::DiagBlit {
                axis: iron_canvas_core::geometry::prim::Axis::Row,
                delta: 4,
                src: iron_canvas_core::PixelRect::default(),
                dst: iron_canvas_core::PixelRect::default(),
                clip: None,
                strip: iron_canvas_core::PixelRect::default(),
                revealed: Vec::new(),
                result: iron_canvas_core::DiagBlitResultTag::GridFallback,
                cold_cache: Some(true),
            }),
            paint_counts: DiagPaintCounts { rows: 1, cells: 21 },
        };

        let wire = FrameDiagnosticsWire::from(&diag);
        let json = serde_json::to_value(&wire).expect("wire serializes");

        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["attemptSeq"], 7);
        assert_eq!(json["committedSeq"], 6);
        assert_eq!(json["selected"], "slotsReuse");
        assert_eq!(json["work"], serde_json::json!(["content"]));
        assert_eq!(json["delta"], "stable");
        assert_eq!(json["rebuildReason"], "freeze");
        assert_eq!(json["outcome"]["kind"], "painted");
        assert_eq!(json["paintedLayers"]["grid"], true);
        assert_eq!(json["paintedLayers"]["overlay"], false);
        assert_eq!(
            json["probe"],
            serde_json::json!({ "r1": 5, "c1": 4, "r2": 5, "c2": 4 })
        );
        assert_eq!(json["probeSegments"], serde_json::json!(["bottomLeft"]));
        assert_eq!(json["geometry"], serde_json::Value::Null);
        assert_eq!(json["repaint"]["verdict"]["kind"], "rows");
        assert_eq!(json["repaint"]["verdict"]["spans"], 1);
        assert_eq!(json["repaint"]["reason"], "changedRows");
        assert_eq!(
            json["repaint"]["changedRows"],
            serde_json::json!([{ "r1": 5, "r2": 5 }])
        );
        assert_eq!(json["cache"]["plannedAction"], "replace");
        assert_eq!(json["cache"]["fingerprintAction"], "install");
        assert_eq!(json["cache"]["committedBefore"]["bufferTruth"], "stale");
        assert_eq!(json["cache"]["resolution"], "committed");
        assert_eq!(json["cache"]["committedAfter"]["fingerprintTruth"], "exact");
        assert_eq!(
            json["blit"]["clip"],
            serde_json::Value::Null,
            "a blit that never reached push_clip must emit clip: null"
        );
        assert_eq!(json["blit"]["result"], "gridFallback");
        assert_eq!(json["blit"]["delta"], 4);
        assert_eq!(json["paintCounts"]["rows"], 1);
        assert_eq!(json["paintCounts"]["cells"], 21);
    }
}
