//! JS-facing wire shapes for the `IronCanvas` query API.
//!
//! The engine enums in `iron-canvas-core` use tuple variants for their
//! ergonomic in-Rust shape (`RowHeader(i32)`, `Edge(Side)`,
//! `ResizeTarget::Column(i32)`). Serde's internal tagging (`tag = "kind"`)
//! rejects tuple variants, and the engine crates are deliberately kept
//! free of `wasm-bindgen` / `serde-wasm-bindgen` concerns. So the JS shape
//! is materialized here: small, struct-formed mirrors that derive
//! `Serialize`, plus `From` impls from the engine types.

use serde::{Deserialize, Serialize};

use std::borrow::Cow;

use iron_canvas_core::{
    AutofillTarget, CanvasTheme, Corner, FormulaRef, FormulaRefKind, HitTest, RCRange, RefZone,
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
    ColHeader {
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
        grab_col: i32,
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

impl From<Corner> for CornerWire {
    fn from(c: Corner) -> Self {
        match c {
            Corner::TopLeft => CornerWire::TopLeft,
            Corner::TopRight => CornerWire::TopRight,
            Corner::BottomLeft => CornerWire::BottomLeft,
            Corner::BottomRight => CornerWire::BottomRight,
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
            HitTest::ColHeader(column) => HitTestWire::ColHeader { column },
            HitTest::Corner => HitTestWire::Corner,
            HitTest::AutofillHandle { row, column } => HitTestWire::AutofillHandle { row, column },
            HitTest::FormulaRef {
                ref_idx,
                zone,
                grab_row,
                grab_col,
            } => HitTestWire::FormulaRef {
                ref_idx,
                zone: zone.into(),
                grab_row,
                grab_col,
            },
            HitTest::Outside => HitTestWire::Outside,
        }
    }
}

impl From<ResizeTarget> for ResizeTargetWire {
    fn from(r: ResizeTarget) -> Self {
        match r {
            ResizeTarget::Row(row) => ResizeTargetWire::Row { row },
            ResizeTarget::Column(column) => ResizeTargetWire::Column { column },
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
// Phase 2 — overlay setter inputs (Deserialize only).
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
// Phase 3 — theme setter inputs (Deserialize only).
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
