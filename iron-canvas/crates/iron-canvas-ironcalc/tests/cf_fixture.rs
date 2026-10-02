//! Engine-backed conditional-formatting verification.
//!
//! Loads the two workbooks the `2026-09-27` support-gap audit used and checks
//! the *evaluated decorations* the adapter hands the canvas — not just rule
//! counts. This is the Rust half of the review's "Engine/import" acceptance
//! check; the browser pixel comparison is still owed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use iron_canvas_core::{CellDecoration, IconGlyph};
use iron_canvas_ironcalc::color_resolver;
use iron_canvas_ironcalc::convert::cell_decoration_from_extended;
use ironcalc::import::load_from_xlsx;
use ironcalc_base::UserModel;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../IronCalc/xlsx/tests/conditional_formatting/cf_tests.xlsx"
);

fn load(path: &str) -> UserModel<'static> {
    UserModel::from_model(load_from_xlsx(path, "en", "UTC", "en").expect("workbook loads"))
}

/// The adapter's decoration for one cell, with colors resolved through the
/// workbook theme exactly as the app's adapter does.
fn decoration(
    model: &UserModel<'static>,
    sheet: u32,
    row: i32,
    col: i32,
) -> Option<CellDecoration> {
    let ext = model
        .get_extended_cell_style(sheet, row, col)
        .expect("cell read");
    cell_decoration_from_extended(&ext, &color_resolver(model))
}

fn rule_count(model: &UserModel<'static>, sheet: u32) -> usize {
    model
        .get_conditional_formatting_list(sheet)
        .expect("rule list")
        .len()
}

#[test]
fn fixture_imports_fifty_four_rules_across_seven_sheets() {
    let model = load(FIXTURE);
    let counts: Vec<usize> = (0..7).map(|sheet| rule_count(&model, sheet)).collect();
    assert_eq!(
        counts,
        vec![18, 20, 3, 4, 3, 2, 4],
        "the 54 imported rules the audit expects, per sheet"
    );
}

#[test]
fn icons_with_show_value_zero_reach_the_canvas_hidden() {
    let model = load(FIXTURE);
    // Sheet1!B17:B22 carry icons with showValue="0".
    let icon = decoration(&model, 0, 17, 2)
        .expect("B17 has a decoration")
        .icon
        .expect("B17 is an icon cell");
    assert!(
        !icon.show_value,
        "showValue=0 must survive as show_value=false"
    );
}

#[test]
fn x14_data_bar_overrides_reach_the_canvas() {
    let model = load(FIXTURE);
    // H2:H8 — normal gradient bars.
    let gradient = decoration(&model, 0, 2, 8)
        .expect("H2 has a decoration")
        .data_bar
        .expect("H2 is a data-bar cell");
    assert!(gradient.is_gradient, "H2 is a gradient bar");

    // I2:I8 — solid bars from the x14 gradient="0" override.
    let solid = decoration(&model, 0, 2, 9)
        .expect("I2 has a decoration")
        .data_bar
        .expect("I2 is a data-bar cell");
    assert!(
        !solid.is_gradient,
        "the x14 override must force a solid bar"
    );

    // J2:J8 — solid bars, bounds -5..10, hidden values. Zero sits at one third
    // of the range, so the axis is not at the left edge.
    let axis = decoration(&model, 0, 2, 10)
        .expect("J2 has a decoration")
        .data_bar
        .expect("J2 is a data-bar cell");
    assert!(!axis.is_gradient);
    assert!(!axis.show_value, "J2 hides its value");
    assert!(
        (axis.axis_position - 1.0 / 3.0).abs() < 1e-6,
        "bounds -5..10 put the zero axis at one third, got {}",
        axis.axis_position
    );
    // 5 / 15 = 1/3 of the range from the axis is the value endpoint.
    assert!((axis.value - 8.0 / 15.0).abs() < 1e-6, "got {}", axis.value);
}

#[test]
fn ratings_carry_the_engine_glyph_color_and_counts() {
    let model = load(FIXTURE);
    // IconSets!L22:L26 — the rating columns.
    let star = decoration(&model, 1, 22, 2)
        .expect("L22 has a decoration")
        .rating
        .expect("L22 is a rating cell");
    assert_eq!(star.glyph, IconGlyph::Star);
    assert_eq!(
        star.color.as_deref().map(str::to_lowercase).as_deref(),
        Some("#ffd700")
    );
    assert_eq!((star.count, star.max), (1, 3));

    let circle = decoration(&model, 1, 23, 3)
        .expect("M23 has a decoration")
        .rating
        .expect("M23 is a rating cell");
    assert_eq!(circle.glyph, IconGlyph::Circle);
    assert_eq!((circle.count, circle.max), (2, 5));
}
