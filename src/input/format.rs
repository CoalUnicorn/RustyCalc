//! Formatting actions: bold, italic, underline, strikethrough, font size/family.

use ironcalc_base::UserModel;
use leptos::prelude::WithValue;

use crate::canvas::geometry::{LAST_COLUMN, LAST_ROW};
use crate::events::{FormatEvent, SpreadsheetEvent};
use crate::input::error::FormatError;
use crate::input::helpers::{selection_area, try_mutate, EvaluationMode};
use crate::model::{
    style_types::{BooleanValue, HexColor, StylePath},
    FrontendModel, SafeFontFamily, ToolbarState,
};

use crate::state::{ModelStore, WorkbookState};

/// Check if the selection area covers the entire sheet.
/// Returns true for whole-sheet selections (corner click + select all).
fn is_whole_sheet_selected(area: &ironcalc_base::expressions::types::Area) -> bool {
    area.row == 1 && area.column == 1 && area.height == LAST_ROW && area.width == LAST_COLUMN
}

/// Formatting mutations applied to the current selection.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatAction {
    /// Ctrl+B: toggle bold on the selected range.
    ToggleBold,
    /// Ctrl+I: toggle italic on the selected range.
    ToggleItalic,
    /// Ctrl+U: toggle underline on the selected range.
    ToggleUnderline,
    /// Toggle strikethrough on the selected range.
    ToggleStrikethrough,
    /// Set font size (in points) on the selected range.
    /// Clamped to 1–409 in `execute_format()`.
    SetFontSize(f64),
    /// Set font family on the selected range.
    SetFontFamily(SafeFontFamily),
    /// Set text (font) color. `HexColor::transparent()` resets to automatic.
    SetTextColor(HexColor),
    /// Set cell background fill color. `HexColor::transparent()` clears the fill.
    SetBackgroundColor(HexColor),
}

/// Dispatch a [`FormatAction`] against the model and UI state.
pub fn execute_format(
    action: &FormatAction,
    model: ModelStore,
    state: &WorkbookState,
) -> Result<(), FormatError> {
    match action {
        FormatAction::ToggleBold => {
            toggle_style(model, state, StylePath::FONT_BOLD, |ts| ts.format.bold)?;
        }
        FormatAction::ToggleItalic => {
            toggle_style(model, state, StylePath::FONT_ITALIC, |ts| ts.format.italic)?;
        }
        FormatAction::ToggleUnderline => {
            toggle_style(model, state, StylePath::FONT_UNDERLINE, |ts| {
                ts.format.underline
            })?;
        }
        FormatAction::ToggleStrikethrough => {
            toggle_style(model, state, StylePath::FONT_STRIKETHROUGH, |ts| {
                ts.format.strikethrough
            })?;
        }
        FormatAction::SetFontSize(size) => {
            let size = size.clamp(1.0, 409.0);
            let (sheet, start_row, start_col, end_row, end_col) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let area = selection_area(m);
                    (
                        area.sheet,
                        area.row,
                        area.column,
                        area.row + area.height - 1,
                        area.column + area.width - 1,
                    )
                });

            /*
            FIXME:  how we handle cell area / columns / rows selection with different
                    font sizing. How excel handle this?
                1.  Currently if selection includes empty cell default size 13px and bigger we decrement,
                    it will throw console err like:
                    [ironcalc] set_font_size: Invalid value for font size: '-43'.
                    [ironcalc] set_font_size: Invalid value for font size: '0'.
                2.  When font size goes below 10px - not able to increment with buttons
                    This may be `toolbar.rs` issue ?
            */
            try_mutate(
                model,
                state,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = selection_area(m);
                    let val = format!("{}", size as i32 - m.toolbar_state().style.font_size as i32);
                    m.update_range_style(&area, StylePath::FONT_SIZE_DELTA.as_str(), &val)
                        .map_err(FormatError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            }));
        }
        FormatAction::SetFontFamily(family) => {
            let name = family.model_name();
            let (sheet, start_row, start_col, end_row, end_col) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let area = selection_area(m);
                    (
                        area.sheet,
                        area.row,
                        area.column,
                        area.row + area.height - 1,
                        area.column + area.width - 1,
                    )
                });

            try_mutate(
                model,
                state,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> { set_font_name(m, name) },
            )?;

            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            }));
        }
        FormatAction::SetTextColor(hex) => {
            // IronCalc "font.color": empty string clears (-> transparent), hex string sets.
            // Uses the same update_range_style path as bold/italic/size for proper
            // style-pool persistence and XLSX round-trip.
            let (sheet, start_row, start_col, end_row, end_col) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let a = selection_area(m);
                    (
                        a.sheet,
                        a.row,
                        a.column,
                        a.row + a.height - 1,
                        a.column + a.width - 1,
                    )
                });
            let value = hex.as_str();
            try_mutate(
                model,
                state,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = selection_area(m);
                    m.update_range_style(&area, StylePath::TEXT_COLOR.as_str(), value)
                        .map_err(FormatError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            }));
        }
        FormatAction::SetBackgroundColor(hex) => {
            // IronCalc "fill.fg_color": empty string clears, hex string sets.
            // IronCalc automatically sets pattern_type = "solid" when a color is given.
            let (sheet, start_row, start_col, end_row, end_col) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let a = selection_area(m);
                    (
                        a.sheet,
                        a.row,
                        a.column,
                        a.row + a.height - 1,
                        a.column + a.width - 1,
                    )
                });
            let value = hex.as_str();

            try_mutate(
                model,
                state,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = selection_area(m);
                    // NOTE: questionable - may not need
                    // **PERFORMANCE OPTIMIZATION**: IronCalc has optimizations for full-column and full-row
                    // ranges, but NOT for full-sheet ranges. Whole-sheet selections fall into the
                    // unoptimized O(rowsxcolumns) path. Fix this by applying column-by-column.
                    if is_whole_sheet_selected(&area) {
                        // Fast path: Apply to all columns individually (O(columns) instead of O(rowsxcolumns))
                        // Each column operation is optimized by IronCalc's full-column logic
                        (1..=LAST_COLUMN).try_for_each(|col| {
                            m.update_range_style(
                                &ironcalc_base::expressions::types::Area {
                                    sheet: area.sheet,
                                    row: 1,
                                    column: col,
                                    height: LAST_ROW,
                                    width: 1,
                                },
                                StylePath::BACKGROUND_COLOR.as_str(),
                                value,
                            )
                            .map_err(FormatError::Engine)
                        })?;
                    } else {
                        // Slow path: O(rows x columns) cell-by-cell styling for partial selections
                        m.update_range_style(&area, StylePath::BACKGROUND_COLOR.as_str(), value)
                            .map_err(FormatError::Engine)?;
                    }
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            }));
        }
    }
    Ok(())
}

/// Toggle a boolean style property on the selected range.
///
/// Reads the current value from `ToolbarState` (active cell) via `current_val`,
/// then sets the opposite on the full selection via `update_range_style`.
fn toggle_style(
    model: ModelStore,
    state: &WorkbookState,
    style_path: StylePath,
    current_val: fn(&ToolbarState) -> bool,
) -> Result<(), FormatError> {
    let (sheet, start_row, start_col, end_row, end_col) =
        model.with_value(|m: &ironcalc_base::UserModel<'static>| {
            let area = selection_area(m);
            (
                area.sheet,
                area.row,
                area.column,
                area.row + area.height - 1,
                area.column + area.width - 1,
            )
        });

    try_mutate(
        model,
        state,
        EvaluationMode::Deferred,
        |m| -> Result<(), FormatError> {
            let ts = m.toolbar_state();
            let current_bool = current_val(&ts);
            let new_val = BooleanValue::from_bool(!current_bool);
            let area = selection_area(m);
            m.update_range_style(&area, style_path.as_str(), new_val.as_str())
                .map_err(FormatError::Engine)?;
            Ok(())
        },
    )?;

    state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
        sheet,
        start_row,
        start_col,
        end_row,
        end_col,
    }));
    Ok(())
}

/// Set `font.name` on every cell in the selection.
///
/// IronCalc's `update_range_style` doesn't support `font.name`, so we
/// read each cell's style, mutate the name, and write it back via
/// `on_paste_styles` (which records undo diffs).
fn set_font_name(m: &mut UserModel<'static>, name: &str) -> Result<(), FormatError> {
    let v = m.get_selected_view();
    let [r1, c1, r2, c2] = v.range;
    let (r_min, r_max) = (r1.min(r2), r1.max(r2));
    let (c_min, c_max) = (c1.min(c2), c1.max(c2));

    let rows: Vec<Vec<_>> = (r_min..=r_max)
        .map(|row| {
            (c_min..=c_max)
                .map(|col| {
                    let mut style = m.get_cell_style(v.sheet, row, col).unwrap_or_default();
                    style.font.name = name.to_owned();
                    style
                })
                .collect()
        })
        .collect();
    m.on_paste_styles(&rows).map_err(FormatError::Engine)?;
    Ok(())
}
