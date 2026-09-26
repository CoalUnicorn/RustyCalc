//! Tests for the hyperlink actions in `src/input/link.rs`.
//!
//! The URL allowlist and the internal-target resolution are the two pieces of
//! host logic that decide whether a workbook's link can leave the browser or
//! move the selection; both are pure and tested here without a DOM.

use crate::coord::{CellAddress, CellArea, SheetRange};
use crate::input::error::LinkError;
use crate::input::link::{
    FORMULA_OWNED, LinkAction, LinkKind, LinkPlan, SINGLE_CELL_ONLY, allowed_external_url,
    plan_link_action, resolve_internal_location, write_cell_link,
};
use ironcalc_base::UserModel;
use ironcalc_base::types::Link;

/// Empty workbook with `Sheet1`, plus `Sheet2` for the cross-sheet cases.
fn make_model() -> UserModel<'static> {
    let mut m = UserModel::new_empty("Sheet1", "en", "UTC", "en")
        .expect("failed to create test model");
    m.new_sheet().expect("failed to add Sheet2");
    m
}

fn anchor() -> CellAddress {
    CellAddress {
        sheet: 0,
        row: 1,
        column: 1,
    }
}

#[test]
fn allowlist_accepts_http_https_mailto() {
    assert_eq!(allowed_external_url("http://a.test"), Some("http://a.test"));
    assert_eq!(
        allowed_external_url("https://a.test/x?y=1#z"),
        Some("https://a.test/x?y=1#z")
    );
    assert_eq!(allowed_external_url("HTTPS://a.test"), Some("HTTPS://a.test"));
    assert_eq!(
        allowed_external_url("mailto:a@b.test"),
        Some("mailto:a@b.test")
    );
    assert_eq!(
        allowed_external_url("  https://a.test  "),
        Some("https://a.test")
    );
}

#[test]
fn allowlist_rejects_other_schemes_and_malformed_targets() {
    for target in [
        "javascript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "file:///etc/passwd",
        "ftp://a.test",
        "sheet1!a1",
        "https:",
        "",
        "1http://a.test",
        "://x",
    ] {
        assert_eq!(allowed_external_url(target), None, "{target} must be refused");
    }
    // "https://" has an empty authority but a non-empty rest, so it passes the
    // scheme allowlist; only the scheme is vetted here. The browser then
    // refuses to load it.
    assert_eq!(allowed_external_url("https://"), Some("https://"));
}

/// A qualified cross-sheet location resolves on the named sheet, not on the
/// anchor's sheet — the case a `split('!')` parser would get wrong.
#[test]
fn internal_location_resolves_on_the_named_sheet() {
    let m = make_model();
    let resolved = resolve_internal_location(&m, "Sheet2!C5", anchor());
    assert_eq!(resolved, Some(SheetRange::new(1, 5, 3, 5, 3)));
}

/// A range location keeps both corners.
#[test]
fn internal_location_keeps_a_range() {
    let m = make_model();
    let resolved = resolve_internal_location(&m, "$B$2:$D$8", anchor());
    assert_eq!(resolved, Some(SheetRange::new(0, 2, 2, 8, 4)));
}

/// A bare reference resolves relative to the anchor's sheet.
#[test]
fn internal_location_bare_reference_uses_the_anchor_sheet() {
    let m = make_model();
    let resolved = resolve_internal_location(&m, "A30", anchor());
    assert_eq!(resolved, Some(SheetRange::new(0, 30, 1, 30, 1)));
}

/// A location that names nothing returns `None`, which the caller reports;
/// it must never fall back to "select A1".
#[test]
fn internal_location_without_a_reference_is_none() {
    let m = make_model();
    assert_eq!(resolve_internal_location(&m, "NoSuchSheet!A1", anchor()), None);
    assert_eq!(resolve_internal_location(&m, "not a reference", anchor()), None);
    assert_eq!(resolve_internal_location(&m, "", anchor()), None);
}

fn single_cell() -> CellArea {
    CellArea {
        r1: 1,
        c1: 1,
        r2: 1,
        c2: 1,
    }
}

fn two_cells() -> CellArea {
    CellArea {
        r1: 1,
        c1: 1,
        r2: 2,
        c2: 1,
    }
}

fn set_link_action() -> LinkAction {
    LinkAction::SetLink {
        kind: LinkKind::External,
        target_or_location: "https://example.com".to_string(),
        tooltip: None,
        label: None,
    }
}

/// One action touches one cell. A selection-wide apply would push one undo
/// entry per cell, so a single Ctrl+Z could not undo it.
#[test]
fn a_multi_cell_selection_is_refused_for_apply_and_delete() {
    for action in [set_link_action(), LinkAction::DeleteLink] {
        let refused = plan_link_action(&action, two_cells(), anchor(), false);
        assert!(
            matches!(&refused, Err(LinkError::Refused(m)) if m == SINGLE_CELL_ONLY),
            "a two-cell selection must be refused, got {refused:?}"
        );
    }
}

/// A selection that spans rows and columns is refused by the same rule.
#[test]
fn a_rectangular_selection_is_refused() {
    let area = CellArea {
        r1: 1,
        c1: 1,
        r2: 4,
        c2: 3,
    };
    assert!(plan_link_action(&LinkAction::DeleteLink, area, anchor(), false).is_err());
}

/// A formula-owned link is refused even for a single cell: the next
/// recalculation would recreate it.
#[test]
fn a_formula_owned_link_is_refused() {
    let refused = plan_link_action(&set_link_action(), single_cell(), anchor(), true);
    assert!(
        matches!(&refused, Err(LinkError::Refused(m)) if m == FORMULA_OWNED),
        "a formula-owned link must be refused, got {refused:?}"
    );
}

/// A single-cell apply resolves to that cell, with the destination kind mapped
/// and the label preserved for the caller to write.
#[test]
fn a_single_cell_apply_resolves_to_its_cell() {
    let action = LinkAction::SetLink {
        kind: LinkKind::Internal,
        target_or_location: "Sheet2!C5".to_string(),
        tooltip: Some("go".to_string()),
        label: Some("jump".to_string()),
    };
    match plan_link_action(&action, single_cell(), anchor(), false) {
        Ok(LinkPlan::Set {
            row,
            column,
            link,
            label,
        }) => {
            assert_eq!((row, column), (1, 1), "the plan targets the anchor cell");
            assert_eq!(
                link,
                Link::Internal {
                    location: "Sheet2!C5".to_string(),
                    tooltip: Some("go".to_string()),
                }
            );
            assert_eq!(label.as_deref(), Some("jump"));
        }
        other => panic!("a single-cell apply must resolve to a set, got {other:?}"),
    }
}

/// A single-cell delete resolves to that cell and carries nothing else.
#[test]
fn a_single_cell_delete_resolves_to_its_cell() {
    match plan_link_action(&LinkAction::DeleteLink, single_cell(), anchor(), false) {
        Ok(LinkPlan::Delete { row, column }) => {
            assert_eq!((row, column), (1, 1), "the plan targets the anchor cell");
        }
        other => panic!("a single-cell delete must resolve to a delete, got {other:?}"),
    }
}

/// A label the engine cannot write must leave the link state untouched.
///
/// The engine's `set_cell_link` installs the link before it asks whether the
/// label may replace the cell content, and returns that rejection without an
/// undo entry — so the host writes the label first. Without that order this
/// test observes a link whose creation no undo step names.
#[test]
fn a_rejected_label_write_leaves_the_link_state_untouched() {
    let mut m = make_model();
    // A1:A2 legacy array formula: A2 is part of it, so the engine rejects a
    // content write there.
    m.set_user_array_formula(0, 1, 1, 1, 2, "=1+1")
        .expect("an array formula over A1:A2");
    let content_before = m.get_formatted_cell_value(0, 2, 1).expect("cell value");
    let link = Link::External {
        target: "https://example.com".to_string(),
        tooltip: None,
    };

    let err = write_cell_link(&mut m, 0, 2, 1, link, Some("a label"))
        .expect_err("a cell inside an array formula rejects a label write");
    assert!(err.contains("array formula"), "{err}");
    assert_eq!(
        m.get_cell_link(0, 2, 1).expect("read the link"),
        None,
        "the rejected write must not install the link"
    );
    assert_eq!(
        m.get_formatted_cell_value(0, 2, 1).expect("cell value"),
        content_before,
        "the rejected write must not change the content"
    );

    // The rejected write left no history entry: one undo still removes the
    // array formula itself. Had the link write been recorded first, this undo
    // would have removed the link and left the formula in place.
    m.undo().expect("undo the array formula");
    assert_eq!(m.get_formatted_cell_value(0, 1, 1).expect("cell value"), "");
    assert_eq!(m.get_cell_link(0, 2, 1).expect("read the link"), None);
}

/// A label write the engine accepts installs the link and the content. The
/// label and the link are one undo entry each, and a repeat of the same action
/// adds none.
#[test]
fn an_accepted_label_write_installs_the_link_once() {
    let mut m = UserModel::new_empty("Sheet1", "en", "UTC", "en").expect("test model");
    let link = Link::External {
        target: "https://example.com".to_string(),
        tooltip: None,
    };

    write_cell_link(&mut m, 0, 1, 1, link.clone(), Some("a label")).expect("write the link");
    assert_eq!(
        m.get_formatted_cell_value(0, 1, 1).expect("cell value"),
        "a label"
    );
    assert_eq!(
        m.get_cell_link(0, 1, 1).expect("read the link"),
        Some(link.clone())
    );

    // The same link with the same label: no cell changes, so nothing is
    // pushed. The two entries below are the first call's label write and link
    // write, in that order — a spurious entry from the repeat would leave the
    // link in place after the first undo.
    write_cell_link(&mut m, 0, 1, 1, link, Some("a label")).expect("rewrite the same link");
    m.undo().expect("undo the link write");
    assert_eq!(
        m.get_cell_link(0, 1, 1).expect("read the link"),
        None,
        "the repeat must not push an entry"
    );
    assert_eq!(
        m.get_formatted_cell_value(0, 1, 1).expect("cell value"),
        "a label",
        "the label write is undone on its own entry"
    );
    m.undo().expect("undo the label write");
    assert_eq!(m.get_formatted_cell_value(0, 1, 1).expect("cell value"), "");
}
