//! Tests for the hyperlink actions in `src/input/link.rs`.
//!
//! The URL allowlist and the internal-target resolution are the two pieces of
//! host logic that decide whether a workbook's link can leave the browser or
//! move the selection; both are pure and tested here without a DOM.

use crate::coord::{CellAddress, SheetRange};
use crate::input::link::{allowed_external_url, resolve_internal_location};
use ironcalc_base::UserModel;

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

