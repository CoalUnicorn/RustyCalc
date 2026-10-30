//! Scene-path render handle for the worksheet.
//!
//! M4 replacement for the legacy `IronCanvas` handle: owns one
//! `CanvasSession<Canvas2dSceneBackend>` and exposes the query surface the
//! input layer needs. The session borrows `&UserModel` only inside `render`;
//! no workbook handle is stored.

use iron_canvas::{
    CanvasSession, DisplayCell, FormulaRefHit, OverlayState, RenderOutcome, RenderRequest,
    RevisionToken,
};
use iron_canvas_canvas2d::Canvas2dSceneBackend;
use iron_canvas_core::{
    CanvasMetrics, CanvasSize, CanvasTheme, CellCoord, PixelRect, Point,
    scene_geometry::{GridHit, GridRange, GridResize, RangeFragment},
};
use iron_canvas_ironcalc::WorksheetViewport;
use iron_canvas_ironcalc::autofit::AutoFitError;
use ironcalc_base::{UserModel, links::CellLinkView};
use web_sys::HtmlCanvasElement;

use crate::coord::CellArea;

/// Owns the scene session and its Canvas2D backend. Size and scale are fixed
/// per instance; a resize rebuilds both.
pub struct SceneHandle {
    session: CanvasSession<Canvas2dSceneBackend>,
    size: CanvasSize,
    scale: f64,
    committed_context: Option<SceneContext>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SceneContext {
    workbook_id: u64,
    sheet: u32,
}

impl SceneHandle {
    pub fn new(canvas: HtmlCanvasElement, size: CanvasSize, scale: f64) -> Result<Self, String> {
        let metrics = CanvasMetrics::new(size, scale).map_err(|e| e.to_string())?;
        let backend = Canvas2dSceneBackend::new(canvas, metrics).map_err(|e| format!("{e:?}"))?;
        Ok(Self {
            session: CanvasSession::new(backend),
            size,
            scale,
            committed_context: None,
        })
    }

    /// Rebuild the backend and session at a new size or scale. M3 fixes the
    /// raster size per backend, so a resize is a fresh session rather than a
    /// mutation; the previous committed frame is dropped with it.
    pub fn resize(
        &mut self,
        canvas: HtmlCanvasElement,
        size: CanvasSize,
        scale: f64,
    ) -> Result<(), String> {
        *self = Self::new(canvas, size, scale)?;
        Ok(())
    }

    /// Prepare, stage, and commit one frame. The workbook is borrowed only for
    /// the duration of the call.
    pub fn render(
        &mut self,
        workbook: &UserModel<'_>,
        request: &RenderRequest,
    ) -> Result<RenderOutcome, String> {
        const TIMER_LABEL: &str = "rustycalc renderer";
        web_sys::console::time_with_label(TIMER_LABEL);
        let result = self.session.render(workbook, request);
        if result.is_ok() {
            self.committed_context = Some(SceneContext {
                workbook_id: request.revision.workbook_id,
                sheet: request.sheet,
            });
        }
        web_sys::console::time_end_with_label(TIMER_LABEL);
        result.map_err(|e| format!("{e:?}"))
    }

    /// Drop backend metric caches after a font change.
    pub fn invalidate(&mut self) {
        self.session.invalidate();
    }

    pub fn size(&self) -> CanvasSize {
        self.size
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    // Committed-frame queries, mirroring the legacy `IronCanvas` surface.
    pub fn hit_test(&self, point: Point) -> Option<GridHit> {
        self.session.hit_test(point)
    }

    pub fn formula_ref_hit_test(
        &self,
        point: Point,
        draggable_ref_indices: &[usize],
    ) -> Option<FormulaRefHit> {
        self.session
            .formula_ref_hit_test(point, draggable_ref_indices)
    }

    pub fn resize_target(&self, point: Point, tolerance: i32) -> Option<GridResize> {
        self.session.resize_target(point, tolerance)
    }

    pub fn display_cell_at(&self, point: Point) -> Option<DisplayCell<'_>> {
        self.session.display_cell_at(point)
    }

    pub fn link_at(&self, point: Point) -> Option<&CellLinkView> {
        self.session.link_at(point)
    }

    pub fn visible_fragments(&self, range: GridRange) -> Vec<RangeFragment> {
        self.session.visible_fragments(range)
    }

    pub fn cell_rect(&self, row: i32, col: i32) -> Option<PixelRect> {
        self.session.cell_rect(row, col)
    }

    pub fn scroll_pane_rect(&self) -> Option<PixelRect> {
        self.session.scroll_pane_rect()
    }

    pub fn autofill_handle_rect(&self, range: GridRange) -> Option<PixelRect> {
        self.session.autofill_handle_rect(range)
    }

    /// Whether committed geometry belongs to the live worksheet and workbook.
    pub(crate) fn matches_context(&self, sheet: u32, workbook_id: u64) -> bool {
        self.committed_context == Some(SceneContext { workbook_id, sheet })
    }

    /// Minimal origin that brings `(row, column)` fully inside the committed
    /// scroll pane, or `None` when it already fits. Reads model extents.
    pub fn scroll_to_show(
        &self,
        workbook: &UserModel<'_>,
        sheet: u32,
        workbook_id: u64,
        row: i32,
        column: i32,
    ) -> Option<(i32, i32)> {
        if !self.matches_context(sheet, workbook_id) {
            return None;
        }
        self.session.scroll_to_show(workbook, row, column)
    }

    pub fn fit_column_width(
        &self,
        workbook: &UserModel<'_>,
        col: i32,
        first_row: i32,
        last_row: i32,
    ) -> Result<Option<f64>, AutoFitError> {
        self.session
            .fit_column_width(workbook, col, first_row, last_row)
    }

    pub fn fit_row_height(
        &self,
        workbook: &UserModel<'_>,
        row: i32,
        first_col: i32,
        last_col: i32,
    ) -> Result<Option<f64>, AutoFitError> {
        self.session
            .fit_row_height(workbook, row, first_col, last_col)
    }
}

/// Clamp a live requested origin so the first scrollable row and column do
/// not overlap their frozen bands.
pub(crate) fn clamp_viewport_origin(
    origin: CellCoord,
    frozen_rows: i32,
    frozen_columns: i32,
) -> CellCoord {
    CellCoord {
        row: origin.row.max(frozen_rows.saturating_add(1)),
        col: origin.col.max(frozen_columns.saturating_add(1)),
    }
}

/// Keep a copied range on its source sheet when building a worksheet overlay.
pub(crate) fn clipboard_range_on_sheet(
    clipboard_sheet: u32,
    range: CellArea,
    requested_sheet: u32,
) -> Option<GridRange> {
    (clipboard_sheet == requested_sheet).then(|| range.into())
}

/// Build a render request for one worksheet frame at a canvas size.
pub fn request_for(
    sheet: u32,
    first_cell: CellCoord,
    size: CanvasSize,
    theme: CanvasTheme,
    overlays: OverlayState,
    revision: RevisionToken,
) -> RenderRequest {
    let (width, height) = size.to_logical_extent();
    RenderRequest {
        sheet,
        revision,
        viewport: WorksheetViewport {
            first_cell,
            width,
            height,
        },
        theme,
        overlays,
    }
}
