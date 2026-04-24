//! Phase-3 overlays: selection rectangle, autofill drag preview, clipboard
//! marching ants, point-mode range, formula-ref highlights.
//!
//! Everything here is drawn *after* cell backgrounds + headers but *before*
//! cell text (Phase 4). Each helper bails early via
//! `range_pixel_bounds(...)?` when the range is entirely outside the
//! drawable fold, so overlays never leak onto the canvas for off-screen
//! refs like `=BB3`.

use ironcalc_base::UserModel;

use crate::canvas::Point;
use crate::coord::{CellAddress, CellArea, FormulaRef, SheetArea};
use crate::theme::FORMULA_REF_COLORS;

use super::super::geometry::{PixelRect, AUTOFILL_HANDLE_PX};
use super::super::types::{AutofillTarget, DashFill, FrozenOffset};
use super::{CanvasRenderer, DASHED_BORDER_WIDTH, SELECTION_BORDER_WIDTH, STANDARD_BORDER_WIDTH};

impl CanvasRenderer {
    /// Draw the blue selection border, semi-transparent fill, and autofill
    /// handle for the current selection.
    pub(super) fn draw_selection(&self, model: &UserModel, frozen: &FrozenOffset) {
        let view = model.get_selected_view();
        let sheet = model.get_selected_sheet();
        let addr = CellAddress {
            sheet,
            row: view.row,
            column: view.column,
        };
        let Some(b) =
            self.range_pixel_bounds(model, frozen, CellArea::from_view(model).normalized())
        else {
            return;
        };

        self.rect_fill(b, self.theme.selection_fill);

        // Restore the active cell's fill + borders on top of the selection
        // tint so its actual style shows through while selected. Phase 4
        // paints text over everything later.
        self.repaint_active_cell(model, addr, frozen);

        self.rect_stroke(b, self.theme.selection_color, SELECTION_BORDER_WIDTH);

        let handle = PixelRect {
            top_left: Point {
                x: b.right() - AUTOFILL_HANDLE_PX / 2.0,
                y: b.bottom() - AUTOFILL_HANDLE_PX / 2.0,
            },
            width: AUTOFILL_HANDLE_PX,
            height: AUTOFILL_HANDLE_PX,
        };
        self.rect_fill(handle, self.theme.selection_color);
    }

    /// Dashed preview of the autofill-handle drag target.
    pub(super) fn draw_extend_preview(
        &self,
        model: &UserModel,
        frozen: &FrozenOffset,
        target: AutofillTarget,
    ) {
        let sel = CellArea::from_view(model).normalized();
        let range = CellArea {
            r1: sel.r1.min(target.row),
            c1: sel.c1.min(target.col),
            r2: sel.r2.max(target.row),
            c2: sel.c2.max(target.col),
        };
        let Some(b) = self.range_pixel_bounds(model, frozen, range) else {
            return;
        };

        self.rect_dashed(b, self.theme.selection_color, STANDARD_BORDER_WIDTH);
    }

    /// Clipboard marching-ants border around the last Ctrl+C copied range.
    /// No-op when the clipboard is empty or lives on another sheet.
    pub(super) fn draw_clipboard_overlay(
        &self,
        model: &UserModel,
        frozen: &FrozenOffset,
        clipboard: Option<&SheetArea>,
    ) {
        let sheet = model.get_selected_sheet();
        let Some(cb) = clipboard else { return };
        if cb.sheet != sheet {
            return;
        }
        self.draw_dashed_range(
            model,
            frozen,
            cb.area.normalized(),
            self.theme.selection_color,
            DashFill::Outline,
        );
    }

    /// Point-mode range highlight — blue dashed outline with an 8% fill tint.
    pub(super) fn draw_point_overlay(
        &self,
        model: &UserModel,
        frozen: &FrozenOffset,
        point_range: Option<CellArea>,
    ) {
        let Some(pr) = point_range else { return };
        self.draw_dashed_range(
            model,
            frozen,
            pr.normalized(),
            self.theme.pointing,
            DashFill::Tinted,
        );
    }

    /// Per-reference formula highlights for the in-edit cell. One color per
    /// `color_idx` (mod the palette), off-sheet refs silently skipped.
    pub(super) fn draw_formula_ref_overlays(
        &self,
        model: &UserModel,
        frozen: &FrozenOffset,
        refs: &[FormulaRef],
    ) {
        let sheet = model.get_selected_sheet();
        for fr in refs {
            if fr.sheet_area.sheet != sheet {
                continue;
            }
            self.draw_dashed_range(
                model,
                frozen,
                fr.sheet_area.area.normalized(),
                FORMULA_REF_COLORS[fr.color_idx % FORMULA_REF_COLORS.len()],
                DashFill::Tinted,
            );
        }
    }

    /// Dashed rectangle over `range`. Used for clipboard marching ants
    /// (`DashFill::Outline`) and point-mode / formula-ref highlights
    /// (`DashFill::Tinted`, which also draws an 8% fill).
    pub(super) fn draw_dashed_range(
        &self,
        model: &UserModel,
        frozen: &FrozenOffset,
        range: CellArea,
        color: &str,
        fill: DashFill,
    ) {
        let Some(b) = self.range_pixel_bounds(model, frozen, range) else {
            return;
        };

        self.rect_dashed(b, color, DASHED_BORDER_WIDTH);

        if fill == DashFill::Tinted {
            let tint = hex_to_rgba(color, 0.08);
            self.rect_fill(b, &tint);
        }
    }
}

/// Convert a 6-digit hex color (`"#1E6FD9"`) to an `rgba(...)` CSS string
/// with the given alpha. Falls back to transparent black on malformed input.
fn hex_to_rgba(hex: &str, alpha: f64) -> String {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return format!("rgba(0,0,0,{alpha})");
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    format!("rgba({r},{g},{b},{alpha})")
}
