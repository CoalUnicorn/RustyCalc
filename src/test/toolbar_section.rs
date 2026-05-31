use crate::components::chrome::toolbar::section::ToolbarSection;

#[test]
fn toolbar_section_defaults_to_home() {
    assert_eq!(ToolbarSection::default(), ToolbarSection::Home);
}

#[test]
fn toolbar_section_all_lists_every_tab_in_order() {
    assert_eq!(
        ToolbarSection::all(),
        [
            ToolbarSection::Home,
            ToolbarSection::Data,
            ToolbarSection::View
        ],
    );
}

#[test]
fn toolbar_section_labels_are_stable() {
    assert_eq!(ToolbarSection::Home.label(), "Home");
    assert_eq!(ToolbarSection::Data.label(), "Data");
    assert_eq!(ToolbarSection::View.label(), "View");
}

use crate::components::chrome::toolbar::overflow::fit_count;

#[test]
fn fit_count_everything_fits_no_more_button() {
    // 3 slots of 30 + 2 gaps of 4 = 98 <= 200 -> all fit, no reservation.
    assert_eq!(fit_count(&[30.0, 30.0, 30.0], 4.0, 200.0, 40.0), 3);
}

#[test]
fn fit_count_reserves_room_for_more_when_overflowing() {
    // full = 162 > 120 -> reserve 40 => budget 80. 30(=30)+4+30(=64)+4+30(=98>80) => 2.
    assert_eq!(fit_count(&[30.0, 30.0, 30.0, 30.0], 4.0, 120.0, 40.0), 2);
}

#[test]
fn fit_count_handles_none_fitting() {
    assert_eq!(fit_count(&[100.0, 100.0], 4.0, 60.0, 40.0), 0);
}

#[test]
fn fit_count_empty_is_zero() {
    assert_eq!(fit_count(&[], 4.0, 200.0, 40.0), 0);
}

use crate::components::chrome::toolbar::icon::IconName;

#[test]
fn every_icon_has_non_empty_path() {
    for name in IconName::all() {
        assert!(!name.path().is_empty(), "{name:?} has no SVG path");
    }
}
