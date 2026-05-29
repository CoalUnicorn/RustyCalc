use crate::coord::CellArea;
use crate::model::ArrowKey;
use ironcalc_base::UserModel;
    use crate::model::frontend_model::*;

    /// Helper: create a minimal empty workbook model for testing.
    #[allow(clippy::expect_used)]
    fn make_model() -> UserModel<'static> {
        UserModel::new_empty("Sheet1", "en", "UTC", "en").expect("failed to create test model")
    }

    #[test]
    fn toolbar_state_reflects_active_cell() {
        let m = make_model();
        let ts = m.toolbar_state();
        assert!(!ts.format.bold);
        assert!(!ts.format.italic);
        assert!(ts.style.font_size > 0.0);
    }

    #[test]
    fn nav_arrow_down_moves_selection() {
        let mut m = make_model();
        let before = m.get_selected_view().row;
        m.nav_arrow(ArrowKey::Down);
        let after = m.get_selected_view().row;
        assert_eq!(after, before + 1);
    }

    #[test]
    fn nav_set_cell_clamps_out_of_range() {
        let mut m = make_model();
        m.nav_set_cell(-1, 0);
        let v = m.get_selected_view();
        assert_eq!(v.row, 1);
        assert_eq!(v.column, 1);
    }

    #[test]
    fn nav_select_range_sets_active_cell_and_range() {
        let mut m = make_model();
        m.nav_select_range(CellArea {
            r1: 2,
            c1: 3,
            r2: 5,
            c2: 7,
        });
        let v = m.get_selected_view();
        assert_eq!(v.row, 2);
        assert_eq!(v.column, 3);
        assert_eq!(v.range, [2, 3, 5, 7]);
    }

    #[test]
    fn nav_expand_selection_extends_range() {
        let mut m = make_model();
        // Start at (1,1), expand down: range should cover row 1..2
        m.nav_expand_selection(ArrowKey::Down);
        let v = m.get_selected_view();
        let r_min = v.range[0].min(v.range[2]);
        let r_max = v.range[0].max(v.range[2]);
        assert_eq!(r_min, 1);
        assert_eq!(r_max, 2);
    }

    #[test]
    fn nav_home_row_moves_to_column_1() {
        let mut m = make_model();
        m.nav_set_cell(5, 10);
        m.nav_home_row();
        let v = m.get_selected_view();
        assert_eq!(v.row, 5);
        assert_eq!(v.column, 1);
    }

    #[test]
    fn nav_select_column_sets_full_range() {
        let mut m = make_model();
        m.nav_select_column(3);
        let v = m.get_selected_view();
        assert_eq!(v.column, 3);
        assert_eq!(v.range[1], 3);
        assert_eq!(v.range[3], 3);
    }

    #[test]
    fn sheet_dimension_empty_sheet() {
        let m = make_model();
        let d = m.sheet_dimension();
        // Empty sheet defaults to (1,1,1,1).
        assert_eq!(d.r1, 1);
        assert_eq!(d.c1, 1);
        assert_eq!(d.r2, 1);
        assert_eq!(d.c2, 1);
    }

    #[allow(clippy::unwrap_used)]
    #[test]
    fn sheet_dimension_after_input() {
        let mut m = make_model();
        m.set_user_input(0, 5, 3, "hello").unwrap();
        m.evaluate();
        let d = m.sheet_dimension();
        assert!(d.r2 >= 5);
        assert!(d.c2 >= 3);
    }
