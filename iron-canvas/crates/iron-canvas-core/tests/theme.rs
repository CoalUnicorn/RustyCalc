use std::borrow::Cow;
use std::collections::HashMap;

use iron_canvas_core::theme::{CanvasTheme, ThemeVariables};

#[test]
fn light_equals_itself() {
    assert_eq!(CanvasTheme::light(), CanvasTheme::light());
}

#[test]
fn dark_equals_itself() {
    assert_eq!(CanvasTheme::dark(), CanvasTheme::dark());
}

#[test]
fn light_and_dark_differ() {
    assert_ne!(CanvasTheme::light(), CanvasTheme::dark());
}

// Built-in themes carry every field as `Cow::Borrowed(&'static str)`. That is
// the contract that lets the painter cache ptr-eq theme colors — a regression
// to `Cow::Owned` (e.g. via a `String::from` in the const) would silently move
// the static path onto the content-eq cache.
#[test]
fn light_fields_are_borrowed_cows() {
    let t = CanvasTheme::light();
    assert!(matches!(t.grid_color, Cow::Borrowed(_)));
    assert!(matches!(t.grid_separator_color, Cow::Borrowed(_)));
    assert!(matches!(t.header_bg, Cow::Borrowed(_)));
    assert!(matches!(t.header_border_color, Cow::Borrowed(_)));
    assert!(matches!(t.header_text_color, Cow::Borrowed(_)));
    assert!(matches!(t.header_selected_bg, Cow::Borrowed(_)));
    assert!(matches!(t.header_selected_color, Cow::Borrowed(_)));
    assert!(matches!(t.default_text_color, Cow::Borrowed(_)));
    assert!(matches!(t.error_text_color, Cow::Borrowed(_)));
    assert!(matches!(t.selection_color, Cow::Borrowed(_)));
    assert!(matches!(t.cell_bg, Cow::Borrowed(_)));
    assert!(matches!(t.pointing, Cow::Borrowed(_)));
    assert!(matches!(t.selection_fill, Cow::Borrowed(_)));
    assert!(matches!(t.pointing_tint, Cow::Borrowed(_)));
}

#[test]
fn dark_fields_are_borrowed_cows() {
    let t = CanvasTheme::dark();
    assert!(matches!(t.grid_color, Cow::Borrowed(_)));
    assert!(matches!(t.cell_bg, Cow::Borrowed(_)));
    assert!(matches!(t.default_text_color, Cow::Borrowed(_)));
}

// Stage 2: ThemeVariables -> CanvasTheme conversion.

#[test]
fn empty_variables_round_trip_to_light() {
    let theme: CanvasTheme = ThemeVariables::default().into();
    assert_eq!(theme, CanvasTheme::light());
}

#[test]
fn empty_variables_via_builder_round_trip_to_light() {
    let theme = ThemeVariables::new().build();
    assert_eq!(theme, CanvasTheme::light());
}

#[test]
fn one_field_override_changes_only_that_field() {
    let theme = ThemeVariables::new().with_header_bg("#123456").build();
    let light = CanvasTheme::light();
    assert_eq!(
        theme.header_bg,
        Cow::<'static, str>::Owned("#123456".into())
    );
    // Every other field must be untouched.
    assert_eq!(theme.grid_color, light.grid_color);
    assert_eq!(theme.grid_separator_color, light.grid_separator_color);
    assert_eq!(theme.header_border_color, light.header_border_color);
    assert_eq!(theme.header_text_color, light.header_text_color);
    assert_eq!(theme.header_selected_bg, light.header_selected_bg);
    assert_eq!(theme.header_selected_color, light.header_selected_color);
    assert_eq!(theme.default_text_color, light.default_text_color);
    assert_eq!(theme.error_text_color, light.error_text_color);
    assert_eq!(theme.selection_color, light.selection_color);
    assert_eq!(theme.cell_bg, light.cell_bg);
    assert_eq!(theme.pointing, light.pointing);
    assert_eq!(theme.selection_fill, light.selection_fill);
    assert_eq!(theme.pointing_tint, light.pointing_tint);
}

#[test]
fn override_field_is_owned_cow_unspecified_stays_borrowed() {
    let theme = ThemeVariables::new().with_grid_color("#abcdef").build();
    assert!(matches!(theme.grid_color, Cow::Owned(_)));
    assert!(matches!(theme.cell_bg, Cow::Borrowed(_)));
}

// Smoke test mirroring the shape of upstream's storybook "crazy theme" — every
// CanvasTheme field overridden, all fields land as Cow::Owned, none equal to
// the LIGHT default.
// Stage 4: idempotence of `IronCanvas::set_theme_variables`.
//
// The setter is `self.set_theme(vars.build())`. Its dirty-bit short-circuit
// reduces to a `==` comparison between successive `vars.build()` results, so
// the property to pin is "two builds of the same `ThemeVariables` are equal
// `CanvasTheme`s". `IronCanvas` itself can't be instantiated on host (it
// needs `HtmlCanvasElement`); the existing orchestrator tests use the same
// shape — assert the underlying invariant rather than spin up a layer stack.

#[test]
fn theme_variables_build_is_deterministic() {
    let vars = ThemeVariables::new()
        .with_grid_color("#abcdef")
        .with_header_bg("#fedcba")
        .with_default_text_color("#222222");
    let first: CanvasTheme = vars.clone().into();
    let second: CanvasTheme = vars.into();
    assert_eq!(first, second);
}

#[test]
fn empty_variables_build_equals_set_theme_name_light() {
    let from_vars: CanvasTheme = ThemeVariables::default().into();
    let from_name = CanvasTheme::light();
    assert_eq!(from_vars, from_name);
}

// Stage 3 (part 1): CSS-var reader → ThemeVariables.

fn map_reader(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
    move |k: &str| map.get(k).map(|v| (*v).to_string())
}

#[test]
fn empty_reader_yields_all_none_variables() {
    let v = ThemeVariables::from_css_reader(|_| None);
    let theme: CanvasTheme = v.into();
    assert_eq!(theme, CanvasTheme::light());
}

#[test]
fn reader_populates_direct_mapping() {
    let map = HashMap::from([
        ("--palette-sheet-grid-color", "#abcdef"),
        ("--palette-sheet-header-background", "#fedcba"),
    ]);
    let v = ThemeVariables::from_css_reader(map_reader(map));
    assert_eq!(v.grid_color.as_deref(), Some("#abcdef"));
    assert_eq!(v.header_bg.as_deref(), Some("#fedcba"));
    // Unspecified keys stay None.
    assert!(v.default_text_color.is_none());
    assert!(v.cell_bg.is_none());
}

#[test]
fn empty_string_value_is_treated_as_unset() {
    let map = HashMap::from([("--palette-sheet-grid-color", "   ")]);
    let v = ThemeVariables::from_css_reader(map_reader(map));
    assert!(v.grid_color.is_none());
}

#[test]
fn primary_main_drives_selection_pointing_and_alpha_tints() {
    let map = HashMap::from([("--palette-primary-main", "#1E6FD9")]);
    let v = ThemeVariables::from_css_reader(map_reader(map));
    assert_eq!(v.selection_color.as_deref(), Some("#1E6FD9"));
    assert_eq!(v.pointing.as_deref(), Some("#1E6FD9"));
    assert_eq!(v.selection_fill.as_deref(), Some("rgba(30,111,217,0.12)"));
    assert_eq!(v.pointing_tint.as_deref(), Some("rgba(30,111,217,0.08)"));
}

#[test]
fn primary_main_with_short_hex_expands_correctly() {
    let map = HashMap::from([("--palette-primary-main", "#0af")]);
    let v = ThemeVariables::from_css_reader(map_reader(map));
    // #0af → #00aaff → rgba(0,170,255,...)
    assert_eq!(v.selection_fill.as_deref(), Some("rgba(0,170,255,0.12)"));
}

#[test]
fn malformed_primary_main_skips_alpha_derivation() {
    let map = HashMap::from([("--palette-primary-main", "rgb(30,111,217)")]);
    let v = ThemeVariables::from_css_reader(map_reader(map));
    // Selection / pointing still get the verbatim string — derive_rgba only
    // governs the *_fill / *_tint slots.
    assert_eq!(v.selection_color.as_deref(), Some("rgb(30,111,217)"));
    assert!(v.selection_fill.is_none());
    assert!(v.pointing_tint.is_none());
}

// `with_primary` must be the builder analogue of `--palette-primary-main` in
// the CSS reader: both set the same four fields with the same alpha derivations,
// so a builder-driven theme matches a reader-driven one with the same input.
#[test]
fn with_primary_matches_css_reader_primary_main() {
    let from_builder = ThemeVariables::new().with_primary("#1E6FD9");
    let from_reader = ThemeVariables::from_css_reader(map_reader(HashMap::from([(
        "--palette-primary-main",
        "#1E6FD9",
    )])));
    assert_eq!(from_builder.selection_color, from_reader.selection_color);
    assert_eq!(from_builder.pointing, from_reader.pointing);
    assert_eq!(from_builder.selection_fill, from_reader.selection_fill);
    assert_eq!(from_builder.pointing_tint, from_reader.pointing_tint);
}

#[test]
fn with_primary_skips_alpha_for_non_hex_input() {
    let v = ThemeVariables::new().with_primary("rgb(30,111,217)");
    assert_eq!(v.selection_color.as_deref(), Some("rgb(30,111,217)"));
    assert_eq!(v.pointing.as_deref(), Some("rgb(30,111,217)"));
    assert!(v.selection_fill.is_none());
    assert!(v.pointing_tint.is_none());
}

#[test]
fn explicit_with_selection_fill_overrides_with_primary() {
    let v = ThemeVariables::new()
        .with_primary("#1E6FD9")
        .with_selection_fill("rgba(0,0,0,0.5)");
    assert_eq!(v.selection_fill.as_deref(), Some("rgba(0,0,0,0.5)"));
    // pointing_tint untouched by the override.
    assert_eq!(v.pointing_tint.as_deref(), Some("rgba(30,111,217,0.08)"));
}

#[test]
fn error_main_populates_error_text_color() {
    let map = HashMap::from([("--palette-error-main", "#CC0000")]);
    let v = ThemeVariables::from_css_reader(map_reader(map));
    assert_eq!(v.error_text_color.as_deref(), Some("#CC0000"));
}

#[test]
fn full_upstream_var_set_round_trips_through_canvas_theme() {
    let map = HashMap::from([
        ("--palette-common-white", "#FFFFFF"),
        ("--palette-sheet-grid-color", "#E0E0E0"),
        ("--palette-sheet-grid-separator-color", "#E0E0E0"),
        ("--palette-sheet-header-background", "#FFF"),
        ("--palette-sheet-header-border-color", "#E0E0E0"),
        ("--palette-sheet-header-text-color", "#333"),
        ("--palette-sheet-header-selected-background", "#EEEEEE"),
        ("--palette-sheet-header-selected-color", "#333"),
        ("--palette-sheet-default-text-color", "#2E414D"),
        ("--palette-error-main", "#CC0000"),
        ("--palette-primary-main", "#1E6FD9"),
    ]);
    let theme: CanvasTheme = ThemeVariables::from_css_reader(map_reader(map)).into();
    // Direct keys land verbatim.
    assert_eq!(
        theme.grid_color,
        Cow::<'static, str>::Owned("#E0E0E0".into())
    );
    assert_eq!(theme.cell_bg, Cow::<'static, str>::Owned("#FFFFFF".into()));
    assert_eq!(
        theme.error_text_color,
        Cow::<'static, str>::Owned("#CC0000".into())
    );
    // primary-main propagates through derivation.
    assert_eq!(
        theme.selection_color,
        Cow::<'static, str>::Owned("#1E6FD9".into())
    );
    assert_eq!(theme.pointing, Cow::<'static, str>::Owned("#1E6FD9".into()));
    assert_eq!(
        theme.selection_fill,
        Cow::<'static, str>::Owned("rgba(30,111,217,0.12)".into())
    );
}

#[test]
fn full_override_replaces_every_field() {
    let theme = ThemeVariables::new()
        .with_grid_color("#aaa111")
        .with_grid_separator_color("#aaa222")
        .with_header_bg("#aaa333")
        .with_header_border_color("#aaa444")
        .with_header_text_color("#aaa555")
        .with_header_selected_bg("#aaa666")
        .with_header_selected_color("#aaa777")
        .with_default_text_color("#aaa888")
        .with_error_text_color("#aaa999")
        .with_selection_color("#bbb111")
        .with_cell_bg("#bbb222")
        .with_pointing("#bbb333")
        .with_selection_fill("rgba(1,2,3,0.10)")
        .with_pointing_tint("rgba(4,5,6,0.10)")
        .build();
    let light = CanvasTheme::light();
    assert_ne!(theme, light);
    assert!(matches!(theme.grid_color, Cow::Owned(_)));
    assert!(matches!(theme.grid_separator_color, Cow::Owned(_)));
    assert!(matches!(theme.header_bg, Cow::Owned(_)));
    assert!(matches!(theme.header_border_color, Cow::Owned(_)));
    assert!(matches!(theme.header_text_color, Cow::Owned(_)));
    assert!(matches!(theme.header_selected_bg, Cow::Owned(_)));
    assert!(matches!(theme.header_selected_color, Cow::Owned(_)));
    assert!(matches!(theme.default_text_color, Cow::Owned(_)));
    assert!(matches!(theme.error_text_color, Cow::Owned(_)));
    assert!(matches!(theme.selection_color, Cow::Owned(_)));
    assert!(matches!(theme.cell_bg, Cow::Owned(_)));
    assert!(matches!(theme.pointing, Cow::Owned(_)));
    assert!(matches!(theme.selection_fill, Cow::Owned(_)));
    assert!(matches!(theme.pointing_tint, Cow::Owned(_)));
}
