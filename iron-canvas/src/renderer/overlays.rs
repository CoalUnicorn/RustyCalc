//! Overlay-layer paints: selection rectangle, autofill drag preview,
//! clipboard marching ants, point-mode range, formula-ref highlights.
//!
//! These run on the transparent overlay canvas, which paints after the
//! grid layer (cells + borders + text + headers + corner). Each helper
//! bails early via `range_pixel_bounds(...)?` when the range is entirely
//! outside the drawable fold, so overlays never leak onto the canvas for
//! off-screen refs like `=BB3`.

use std::borrow::Cow;

use crate::geometry::constants::{
    DASHED_BORDER_WIDTH, SELECTION_BORDER_WIDTH, STANDARD_BORDER_WIDTH,
};
use crate::painter::{PaintColor, Painter};
use crate::theme::{FORMULA_REF_COLORS, FORMULA_REF_TINTS};
use crate::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
use crate::CanvasModel;

use super::super::geometry::constants::AUTOFILL_HANDLE_BORDER_PX;
use super::super::types::coord::CellAddress;
use super::{FrameContext, RendererCore};

/// Controls whether `draw_dashed_range` fills the interior with a light tint.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum DashFill {
    /// Outline only (used for clipboard marching ants).
    Outline,
    /// Outline + semi-transparent fill tint (used for point-mode range and
    /// formula refs). Carries a precomputed `rgba(...)` tint as
    /// `Cow<'static, str>` — built-in themes use `Cow::Borrowed` for the
    /// zero-alloc ptr-eq path; host-page themes carry the owned tint.
    Tinted(Cow<'static, str>),
}

impl<P: Painter> RendererCore<P> {
    /// Draw the blue selection border, semi-transparent fill, and autofill
    /// handle for the current selection.
    pub(super) fn draw_selection(&self, model: &dyn CanvasModel, frame: &FrameContext) {
        let view = model.get_selected_view();
        let sheet = model.get_selected_sheet();
        let addr = CellAddress {
            sheet,
            row: view.row,
            column: view.column,
        };
        let Some(cell) = self.range_pixel_bounds(frame, RCRange::from_view(model).normalized())
        else {
            return;
        };

        self.painter.rect_fill(
            cell,
            PaintColor::from_theme_str(&frame.theme.selection_fill),
        );

        // Restore the active cell's fill + borders + text on top of the
        // selection tint so its actual style shows through while selected.
        self.repaint_active_cell(model, addr, frame);

        self.painter.rect_stroke(
            cell,
            PaintColor::from_theme_str(&frame.theme.selection_color),
            f64::from(SELECTION_BORDER_WIDTH),
        );

        // Autofill handle: top-left at the selection's bottom-right corner
        // (Excel anchor — pokes outside the selection). Filled with
        // selection_color, ringed in cell_bg so it stays visible against any
        // cell fill underneath. Skips full-row/col selections — handle_rect
        // returns None there, matching `autofill_handle()` semantics.
        let handle = frame.autofill_handle_rect();

        self.painter.rect_fill(
            handle,
            PaintColor::from_theme_str(&frame.theme.selection_color),
        );
        self.painter.rect_stroke(
            handle,
            PaintColor::from_theme_str(&frame.theme.cell_bg),
            f64::from(AUTOFILL_HANDLE_BORDER_PX),
        );
    }

    /// Dashed preview of the autofill-handle drag target.
    pub(super) fn draw_extend_preview(
        &self,
        model: &dyn CanvasModel,
        frame: &FrameContext,
        target: AutofillTarget,
    ) {
        let sel = RCRange::from_view(model).normalized();
        let range = RCRange {
            r1: sel.r1.min(target.row),
            c1: sel.c1.min(target.col),
            r2: sel.r2.max(target.row),
            c2: sel.c2.max(target.col),
        };
        let Some(b) = self.range_pixel_bounds(frame, range) else {
            return;
        };

        self.painter.rect_dashed(
            b,
            PaintColor::from_theme_str(&frame.theme.selection_color),
            f64::from(STANDARD_BORDER_WIDTH),
        );
    }

    /// Clipboard marching-ants border around the last Ctrl+C copied range.
    /// No-op when the clipboard is empty or lives on another sheet.
    pub(super) fn draw_clipboard_overlay(
        &self,
        model: &dyn CanvasModel,
        frame: &FrameContext,
        clipboard: Option<&SheetArea>,
    ) {
        let sheet = model.get_selected_sheet();
        let Some(cb) = clipboard else { return };
        if cb.sheet != sheet {
            return;
        }
        self.draw_dashed_range(
            frame,
            cb.range.normalized(),
            PaintColor::from_theme_str(&frame.theme.selection_color),
            DashFill::Outline,
        );
    }

    /// Point-mode range highlight - blue dashed outline with an 8% fill tint.
    pub(super) fn draw_point_overlay(&self, frame: &FrameContext, point_range: Option<RCRange>) {
        let Some(pr) = point_range else { return };
        self.draw_dashed_range(
            frame,
            pr.normalized(),
            PaintColor::from_theme_str(&frame.theme.pointing),
            DashFill::Tinted(frame.theme.pointing_tint.clone()),
        );
    }

    /// Per-reference formula highlights for the in-edit cell. One color per
    /// `color_idx` (mod the palette), off-sheet refs silently skipped.
    pub(super) fn draw_formula_ref_overlays(
        &self,
        model: &dyn CanvasModel,
        frame: &FrameContext,
        formula_refs: &[FormulaRef],
    ) {
        let sheet = model.get_selected_sheet();
        for fr in formula_refs {
            if fr.sheet_area.sheet != sheet {
                continue;
            }
            let idx = fr.color_idx % FORMULA_REF_COLORS.len();
            self.draw_dashed_range(
                frame,
                fr.sheet_area.range.normalized(),
                PaintColor::Static(FORMULA_REF_COLORS[idx]),
                DashFill::Tinted(Cow::Borrowed(FORMULA_REF_TINTS[idx])),
            );
        }
    }

    /// Dashed rectangle over `range`. Used for clipboard marching ants
    /// (`DashFill::Outline`) and point-mode / formula-ref highlights
    /// (`DashFill::Tinted`, which also draws the carried 8% fill).
    pub(super) fn draw_dashed_range(
        &self,
        frame: &FrameContext,
        range: RCRange,
        color: PaintColor,
        fill: DashFill,
    ) {
        let Some(b) = self.range_pixel_bounds(frame, range) else {
            return;
        };

        // Tint first so the dashed outline lands cleanly on top — the 8%
        // alpha would otherwise wash over the dashes and dim them.
        // `DashFill::Tinted` carries `Cow<'static, str>`; the helper
        // preserves the ptr-eq fast path for built-in themes.
        if let DashFill::Tinted(tint) = &fill {
            self.painter.rect_fill(b, PaintColor::from_theme_str(tint));
        }

        self.painter
            .rect_dashed(b, color, f64::from(DASHED_BORDER_WIDTH));
    }
}
