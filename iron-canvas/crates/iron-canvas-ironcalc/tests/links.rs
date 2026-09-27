//! Link conversion and the `CanvasModel::get_sheet_links` adapter contract.

use iron_canvas_core::{CanvasModel, CellLink, LinkTarget};
use iron_canvas_ironcalc::convert::{hyperlink_color, link_to_core, style_to_core};
use iron_canvas_ironcalc::{IronCalcModel, color_resolver};
use ironcalc_base::UserModel;
use ironcalc_base::types as ic;

fn new_model() -> UserModel<'static> {
    match UserModel::new_empty("wb", "en", "UTC", "en") {
        Ok(m) => m,
        Err(e) => panic!("empty workbook must construct: {e}"),
    }
}

fn external(target: &str) -> ic::Link {
    ic::Link::External {
        target: target.to_string(),
        tooltip: None,
    }
}

fn link_at(links: &[CellLink], row: i32, column: i32) -> &CellLink {
    links
        .iter()
        .find(|link| link.range().r1 == row && link.range().c1 == column)
        .unwrap_or_else(|| panic!("no link at ({row}, {column})"))
}

/// A static worksheet link and a formula link are both reported, the static
/// one wins at an address that has both, and `dynamic` distinguishes them.
#[test]
fn static_and_formula_links_merge_with_static_precedence() {
    let mut m = new_model();
    m.set_cell_link(0, 1, 1, external("https://static.example"), None)
        .expect("static link write");
    m.set_user_input(0, 2, 1, "=HYPERLINK(\"https://formula.example\",\"x\")")
        .expect("formula write");
    m.evaluate();
    // Both kinds at the same address: the worksheet link must win.
    m.set_user_input(0, 3, 1, "=HYPERLINK(\"https://dynamic.example\",\"y\")")
        .expect("formula write");
    m.evaluate();
    m.set_cell_link(0, 3, 1, external("https://static-wins.example"), None)
        .expect("static link write");
    m.evaluate();

    let adapter = IronCalcModel(m);
    let links = adapter
        .get_sheet_links(0)
        .expect("a native link read succeeds");

    let static_link = link_at(&links, 1, 1);
    assert_eq!(static_link.target().as_str(), "https://static.example");
    assert!(!static_link.is_dynamic());

    let dynamic_link = link_at(&links, 2, 1);
    assert_eq!(dynamic_link.target().as_str(), "https://formula.example");
    assert!(dynamic_link.is_dynamic());

    let both = link_at(&links, 3, 1);
    assert_eq!(both.target().as_str(), "https://static-wins.example");
    assert!(!both.is_dynamic(), "a worksheet link takes precedence");
}

/// An internal formula target (`#Sheet1!A5`) keeps its kind and drops the `#`.
#[test]
fn internal_formula_link_converts_to_an_internal_target() {
    let mut m = new_model();
    m.set_user_input(0, 2, 1, "=HYPERLINK(\"#Sheet1!A5\",\"go\")")
        .expect("formula write");
    m.evaluate();

    let adapter = IronCalcModel(m);
    let links = adapter
        .get_sheet_links(0)
        .expect("a native link read succeeds");
    let link = link_at(&links, 2, 1);
    assert!(!link.target().is_external());
    assert_eq!(link.target().as_str(), "Sheet1!A5");
}

/// The literal theme index in `hyperlink_color` must agree with the color a
/// static link's own cell style resolves. This guards it against upstream
/// drift.
#[test]
fn static_link_color_matches_the_cell_style_color() {
    let mut m = new_model();
    m.set_cell_link(0, 1, 1, external("https://example.com"), None)
        .expect("static link write");

    let resolve = color_resolver(&m);
    let link = link_to_core(external("https://example.com"), 1, 1, false, &resolve);
    let style = style_to_core(
        m.get_cell_style(0, 1, 1)
            .expect("the link cell has a style"),
        &resolve,
    );

    assert!(style.font.underline, "a static link is underlined");
    assert!(style.font.color.is_some());
    assert_eq!(
        link.color(),
        style.font.color.as_deref(),
        "the link color must be the color the cell style already carries"
    );
    assert_eq!(hyperlink_color(&resolve), link.color().map(str::to_string));
}

/// `link_to_core` maps both target kinds and preserves the tooltip.
#[test]
fn link_to_core_preserves_kind_tooltip_and_address() {
    let m = new_model();
    let resolve = color_resolver(&m);

    let core = link_to_core(
        ic::Link::Internal {
            location: "Sheet2!B7".to_string(),
            tooltip: Some("jump".to_string()),
        },
        4,
        2,
        true,
        &resolve,
    );
    assert_eq!(core.range().r1, 4);
    assert_eq!(core.range().c1, 2);
    assert_eq!(core.target(), &LinkTarget::Internal("Sheet2!B7".to_string()));
    assert_eq!(core.tooltip(), Some("jump"));
    assert!(core.is_dynamic());
}

/// A precedent edit changes the formula link's target while the displayed
/// label stays put, and an erroring formula drops the link entirely — the
/// engine rebuilds `Model::links` on every evaluation.
#[test]
fn a_precedent_edit_changes_the_target_and_an_error_drops_the_link() {
    let mut m = new_model();
    m.set_user_input(0, 1, 1, "https://one.example")
        .expect("precedent write");
    // The formula lives at (3,1); the precedent URL at (1,1) also carries the
    // engine's auto static link, so every read selects the formula's address.
    m.set_user_input(0, 3, 1, "=HYPERLINK(A1,\"label\")")
        .expect("formula write");
    m.evaluate();

    let adapter = IronCalcModel(m);
    let links = adapter.get_sheet_links(0).expect("link read");
    let dynamic = link_at(&links, 3, 1);
    assert!(dynamic.is_dynamic());
    assert_eq!(dynamic.target().as_str(), "https://one.example");

    // Move the model back out to edit it, then re-read through the adapter.
    let mut m = adapter.0;
    m.set_user_input(0, 1, 1, "https://two.example")
        .expect("precedent write");
    m.evaluate();
    let adapter = IronCalcModel(m);
    let links = adapter.get_sheet_links(0).expect("link read");
    assert_eq!(
        link_at(&links, 3, 1).target().as_str(),
        "https://two.example",
        "the label is unchanged, so only the target moves"
    );
    assert_eq!(
        adapter
            .get_formatted_cell_value(0, 3, 1)
            .expect("the label is readable"),
        "label"
    );

    let mut m = adapter.0;
    m.set_user_input(0, 1, 1, "=1/0").expect("precedent write");
    m.evaluate();
    let adapter = IronCalcModel(m);
    let links = adapter.get_sheet_links(0).expect("link read");
    assert!(
        links
            .iter()
            .all(|link| link.range().r1 != 3 || link.range().c1 != 1),
        "an erroring HYPERLINK attaches no link at the formula cell"
    );
}
