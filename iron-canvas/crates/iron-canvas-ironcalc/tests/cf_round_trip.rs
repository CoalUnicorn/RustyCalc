//! Conditional-formatting persistence and edit-history verification.
//!
//! The audit's "Persistence" acceptance: save and reopen the rules, then
//! compare *semantic fields and evaluated output* — a rule-list length match is
//! not a fidelity test. The second half exercises the edit path the rule
//! editor drives (gradient / value-visibility toggles) through undo and redo.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use iron_canvas_core::CellDecoration;
use iron_canvas_ironcalc::color_resolver;
use iron_canvas_ironcalc::convert::cell_decoration_from_extended;
use ironcalc::export::save_to_xlsx;
use ironcalc::import::load_from_xlsx;
use ironcalc_base::cf_types::{CfRuleInput, Cfvo};
use ironcalc_base::types::Color;
use ironcalc_base::{Model, UserModel};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../IronCalc/xlsx/tests/conditional_formatting/cf_tests.xlsx"
);

/// Cells that exercise each decoration category the audit calls out.
const PROBE_CELLS: &[(u32, i32, i32)] = &[
    (0, 17, 2), // Sheet1 icon, showValue=0
    (0, 2, 8),  // Sheet1 gradient data bar
    (0, 2, 9),  // Sheet1 solid data bar (x14)
    (0, 2, 10), // Sheet1 bounds -5..10, hidden value, split axis
    (1, 22, 2), // IconSets rating: stars
    (1, 22, 4), // IconSets rating: boxes
    (1, 4, 2),  // IconSets icon
];

fn load_model(path: &str) -> Model<'static> {
    load_from_xlsx(path, "en", "UTC", "en").expect("workbook loads")
}

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

fn temp_xlsx(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("iron-canvas-cf-{tag}-{}.xlsx", std::process::id()))
}

#[test]
fn xlsx_round_trip_preserves_rule_semantics_and_evaluated_decorations() {
    let path = temp_xlsx("roundtrip");
    let first_model = load_model(FIXTURE);
    save_to_xlsx(&first_model, path.to_str().expect("utf-8 temp path")).expect("save xlsx");
    let second_model = load_model(path.to_str().expect("utf-8 temp path"));
    let _ = std::fs::remove_file(&path);

    let first = UserModel::from_model(first_model);
    let second = UserModel::from_model(second_model);

    // Rule identity — storage index, range, priority, and rule kind — survives
    // on every sheet.
    for sheet in 0..7 {
        let before = first.get_conditional_formatting_list(sheet).expect("list");
        let after = second.get_conditional_formatting_list(sheet).expect("list");
        assert_eq!(before.len(), after.len(), "sheet {sheet} rule count");
        for (b, a) in before.iter().zip(&after) {
            assert_eq!(
                (b.index, b.range.as_str(), b.priority),
                (a.index, a.range.as_str(), a.priority),
                "sheet {sheet} rule identity"
            );
            assert_eq!(
                std::mem::discriminant(&b.cf_rule),
                std::mem::discriminant(&a.cf_rule),
                "sheet {sheet} rule kind"
            );
        }
    }

    // Full rule bodies survive everywhere except the two custom-icon-set rules
    // on `IconSets` (J2:J5, L3:L9): the xlsx writer drops their per-threshold
    // icon/color choices, which come back as the type's default preset. That
    // is an IronCalc import/export fidelity limit, recorded in
    // `docs/reviews/2026-09-27-conditional-formatting-support-gaps.md`, not a
    // canvas defect.
    for sheet in [0u32, 2, 3, 4, 5, 6] {
        assert_eq!(
            first.get_conditional_formatting_list(sheet).expect("list"),
            second.get_conditional_formatting_list(sheet).expect("list"),
            "sheet {sheet} rules must survive the round trip"
        );
    }

    // Evaluated output: the decorations the canvas consumes.
    for &(sheet, row, col) in PROBE_CELLS {
        assert_eq!(
            decoration(&first, sheet, row, col),
            decoration(&second, sheet, row, col),
            "cell ({sheet},{row},{col}) decoration must survive the round trip"
        );
    }
}

fn bar_rule(is_gradient: bool, show_value: bool) -> CfRuleInput {
    CfRuleInput::DataBar {
        min: Some(Cfvo::Number(0.0)),
        max: Some(Cfvo::Number(10.0)),
        positive_color: Color::Rgb("#638EC6".to_string()),
        negative_color: Color::Rgb("#FF0000".to_string()),
        is_gradient,
        show_value,
    }
}

fn bar_of(model: &UserModel<'static>, sheet: u32, row: i32, col: i32) -> CellDecoration {
    decoration(model, sheet, row, col).expect("a data bar applies")
}

fn find_rule(model: &UserModel<'static>, sheet: u32, range: &str) -> u32 {
    model
        .get_conditional_formatting_list(sheet)
        .expect("list")
        .into_iter()
        .find(|rule| rule.range == range)
        .unwrap_or_else(|| panic!("no rule for {range}"))
        .index as u32
}

/// The editor's gradient and value-visibility controls, driven through the
/// engine: updating a rule changes the *evaluated* decoration, and undo/redo
/// restores it exactly.
#[test]
fn gradient_and_visibility_edits_round_trip_through_undo() {
    let mut model = UserModel::from_model(load_model(FIXTURE));
    for row in 1..=3 {
        model
            .set_user_input(0, row, 26, "5")
            .expect("write Z cell value");
    }
    model
        .add_conditional_formatting(0, "Z1:Z3", bar_rule(true, true))
        .expect("add rule");
    let index = find_rule(&model, 0, "Z1:Z3");

    let before = bar_of(&model, 0, 1, 26).data_bar.expect("bar");
    assert!(
        before.is_gradient && before.show_value,
        "initial: gradient, visible"
    );

    // Turn the Gradient fill off and hide the value, as the editor's checkboxes do.
    model
        .update_conditional_formatting(0, index, "Z1:Z3", bar_rule(false, false))
        .expect("update rule");
    let after = bar_of(&model, 0, 1, 26).data_bar.expect("bar");
    assert!(
        !after.is_gradient && !after.show_value,
        "the edit must reach the evaluated decoration"
    );

    model.undo().expect("undo the edit");
    let undone = bar_of(&model, 0, 1, 26).data_bar.expect("bar");
    assert_eq!(undone, before, "undo restores the evaluated decoration");

    model.redo().expect("redo the edit");
    let redone = bar_of(&model, 0, 1, 26).data_bar.expect("bar");
    assert_eq!(redone, after, "redo re-applies the evaluated decoration");
}
