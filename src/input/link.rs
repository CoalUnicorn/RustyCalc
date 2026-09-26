//! Cell hyperlink actions: set or delete a link on the selection, and follow
//! the link under a modified click.
//!
//! Mirrors `structure.rs`: a pure action enum resolved against the model. One
//! difference matters: a link can be owned by a formula (`HYPERLINK`) — such a
//! link is "dynamic" (`CellLink::is_dynamic`) and the host refuses to edit or
//! delete it, because the next recalculation would recreate it anyway.

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

/// Link mutations, applied to the current selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAction {
    /// Attach a link to every cell of the selection. `label` becomes the
    /// content of the cell, so it applies only when the selection is a single
    /// cell — one shared label would flatten a multi-cell selection.
    SetLink {
        kind: LinkKind,
        target_or_location: String,
        tooltip: Option<String>,
        label: Option<String>,
    },
    /// Remove the link from every cell of the selection. Cell content and
    /// formatting stay untouched.
    DeleteLink,
}

/// Shown whenever a formula owns the link: it cannot be edited or deleted.
pub const FORMULA_OWNED: &str =
    "This hyperlink is created by a formula. Edit the formula to change or delete it.";

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

    if selection_has_dynamic_link(icv, area) {
        return Err(LinkError::Refused(FORMULA_OWNED.to_string()));
    }

    let label_applies = area.is_single_cell();
    let old_value = model.with_value(|m| cell_text(m, anchor));

    match action {
        LinkAction::SetLink {
            kind,
            target_or_location,
            tooltip,
            label,
        } => {
            let link = match kind {
                LinkKind::External => Link::External {
                    target: target_or_location.clone(),
                    tooltip: tooltip.clone(),
                },
                LinkKind::Internal => Link::Internal {
                    location: target_or_location.clone(),
                    tooltip: tooltip.clone(),
                },
            };
            let label = if label_applies {
                label.as_deref()
            } else {
                None
            };
            try_mutate(model, EvaluationMode::Immediate, |m| -> Result<(), LinkError> {
                for (row, column) in area.cells() {
                    m.set_cell_link(anchor.sheet, row, column, link.clone(), label)
                        .map_err(LinkError::Engine)?;
                }
                Ok(())
            })?;
        }
        LinkAction::DeleteLink => {
            try_mutate(model, EvaluationMode::Immediate, |m| -> Result<(), LinkError> {
                for (row, column) in area.cells() {
                    m.delete_cell_link(anchor.sheet, row, column)
                        .map_err(LinkError::Engine)?;
                }
                Ok(())
            })?;
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
    let Some(sheet_area) = model.with_value(|m| resolve_internal_location(m, location, anchor)) else {
        return Err(LinkError::Refused(format!(
            "\"{location}\" does not resolve to a cell reference"
        )));
    };

    try_mutate(model, EvaluationMode::Deferred, |m| -> Result<(), LinkError> {
        // The sheet first: `set_selected_area` resolves against the sheet that
        // is selected when it runs.
        m.set_selected_sheet(sheet_area.sheet)
            .map_err(LinkError::Engine)?;
        m.set_selected_area(sheet_area.area);
        Ok(())
    })?;

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

/// True when any cell of `area` carries a link a formula owns. Without a
/// mounted canvas there is no committed link state, hence nothing to protect.
fn selection_has_dynamic_link(icv: CanvasHandle, area: CellArea) -> bool {
    with_canvas(icv, |ic| {
        area.cells()
            .any(|(row, column)| ic.link_at(row, column).is_some_and(|l| l.is_dynamic()))
    })
    .unwrap_or(false)
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
