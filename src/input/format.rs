//! Formatting actions: bold, italic, underline, strikethrough, font size/family.

use ironcalc_base::UserModel;
use leptos::prelude::WithValue;

use crate::input::helpers::{mutate, selection_area, Eval};
use crate::model::{FrontendModel, SafeFontFamily, ToolbarState};
use crate::state::{ModelStore, WorkbookState};
use crate::util::warn_if_err;

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
}

pub fn execute_format(action: &FormatAction, model: ModelStore, state: &WorkbookState) {
    match action {
        FormatAction::ToggleBold => {
            toggle_style(model, state, "font.b", |ts| ts.format.bold);
        }
        FormatAction::ToggleItalic => {
            toggle_style(model, state, "font.i", |ts| ts.format.italic);
        }
        FormatAction::ToggleUnderline => {
            toggle_style(model, state, "font.u", |ts| ts.format.underline);
        }
        FormatAction::ToggleStrikethrough => {
            toggle_style(model, state, "font.strike", |ts| ts.format.strikethrough);
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
            mutate(model, state, Eval::No, |m| {
                let area = selection_area(m);
                let val = format!("{}", size as i32 - m.toolbar_state().style.font_size as i32);
                warn_if_err(
                    m.update_range_style(&area, "font.size_delta", &val),
                    "set_font_size",
                );
            });

            // Fire format event for font size change
            state.emit_event(crate::events::SpreadsheetEvent::Format(
                crate::events::FormatEvent::RangeStyleChanged {
                    sheet,
                    start_row,
                    start_col,
                    end_row,
                    end_col,
                },
            ));
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

            mutate(model, state, Eval::No, |m| {
                set_font_name(m, name);
            });

            // Fire format event for font family change
            state.emit_event(crate::events::SpreadsheetEvent::Format(
                crate::events::FormatEvent::RangeStyleChanged {
                    sheet,
                    start_row,
                    start_col,
                    end_row,
                    end_col,
                },
            ));
        } // SetFontColor
          //
    }
}

/// Toggle a boolean style property on the selected range.
///
/// Reads the current value from `ToolbarState` (active cell) via `current_val`,
/// then sets the opposite on the full selection via `update_range_style`.
///
/// `style_path` is an IronCalc `update_range_style` key (e.g. `"font.b"`) —
/// foreign string API, not something we can type as an enum.
fn toggle_style(
    model: ModelStore,
    state: &WorkbookState,
    style_path: &str,
    current_val: fn(&ToolbarState) -> bool,
) {
    let path = style_path.to_owned();
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

    mutate(model, state, Eval::No, |m| {
        let ts = m.toolbar_state();
        let new_val = if current_val(&ts) { "false" } else { "true" };
        let area = selection_area(m);
        warn_if_err(m.update_range_style(&area, &path, new_val), &path);
    });

    // Fire format event for style toggle
    state.emit_event(crate::events::SpreadsheetEvent::Format(
        crate::events::FormatEvent::RangeStyleChanged {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        },
    ));
}

/// Set `font.name` on every cell in the selection.
///
/// IronCalc's `update_range_style` doesn't support `font.name`, so we
/// read each cell's style, mutate the name, and write it back via
/// `on_paste_styles` (which records undo diffs).
fn set_font_name(m: &mut UserModel<'static>, name: &str) {
    let v = m.get_selected_view();
    let [r1, c1, r2, c2] = v.range;
    let (r_min, r_max) = (r1.min(r2), r1.max(r2));
    let (c_min, c_max) = (c1.min(c2), c1.max(c2));

    let mut rows = Vec::new();
    for row in r_min..=r_max {
        let mut cols = Vec::new();
        for col in c_min..=c_max {
            let mut style = m.get_cell_style(v.sheet, row, col).unwrap_or_default();
            style.font.name = name.to_owned();
            cols.push(style);
        }
        rows.push(cols);
    }
    warn_if_err(m.on_paste_styles(&rows), "set_font_name");
}
