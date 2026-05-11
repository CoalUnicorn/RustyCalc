//! Formatting actions: bold, italic, underline, strikethrough, font size/family.

use ironcalc_base::{
    types::{HorizontalAlignment, VerticalAlignment},
    UserModel,
};
use leptos::prelude::WithValue;

use crate::coord::{CellArea, SheetRange};
use crate::events::{FormatEvent, SpreadsheetEvent};
use crate::input::error::FormatError;
use crate::model::{
    style_types::{BooleanValue, HexColor, StylePath},
    try_mutate, EvaluationMode, FrontendModel, SafeFontFamily, ToolbarState,
};
use iron_canvas::geometry::constants::{LAST_COLUMN, LAST_ROW};

use crate::state::{ModelStore, WorkbookState};

/// Check if the selection area covers the entire sheet.
/// Returns true for whole-sheet selections (corner click + select all).
fn is_whole_sheet_selected(area: &ironcalc_base::expressions::types::Area) -> bool {
    area.row == 1 && area.column == 1 && area.height == LAST_ROW && area.width == LAST_COLUMN
}

/// Formatting mutations applied to the current selection.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatAction {
    ToggleBold,
    ToggleItalic,
    ToggleUnderline,
    ToggleStrikethrough,
    /// Clamped to 1–409 in `execute_format()`.
    SetFontSize(f64),
    SetFontFamily(SafeFontFamily),
    SetTextColor(HexColor),
    SetBackgroundColor(HexColor),
    /// Apply a number format code to the selection. `"general"` resets to auto.
    SetNumFmt(String),
    /// Reset all formatting (font, color, borders, number format) on the selection.
    ClearFormatting,
    /// Set horizontal text alignment. `General` resets to auto (numbers right, text left).
    SetHorizontalAlign(HorizontalAlignment),
    /// Set vertical cell alignment. `Bottom` is the ironcalc default.
    SetVerticalAlign(VerticalAlignment),
    /// Add one decimal place to the active cell's number format.
    IncreaseDecimals,
    /// Remove one decimal place from the active cell's number format.
    DecreaseDecimals,
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
            let sa = model.with_value(SheetRange::from_view);

            /*
            FIXME:  how we handle cell area / columns / rows selection with different
                    font sizing. How excel handle this?
                1.  Currently if selection includes empty cell default size 13px and bigger we decrement,
                    it will throw console err like:
                    [ironcalc] set_font_size: Invalid value for font size: '-43'.
                    [ironcalc] set_font_size: Invalid value for font size: '0'.
            */
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = m.selection();
                    let val = format!(
                        "{}",
                        size as i32 - m.toolbar_state().style.font_size.round() as i32
                    );
                    m.update_range_style(&area, StylePath::FONT_SIZE_DELTA.as_str(), &val)
                        .map_err(FormatError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                area: sa.area.normalized().with_sheet(sa.sheet),
            }));
        }
        FormatAction::SetFontFamily(family) => {
            let name = family.model_name();
            let sa = model.with_value(SheetRange::from_view);

            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> { set_font_name(m, name) },
            )?;

            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                area: sa.area.normalized().with_sheet(sa.sheet),
            }));
        }
        FormatAction::SetTextColor(hex) => {
            // IronCalc "font.color": empty string clears (-> transparent), hex string sets.
            // Uses the same update_range_style path as bold/italic/size for proper
            // style-pool persistence and XLSX round-trip.
            let sa = model.with_value(SheetRange::from_view);
            let value = hex.as_str();
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = m.selection();
                    m.update_range_style(&area, StylePath::TEXT_COLOR.as_str(), value)
                        .map_err(FormatError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                area: sa.area.normalized().with_sheet(sa.sheet),
            }));
        }
        FormatAction::SetBackgroundColor(hex) => {
            // IronCalc "fill.fg_color": empty string clears, hex string sets.
            // IronCalc automatically sets pattern_type = "solid" when a color is given.
            let sa = model.with_value(SheetRange::from_view);
            let value = hex.as_str();

            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = m.selection();
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
                area: sa.area.normalized().with_sheet(sa.sheet),
            }));
        }
        FormatAction::SetNumFmt(code) => {
            let sa = model.with_value(SheetRange::from_view);
            let code = code.clone();
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = m.selection();
                    m.update_range_style(&area, "num_fmt", &code)
                        .map_err(FormatError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                area: sa.area.normalized().with_sheet(sa.sheet),
            }));
        }
        FormatAction::SetHorizontalAlign(align) => {
            // HorizontalAlignment::Display gives the exact lowercase string ironcalc expects.
            let value = align.to_string();
            let sa = model.with_value(SheetRange::from_view);
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = m.selection();
                    m.update_range_style(&area, StylePath::HORIZONTAL_ALIGN.as_str(), &value)
                        .map_err(FormatError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                area: sa.area.normalized().with_sheet(sa.sheet),
            }));
        }
        FormatAction::SetVerticalAlign(align) => {
            let value = align.to_string();
            let sa = model.with_value(SheetRange::from_view);
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = m.selection();
                    m.update_range_style(&area, StylePath::VERTICAL_ALIGN.as_str(), &value)
                        .map_err(FormatError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                area: sa.area.normalized().with_sheet(sa.sheet),
            }));
        }
        FormatAction::IncreaseDecimals | FormatAction::DecreaseDecimals => {
            let delta = if matches!(action, FormatAction::IncreaseDecimals) {
                1
            } else {
                -1
            };
            let sa = model.with_value(SheetRange::from_view);
            let current = model.with_value(|m| m.active_num_fmt());
            let new_fmt = adjust_decimals(&current, delta);
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = m.selection();
                    m.update_range_style(&area, StylePath::NUM_FMT.as_str(), &new_fmt)
                        .map_err(FormatError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                area: sa.area.normalized().with_sheet(sa.sheet),
            }));
        }
        FormatAction::ClearFormatting => {
            let sa = model.with_value(SheetRange::from_view);
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), FormatError> {
                    let area = m.selection();
                    m.range_clear_formatting(&area)
                        .map_err(FormatError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
                area: sa.area.normalized().with_sheet(sa.sheet),
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
    let sa = model.with_value(SheetRange::from_view);

    try_mutate(
        model,
        EvaluationMode::Deferred,
        |m| -> Result<(), FormatError> {
            let ts = m.toolbar_state();
            let current_bool = current_val(&ts);
            let new_val = BooleanValue::from_bool(!current_bool);
            let area = m.selection();
            m.update_range_style(&area, style_path.as_str(), new_val.as_str())
                .map_err(FormatError::Engine)?;
            Ok(())
        },
    )?;

    state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged {
        area: sa.area.normalized().with_sheet(sa.sheet),
    }));
    Ok(())
}

/// Adjust the number of decimal places in a format string by `delta` (+1 or -1).
///
/// Called by `IncreaseDecimals` and `DecreaseDecimals` before applying `SetNumFmt`.
/// Must handle the most common format shapes: plain numbers (`#,##0`), percentage
/// (`0%`), currency with leading symbol (`£#,##0.00`), and the `"general"` sentinel.
fn adjust_decimals(fmt: &str, delta: i32) -> String {
    // NOTE: IronCalc PR 777 turns num_fmt from String into a NumFmt struct
    // (num_fmt_id + format_code). When that lands, this function should
    // operate on the format_code field.
    if delta == 0 {
        return fmt.to_owned();
    }
    if fmt.eq_ignore_ascii_case("general") {
        return if delta > 0 {
            "0.0".to_owned()
        } else {
            fmt.to_owned()
        };
    }

    let dot = fmt.find('.');

    if delta > 0 {
        match dot {
            Some(dot) => {
                let zeros = fmt[dot + 1..].chars().take_while(|c| *c == '0').count();
                let insert_at = dot + 1 + zeros;
                let mut out = String::with_capacity(fmt.len() + 1);
                out.push_str(&fmt[..insert_at]);
                out.push('0');
                out.push_str(&fmt[insert_at..]);
                out
            }
            None => match fmt.rfind(['0', '#']) {
                Some(pos) => {
                    let insert_at = pos + 1;
                    let mut out = String::with_capacity(fmt.len() + 2);
                    out.push_str(&fmt[..insert_at]);
                    out.push_str(".0");
                    out.push_str(&fmt[insert_at..]);
                    out
                }
                None => fmt.to_owned(),
            },
        }
    } else {
        let Some(dot) = dot else {
            return fmt.to_owned();
        };
        let zeros = fmt[dot + 1..].chars().take_while(|c| *c == '0').count();
        match zeros {
            0 => fmt.to_owned(),
            1 => {
                let mut out = String::with_capacity(fmt.len() - 2);
                out.push_str(&fmt[..dot]);
                out.push_str(&fmt[dot + 2..]);
                out
            }
            _ => {
                let last_zero = dot + zeros;
                let mut out = String::with_capacity(fmt.len() - 1);
                out.push_str(&fmt[..last_zero]);
                out.push_str(&fmt[last_zero + 1..]);
                out
            }
        }
    }
}

/// Set `font.name` on every cell in the selection.
///
/// IronCalc's `update_range_style` doesn't support `font.name`, so we
/// read each cell's style, mutate the name, and write it back via
/// `on_paste_styles` (which records undo diffs).
fn set_font_name(m: &mut UserModel<'static>, name: &str) -> Result<(), FormatError> {
    let sheet = m.get_selected_sheet();
    let norm = CellArea::from_view(m).normalized();

    let rows: Vec<Vec<_>> = (norm.r1..=norm.r2)
        .map(|row| {
            (norm.c1..=norm.c2)
                .map(|col| {
                    let mut style = m.get_cell_style(sheet, row, col).unwrap_or_default();
                    style.font.name = name.to_owned();
                    style
                })
                .collect()
        })
        .collect();
    m.on_paste_styles(&rows).map_err(FormatError::Engine)?;
    Ok(())
}
