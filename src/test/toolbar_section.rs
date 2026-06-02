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
            ToolbarSection::View,
            ToolbarSection::File,
        ],
    );
}

#[test]
fn toolbar_section_labels_are_stable() {
    assert_eq!(ToolbarSection::Home.label(), "Home");
    assert_eq!(ToolbarSection::Data.label(), "Data");
    assert_eq!(ToolbarSection::View.label(), "View");
    assert_eq!(ToolbarSection::File.label(), "File");
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

use crate::components::chrome::toolbar::icon::{
    AlignIcon, BorderIcon, ChromeIcon, EditIcon, FileIcon, Glyph, SheetIcon, TextIcon,
};

#[test]
fn every_icon_has_non_empty_path() {
    fn check<G: Glyph + std::fmt::Debug>(group: &[G]) {
        for icon in group {
            assert!(!icon.path().is_empty(), "{icon:?} has no SVG path");
        }
    }
    check(&EditIcon::all());
    check(&TextIcon::all());
    check(&AlignIcon::all());
    check(&SheetIcon::all());
    check(&FileIcon::all());
    check(&ChromeIcon::all());
    check(&BorderIcon::all());
}

use crate::components::chrome::toolbar::chrome_controls::app_version;

#[test]
fn app_version_marks_debug_builds() {
    // Tests run with debug_assertions on, so the version carries the -dev suffix.
    assert_eq!(app_version(), format!("{}-dev", env!("CARGO_PKG_VERSION")));
}
