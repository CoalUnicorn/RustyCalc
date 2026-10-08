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

use iron_canvas_core::Point;
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

/// The label to write for the cell-text field of the link editor.
///
/// `seed` is the text the editor showed when it opened and `field` the text it
/// holds on Apply. A field the user did not change leaves the cell content
/// alone: the seed is the *formatted* value, so rewriting it as literal text
/// would replace a formula (or lose significant whitespace) that no edit
/// touched. An empty field also leaves the content alone, as it did before.
///
/// The comparison is exact. Cell content is not a URL: trimming it is a
/// normalization the user did not ask for.
pub(crate) fn label_edit(seed: &str, field: &str) -> Option<String> {
    (field != seed && !field.is_empty()).then(|| field.to_string())
}

/// Attach `link` to one cell, and report whether the cell content changed.
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
///
/// The label and the link are separate history entries — see
/// [`execute_link`] for why the host accepts that.
pub(crate) fn write_cell_link(
    m: &mut UserModel<'_>,
    sheet: u32,
    row: i32,
    column: i32,
    link: Link,
    label: Option<&str>,
) -> Result<bool, String> {
    let mut content_written = false;
    if let Some(label) = label {
        // The engine's own rule: `set_cell_link` compares the label with the
        // formatted cell value, and an unchanged label is not rewritten. A
        // write of the same text would add a history entry of its own.
        let changes = m
            .get_formatted_cell_value(sheet, row, column)
            .is_ok_and(|current| current != label);
        if changes {
            m.set_user_input(sheet, row, column, label)?;
            content_written = true;
        }
    }
    m.set_cell_link(sheet, row, column, link, None)?;
    Ok(content_written)
}

/// Events of a completed link action.
///
/// A label write changes cell content, so the values of formulas that
/// reference the cell can change. A consumer that watches only its own range
/// learns that from `CalculationUpdated` alone:
/// the anchor `CellChanged` marks the anchor's row. A link write with no label
/// change is metadata (no formula value reads a link), so the anchor-only
/// notification stays exact.
pub(crate) fn link_events(
    anchor: CellAddress,
    old_value: Option<String>,
    new_value: Option<String>,
    content_written: bool,
    selection: SheetRange,
) -> Vec<SpreadsheetEvent> {
    let mut events = vec![SpreadsheetEvent::Content(ContentEvent::CellChanged {
        address: anchor,
        old_value,
        new_value,
    })];
    if content_written {
        events.push(SpreadsheetEvent::Content(
            ContentEvent::CalculationUpdated {
                affected_sheets: vec![anchor.sheet],
            },
        ));
    }
    events.push(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged {
            sheet_area: selection,
        },
    ));
    events
}

/// Apply `action` to the current selection.
///
/// Errors are forwarded as they are: the engine's own message for a rejected
/// mutation, or a host-authored [`FORMULA_OWNED`] refusal.
///
/// A label plus link Apply costs two undo entries: the label write and the
/// link write. The engine groups them only when it receives both at once
/// (`set_cell_link(.., Some(label))`), and that order installs the link before
/// it validates the label, which leaves an untracked link after a rejection
/// (see [`write_cell_link`]). So the host writes the label first and accepts
/// the second entry rather than trade a clean rejection for one Undo step.
pub fn execute_link(
    action: &LinkAction,
    model: ModelStore,
    state: &WorkbookState,
    icv: CanvasHandle,
) -> Result<(), LinkError> {
    let anchor = model.with_value(CellAddress::from_view);
    let area = model.with_value(|m| CellArea::from_view(m).normalized());
    let plan = plan_link_action(
        action,
        area,
        anchor,
        committed_link_is_dynamic(icv, anchor.row, anchor.column),
    )?;
    let old_value = model.with_value(|m| cell_text(m, anchor));

    let mut content_written = false;
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
                    content_written =
                        write_cell_link(m, anchor.sheet, row, column, link, label.as_deref())
                            .map_err(LinkError::Engine)?;
                    Ok(())
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
    let selection = model.with_value(SheetRange::from_view);
    state.emit_events(link_events(
        anchor,
        old_value,
        new_value,
        content_written,
        selection,
    ));
    Ok(())
}

/// Follow the link committed under `point`.
///
/// `point` is the canvas-local pointer, resolved against the painted frame, so
/// the lookup reads committed canvas state (`link_at`) and never touches the
/// model unless an internal target is actually followed. A `None` link is a
/// no-op.
pub fn activate_link(
    model: ModelStore,
    state: &WorkbookState,
    icv: CanvasHandle,
    point: Point,
) -> Result<(), LinkError> {
    let Some(link) = with_canvas(icv, |h| h.link_at(point).cloned()).flatten() else {
        return Ok(());
    };
    match link.link {
        Link::External { target, .. } => open_external(&target),
        Link::Internal { location, .. } => navigate_internal(model, state, &location),
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

/// True when the committed link attached to `(row, column)` is owned by a
/// formula. The scene exposes links by canvas point, so the cell's committed
/// rect supplies the probe point; a cell that is not in the painted frame has
/// no committed link to protect.
pub(crate) fn committed_link_is_dynamic(icv: CanvasHandle, row: i32, column: i32) -> bool {
    with_canvas(icv, |h| {
        h.cell_rect(row, column).and_then(|rect| {
            let center = rect.center();
            h.link_at(center).map(|link| link.dynamic)
        })
    })
    .flatten()
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
