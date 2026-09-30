//! Cell hyperlink actions: set or delete a link on the selected cell, and
//! follow the link under a modified click.
//!
//! Mirrors `structure.rs`: a pure action enum resolved against the model. Two
//! differences matter. A link can be owned by a formula (`HYPERLINK`) — such a
//! link is "dynamic" (`CellLink::is_dynamic`) and the host refuses to edit or
//! delete it, because the next recalculation would recreate it anyway. And an
//! action touches exactly one cell: the engine records one undo entry per cell,
//! so a selection-wide action could not be undone in one step, and the work it
//! needs is unbounded.

use leptos::prelude::*;

use crate::coord::{CellAddress, CellArea, SheetRange};
use crate::events::{ContentEvent, NavigationEvent, SpreadsheetEvent};
use crate::input::error::LinkError;
use crate::input::mouse::{CanvasHandle, with_canvas};
use crate::model::{EvaluationMode, FormulaAnalyzer, Navigator, try_mutate};
use crate::state::{ModelStore, WorkbookState};

use iron_canvas_core::LinkTarget;
use ironcalc_base::UserModel;
use ironcalc_base::types::Link;

/// Destination kind of a [`LinkAction::SetLink`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// A URL or `mailto:` URI.
    External,
    /// A cell reference (`Sheet1!A30`) or a defined name in this workbook.
    Internal,
}

/// Link mutations, applied to the selected cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAction {
    /// Attach a link to the selected cell. `label`, when given, becomes the
    /// content of that cell.
    SetLink {
        kind: LinkKind,
        target_or_location: String,
        tooltip: Option<String>,
        label: Option<String>,
    },
    /// Remove the link from the selected cell. Cell content and formatting
    /// stay untouched.
    DeleteLink,
}

/// Shown whenever a formula owns the link: it cannot be edited or deleted.
pub const FORMULA_OWNED: &str =
    "This hyperlink is created by a formula. Edit the formula to change or delete it.";

/// Shown whenever the selection covers more than one cell.
pub const SINGLE_CELL_ONLY: &str =
    "Select one cell. A hyperlink is set or removed one cell at a time.";

/// A link action resolved against the selection.
#[derive(Clone, Debug)]
pub(crate) enum LinkPlan {
    Set {
        row: i32,
        column: i32,
        link: Link,
        label: Option<String>,
    },
    Delete {
        row: i32,
        column: i32,
    },
}

/// Resolve `action` against the selection.
///
/// `area` is the normalized selection and `anchor` its active cell.
/// `formula_owned` reports whether the committed link of `anchor` is a
/// formula's — such a link must not be replaced or removed, because the next
/// recalculation would recreate it.
pub(crate) fn plan_link_action(
    action: &LinkAction,
    area: CellArea,
    anchor: CellAddress,
    formula_owned: bool,
) -> Result<LinkPlan, LinkError> {
    if !area.is_single_cell() {
        return Err(LinkError::Refused(SINGLE_CELL_ONLY.to_string()));
    }
    if formula_owned {
        return Err(LinkError::Refused(FORMULA_OWNED.to_string()));
    }
    let (row, column) = (anchor.row, anchor.column);
    Ok(match action {
        LinkAction::SetLink {
            kind,
            target_or_location,
            tooltip,
            label,
        } => LinkPlan::Set {
            row,
            column,
            link: match kind {
                LinkKind::External => Link::External {
                    target: target_or_location.clone(),
                    tooltip: tooltip.clone(),
                },
                LinkKind::Internal => Link::Internal {
                    location: target_or_location.clone(),
                    tooltip: tooltip.clone(),
                },
            },
            label: label.clone(),
        },
        LinkAction::DeleteLink => LinkPlan::Delete { row, column },
    })
}

/// Attach `link` to one cell.
///
/// The label is written first. The engine's `set_cell_link` writes the link
/// before it checks whether the label may replace the cell content, and it
/// returns that rejection without recording the link write — so a rejected
/// label would leave an untracked link behind, and the model's undo entry would
/// never name it. The label write is the only step that can reject a cell
/// (part of an array formula, a spill cell, or a merged cell), and it rejects
/// the cell before mutating anything, so it runs first: either it fails with
/// link, content, style and history untouched, or it succeeds and the link
/// write that follows can only fail on an invalid address.
pub(crate) fn write_cell_link(
    m: &mut UserModel<'_>,
    sheet: u32,
    row: i32,
    column: i32,
    link: Link,
    label: Option<&str>,
) -> Result<(), String> {
    if let Some(label) = label {
        // The engine's own rule: `set_cell_link` compares the label with the
        // formatted cell value, and an unchanged label is not rewritten. A
        // write of the same text would add a history entry of its own.
        let changes = m
            .get_formatted_cell_value(sheet, row, column)
            .is_ok_and(|current| current != label);
        if changes {
            m.set_user_input(sheet, row, column, label)?;
        }
    }
    m.set_cell_link(sheet, row, column, link, None)
}

/// Apply `action` to the current selection.
///
/// Errors are forwarded as they are: the engine's own message for a rejected
/// mutation, or a host-authored [`FORMULA_OWNED`] refusal.
pub fn execute_link(
    action: &LinkAction,
    model: ModelStore,
    state: &WorkbookState,
    icv: CanvasHandle,
) -> Result<(), LinkError> {
    let anchor = model.with_value(CellAddress::from_view);
    let area = model.with_value(|m| CellArea::from_view(m).normalized());
    let plan = plan_link_action(action, area, anchor, link_is_dynamic(icv, anchor))?;
    let old_value = model.with_value(|m| cell_text(m, anchor));

    match plan {
        LinkPlan::Set {
            row,
            column,
            link,
            label,
        } => {
            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), LinkError> {
                    write_cell_link(m, anchor.sheet, row, column, link, label.as_deref())
                        .map_err(LinkError::Engine)
                },
            )?;
        }
        LinkPlan::Delete { row, column } => {
            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), LinkError> {
                    m.delete_cell_link(anchor.sheet, row, column)
                        .map_err(LinkError::Engine)
                },
            )?;
        }
    }

    let new_value = model.with_value(|m| cell_text(m, anchor));
    state.emit_events([
        SpreadsheetEvent::Content(ContentEvent::CellChanged {
            address: anchor,
            old_value,
            new_value,
        }),
        SpreadsheetEvent::Navigation(NavigationEvent::SelectionRangeChanged {
            sheet_area: model.with_value(SheetRange::from_view),
        }),
    ]);
    Ok(())
}

/// Follow the link committed at `(row, column)`.
///
/// `row` / `column` come from the painted-frame hit test, so the lookup reads
/// committed canvas state (`link_at`) and never touches the model unless an
/// internal target is actually followed. A `None` link is a no-op.
pub fn activate_link(
    model: ModelStore,
    state: &WorkbookState,
    icv: CanvasHandle,
    row: i32,
    column: i32,
) -> Result<(), LinkError> {
    let Some(link) = with_canvas(icv, |ic| ic.link_at(row, column)).flatten() else {
        return Ok(());
    };
    match link.target() {
        LinkTarget::External(url) => open_external(url),
        LinkTarget::Internal(location) => navigate_internal(model, state, location),
    }
}

/// Open `target` in a new tab when its scheme is on the allowlist.
///
/// The allowlist is explicit on purpose: a workbook is untrusted input, and
/// `javascript:` / `data:` targets must never reach `window.open`.
fn open_external(target: &str) -> Result<(), LinkError> {
    let Some(url) = allowed_external_url(target) else {
        return Err(LinkError::Refused(format!(
            "\"{target}\" is not an openable link: only http, https and mailto are supported"
        )));
    };
    window()
        .open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer")
        .map_err(|_| LinkError::Refused(format!("Could not open \"{url}\"")))?;
    Ok(())
}

/// The trimmed target when its scheme is on the allowlist.
pub(crate) fn allowed_external_url(target: &str) -> Option<&str> {
    let trimmed = target.trim();
    let (scheme, rest) = trimmed.split_once(':')?;
    let valid_scheme = !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid_scheme || rest.is_empty() {
        return None;
    }
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "mailto"
    )
    .then_some(trimmed)
}

/// Select the reference or defined name `location` resolves to.
///
/// Resolution goes through the host's formula analyzer — the grammar a formula
/// cell already uses — so `Sheet1!A30`, `$B$2:$D$8` and defined names all
/// resolve without a second parser.
fn navigate_internal(
    model: ModelStore,
    state: &WorkbookState,
    location: &str,
) -> Result<(), LinkError> {
    let anchor = model.with_value(CellAddress::from_view);
    let from_sheet = anchor.sheet;
    let Some(sheet_area) = model.with_value(|m| resolve_internal_location(m, location, anchor))
    else {
        return Err(LinkError::Refused(format!(
            "\"{location}\" does not resolve to a cell reference"
        )));
    };

    try_mutate(
        model,
        EvaluationMode::Deferred,
        |m| -> Result<(), LinkError> {
            // The sheet first: `set_selected_area` resolves against the sheet that
            // is selected when it runs.
            m.set_selected_sheet(sheet_area.sheet)
                .map_err(LinkError::Engine)?;
            m.set_selected_area(sheet_area.area);
            Ok(())
        },
    )?;

    state.scroll_into_view.set_value(true);
    let mut events = Vec::new();
    if sheet_area.sheet != from_sheet {
        events.push(SpreadsheetEvent::Navigation(
            NavigationEvent::ActiveSheetChanged {
                from_sheet,
                to_sheet: sheet_area.sheet,
            },
        ));
    }
    events.push(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged { sheet_area },
    ));
    state.emit_events(events);
    Ok(())
}

/// True when the committed link of `address` is owned by a formula. One
/// committed lookup: the link index is sparse, so no cell scan is needed.
/// Without a mounted canvas there is no committed link state, hence nothing
/// to protect.
fn link_is_dynamic(icv: CanvasHandle, address: CellAddress) -> bool {
    with_canvas(icv, |ic| ic.link_at(address.row, address.column))
        .flatten()
        .is_some_and(|link| link.is_dynamic())
}

/// Resolve a link `location` to the first reference it names.
///
/// The analyzer only parses `=...` text, so the location is wrapped once here;
/// everything else stays the shared reference grammar. `anchor` supplies the
/// context for a bare reference (`A1`) or a relative one.
pub(crate) fn resolve_internal_location(
    m: &UserModel<'_>,
    location: &str,
    anchor: CellAddress,
) -> Option<SheetRange> {
    let text = format!("={location}");
    m.analyze_at(&text, anchor)
        .refs()
        .first()
        .map(|active| active.sheet_area)
}

/// Displayed text of a cell, or `None` when the engine cannot report it.
fn cell_text(m: &UserModel<'_>, address: CellAddress) -> Option<String> {
    m.get_formatted_cell_value(address.sheet, address.row, address.column)
        .ok()
}
