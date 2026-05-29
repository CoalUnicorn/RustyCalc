#![allow(clippy::unwrap_used)]

use crate::coord::{CellAddress, DefinedName, FormulaRefKind};
use crate::input::formula::FormulaStatus;
use ironcalc_base::expressions::types::CellReferenceRC;

    use crate::input::formula::analysis::*;
    use crate::coord::CellArea;

    /// Test editing-cell fixture with row=0, column=0. Matches the pre-refactor
    /// parser context so Node-relative coords equal their absolute 1-based form
    /// — every existing assertion on `sheet_area.area` stays valid under the
    /// new signature without arithmetic adjustment.
    fn editing_at(sheet: u32) -> CellAddress {
        CellAddress {
            sheet,
            row: 0,
            column: 0,
        }
    }

    /// Stringify ctx paired with `editing_at` — same-sheet context with empty
    /// sheet name, so `to_localized` emits bare A1-style for same-sheet refs
    /// and `Sheet!A1` only when the Node carries `sheet_name: Some(_)`.
    fn ctx_at(sheet_name: &str) -> CellReferenceRC {
        CellReferenceRC {
            sheet: sheet_name.to_string(),
            row: 0,
            column: 0,
        }
    }

    #[test]
    fn test_single_cell_ref() {
        let analysis = analyze_formula("=A1+1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(
            analysis.refs()[0].sheet_area.area,
            CellArea {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 1
            }
        );
        assert_eq!(analysis.refs()[0].sheet_area.sheet, 0);
        assert!(matches!(analysis.status, FormulaStatus::Valid { .. }));
    }

    #[test]
    fn test_range_ref() {
        let analysis = analyze_formula("=SUM(B2:C4)", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(
            analysis.refs()[0].sheet_area.area,
            CellArea {
                r1: 2,
                c1: 2,
                r2: 4,
                c2: 3
            }
        );
    }

    #[test]
    fn test_multiple_refs_get_different_color_indices() {
        let analysis = analyze_formula("=A1+B2", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 2);
        assert_ne!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_non_formula_returns_empty() {
        let analysis = analyze_formula("hello", editing_at(0), &[], &[]);
        assert!(analysis.refs().is_empty());
        assert!(matches!(analysis.status, FormulaStatus::NotFormula));
    }

    #[test]
    fn test_cross_sheet_ref_resolved() {
        let sheets = vec![(0u32, "Sheet1".to_string()), (1u32, "Sheet2".to_string())];
        let analysis = analyze_formula("=Sheet2!A1", editing_at(0), &sheets, &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(analysis.refs()[0].sheet_area.sheet, 1);
    }

    #[test]
    fn test_unknown_sheet_ref_is_skipped() {
        // A reference to a sheet that doesn't exist in sheet_names should produce
        // no overlay rather than a misleading overlay on the active sheet.
        let sheets = vec![(0u32, "Sheet1".to_string())];
        let analysis = analyze_formula("=Ghost!A1", editing_at(0), &sheets, &[]);
        assert_eq!(analysis.refs().len(), 0);
        assert!(matches!(analysis.status, FormulaStatus::Unresolved { .. }));
    }

    #[test]
    fn test_same_cell_shares_color_slot() {
        // Option A: A1 and A1 collapse to one color slot, regardless of $-prefix.
        let analysis = analyze_formula("=A1+$A$1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 2);
        assert_eq!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_distinct_cells_get_distinct_slots() {
        let analysis = analyze_formula("=A1+B2+A1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 3);
        assert_eq!(analysis.refs()[0].color_idx, analysis.refs()[2].color_idx);
        assert_ne!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_range_and_single_share_when_endpoints_match() {
        let analysis = analyze_formula("=A1+A1:A1", editing_at(0), &[], &[]);
        assert!(matches!(analysis.status, FormulaStatus::Valid { .. }));
        assert_eq!(analysis.refs().len(), 2);
        assert_eq!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_invalid_function_captured() {
        let analysis = analyze_formula("=FOOBAR(1,2)", editing_at(0), &[], &[]);
        let FormulaStatus::Unresolved {
            invalid_functions, ..
        } = &analysis.status
        else {
            panic!("expected Unresolved, got {:?}", analysis.status);
        };
        assert_eq!(invalid_functions.len(), 1);
        let span = invalid_functions[0];
        assert_eq!(&"=FOOBAR(1,2)"[span.start..span.end], "FOOBAR");
    }

    #[test]
    fn test_known_function_not_flagged() {
        let analysis = analyze_formula("=SUM(A1:A3)", editing_at(0), &[], &[]);
        assert!(matches!(analysis.status, FormulaStatus::Valid { .. }));
    }

    #[test]
    fn test_wrong_sheet_ref_captured() {
        let sheets = vec![(0u32, "Sheet1".to_string())];
        let analysis = analyze_formula("=Ghost!A1", editing_at(0), &sheets, &[]);
        let FormulaStatus::Unresolved { invalid_refs, .. } = &analysis.status else {
            panic!("expected Unresolved, got {:?}", analysis.status);
        };
        assert_eq!(invalid_refs.len(), 1);
        assert!(analysis.refs().is_empty());
    }

    #[test]
    fn test_known_defined_name_resolves_as_valid() {
        // With `my_range` plumbed in, the parser emits DefinedNameKind and
        // the identifier no longer trips the Unresolved path.
        let defined = vec![DefinedName {
            name: "my_range".into(),
            scope: None,
            formula: "A1:A10".into(),
        }];
        let analysis = analyze_formula("=my_range+1", editing_at(0), &[], &defined);
        assert!(
            matches!(analysis.status, FormulaStatus::Valid { .. }),
            "expected Valid, got {:?}",
            analysis.status
        );
    }

    #[test]
    fn test_unknown_name_captured() {
        // No defined names  bare identifier parses as NamedVariableKind and
        // must land in Unresolved.names (NOT Unresolved.functions).
        let analysis = analyze_formula("=my_undefined", editing_at(0), &[], &[]);
        let FormulaStatus::Unresolved {
            invalid_names,
            invalid_functions,
            ..
        } = &analysis.status
        else {
            panic!("expected Unresolved, got {:?}", analysis.status);
        };
        assert_eq!(invalid_names.len(), 1, "unknown name should be captured");
        assert!(
            invalid_functions.is_empty(),
            "unknown name must NOT leak into functions"
        );
        let span = invalid_names[0];
        assert_eq!(&"=my_undefined"[span.start..span.end], "my_undefined");
    }

    #[test]
    fn test_mixed_unknown_name_and_function() {
        // Both diagnostics surface independently — the renderer can style
        // them differently (squiggle vs italic) without ambiguity.
        let analysis = analyze_formula("=my_undefined + FOOBAR(1)", editing_at(0), &[], &[]);
        let FormulaStatus::Unresolved {
            invalid_names,
            invalid_functions,
            ..
        } = &analysis.status
        else {
            panic!("expected Unresolved, got {:?}", analysis.status);
        };
        assert_eq!(invalid_names.len(), 1);
        assert_eq!(invalid_functions.len(), 1);
    }

    #[test]
    fn test_validation_error_is_human_readable() {
        // LexerError.message (not Debug format) should be used — no "LexerError {" prefix.
        let analysis = analyze_formula("=@invalid", editing_at(0), &[], &[]);
        if let FormulaStatus::LexerError(ref e) = analysis.status {
            assert!(
                !e.message.contains("LexerError"),
                "validation_error should not contain Rust debug output, got: {}",
                e.message
            );
        }
    }

    // Identity preservation — ref_node must carry `absolute_row` /
    // `absolute_column` / `sheet_name` through analysis. These tests fail
    // until `ast_leaves` pushes the full Node via `RefNode::cell` /
    // `RefNode::range` Until then `refs()` is
    // empty for resolved refs, so `.refs().len()` is 0 and the `refs()[0]`
    // indexing panics — by design, making the stub's presence impossible
    // to miss in test output.

    #[test]
    fn absolute_flags_preserved() {
        // `=$A$1` — both axes absolute. Round-tripping `ref_node` via
        // `to_localized` emits `$A$1` iff the flags reached RefNode. If
        // ast_leaves dropped them (the pre-refactor bug), stringify -> `A1`.
        let analysis = analyze_formula("=$A$1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(
            analysis.refs()[0].ref_node.to_localized(&ctx_at("")),
            "$A$1"
        );
    }

    #[test]
    fn mixed_absolute_preserved() {
        // Per-axis flags survive independently.
        let analysis = analyze_formula("=$A1+B$2", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 2);
        let ctx = ctx_at("");
        assert_eq!(analysis.refs()[0].ref_node.to_localized(&ctx), "$A1");
        assert_eq!(analysis.refs()[1].ref_node.to_localized(&ctx), "B$2");
    }

    #[test]
    fn cross_sheet_name_preserved() {
        let sheets = vec![(0u32, "Sheet1".to_string()), (1u32, "Sheet2".to_string())];
        let analysis = analyze_formula("=Sheet2!A1", editing_at(0), &sheets, &[]);
        assert_eq!(analysis.refs().len(), 1);
        // Stringify ctx on Sheet1 — so a Sheet2! prefix only appears if the
        // Node carries `sheet_name: Some("Sheet2")`.
        assert_eq!(
            analysis.refs()[0].ref_node.to_localized(&ctx_at("Sheet1")),
            "Sheet2!A1"
        );
    }

    #[test]
    fn same_sheet_name_is_none() {
        // Same-sheet ref must NOT acquire a spurious `Sheet1!` prefix just
        // because sheet_names happens to contain the active sheet's entry.
        let sheets = vec![(0u32, "Sheet1".to_string())];
        let analysis = analyze_formula("=A1", editing_at(0), &sheets, &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(
            analysis.refs()[0].ref_node.to_localized(&ctx_at("Sheet1")),
            "A1"
        );
    }

    // refs_at_cursor — byte-span hit-test with inclusive boundaries

    #[test]
    fn cursor_inside_single_ref() {
        // `=A1+1` — A1 at bytes [1, 3). cursor=2 is strictly inside.
        let analysis = analyze_formula("=A1+1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(2).count(), 1);
    }

    #[test]
    fn cursor_at_ref_left_edge() {
        let analysis = analyze_formula("=A1+1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(1).count(), 1);
    }

    #[test]
    fn cursor_at_ref_right_edge_is_inclusive() {
        // cursor=3 sits just after A1's last byte. Under the inclusive
        // right-edge rule, this IS "on" A1 — matching the just-typed-a-ref
        // UX moment.
        let analysis = analyze_formula("=A1+1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(3).count(), 1);
    }

    #[test]
    fn cursor_between_refs_yields_nothing() {
        // `=A1 + B2` — whitespace between A1 (ends at 3) and B2 (starts at 6).
        // cursor=4 sits on the first space — no ref overlap.
        let analysis = analyze_formula("=A1 + B2", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(4).count(), 0);
    }

    #[test]
    fn cursor_inside_range() {
        // `=SUM(A1:B3)` — the whole `A1:B3` is ONE Range token. cursor=7 is
        // on the `:` — inside the range token's span.
        let analysis = analyze_formula("=SUM(A1:B3)", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(7).count(), 1);
    }

    #[test]
    fn cursor_on_non_formula_yields_nothing() {
        // NotFormula -> refs() is empty; cursor query returns nothing regardless.
        let analysis = analyze_formula("hello", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(2).count(), 0);
    }

    #[test]
    fn cursor_on_parse_error_yields_nothing() {
        // `=A1++` — parser rejects the trailing `+`. ParseError variant makes
        // refs() empty (AST too broken to trust) so cursor query yields none.
        let analysis = analyze_formula("=A1++", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(1).count(), 0);
    }

    // ---- FormulaRefKind tagging ----

    #[test]
    fn direct_ref_kind_is_direct() {
        let analysis = analyze_formula("=A1+1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert!(matches!(analysis.refs()[0].kind, FormulaRefKind::Direct));
    }

    #[test]
    fn defined_name_ref_kind_is_defined_name() {
        let defined = vec![DefinedName {
            name: "my_range".into(),
            scope: None,
            formula: "A1:A10".into(),
        }];
        let analysis = analyze_formula("=my_range+1", editing_at(0), &[], &defined);
        assert_eq!(analysis.refs().len(), 1);
        assert!(matches!(
            analysis.refs()[0].kind,
            FormulaRefKind::DefinedName
        ));
    }

    #[test]
    fn mixed_emissions_carry_independent_kinds() {
        // `=A1+my_range` emits one Direct (A1) and one DefinedName (my_range)
        // in document order. The two kinds must route independently — order
        // here mirrors token-stream order in `analyze_formula`.
        let defined = vec![DefinedName {
            name: "my_range".into(),
            scope: None,
            formula: "B1:B10".into(),
        }];
        let analysis = analyze_formula("=A1+my_range", editing_at(0), &[], &defined);
        assert_eq!(analysis.refs().len(), 2);
        assert!(matches!(analysis.refs()[0].kind, FormulaRefKind::Direct));
        assert!(matches!(
            analysis.refs()[1].kind,
            FormulaRefKind::DefinedName
        ));
    }
