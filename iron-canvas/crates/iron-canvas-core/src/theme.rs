use std::borrow::Cow;

/// Color palette for formula reference overlays.
///
/// Assigned round-robin by token insertion order. Mirrors IronCalc's
/// palette so cross-app formulas share a consistent visual language.
pub const FORMULA_REF_COLORS: &[&str] = &[
    "#59B9BC", // Cyan
    "#EC5753", // Flamingo
    "#3358B7", // Blue
    "#F0C419", // Yellow
    "#28A745", // Emerald
    "#8B5CF6", // Violet
    "#9B2335", // Burgundy
    "#8DB600", // Wasabi
    "#E53E3E", // Red
    "#0B9A8A", // Teal
];

/// 8% alpha tints of `FORMULA_REF_COLORS`, indexed in lockstep. Used as the
/// fill for tinted dashed overlays so paint never needs to allocate an
/// `rgba(...)` string per frame.
pub const FORMULA_REF_TINTS: &[&str] = &[
    "rgba(89,185,188,0.08)", // Cyan
    "rgba(236,87,83,0.08)",  // Flamingo
    "rgba(51,88,183,0.08)",  // Blue
    "rgba(240,196,25,0.08)", // Yellow
    "rgba(40,167,69,0.08)",  // Emerald
    "rgba(139,92,246,0.08)", // Violet
    "rgba(155,35,53,0.08)",  // Burgundy
    "rgba(141,182,0,0.08)",  // Wasabi
    "rgba(229,62,62,0.08)",  // Red
    "rgba(11,154,138,0.08)", // Teal
];

/// Resolved color palette consumed by the renderer.
///
/// Fields are `Cow<'static, str>` so a built-in theme (`LIGHT` / `DARK`) costs
/// nothing — `Cow::Borrowed` carries a `&'static str` that the painter cache
/// ptr-eqs in O(1) — while a host-page theme can ship owned `String`s through
/// `Cow::Owned` without dropping back to a `&'static` lifetime.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasTheme {
    pub grid_color: Cow<'static, str>,
    pub grid_separator_color: Cow<'static, str>,
    pub header_bg: Cow<'static, str>,
    pub header_border_color: Cow<'static, str>,
    pub header_text_color: Cow<'static, str>,
    pub header_selected_bg: Cow<'static, str>,
    pub header_selected_color: Cow<'static, str>,
    pub default_text_color: Cow<'static, str>,
    /// Text color for cells whose value is an IronCalc error
    /// (`CellKind::Error` — `#VALUE!`, `#DIV/0!`, `#REF!`, etc.).
    pub error_text_color: Cow<'static, str>,
    pub selection_color: Cow<'static, str>,
    pub cell_bg: Cow<'static, str>,
    pub pointing: Cow<'static, str>,
    /// rgba() string for the semi-transparent range selection fill.
    pub selection_fill: Cow<'static, str>,
    /// 8% alpha tint of `pointing`, used as the point-mode range fill.
    pub pointing_tint: Cow<'static, str>,
}

impl CanvasTheme {
    /// Built-in light palette.
    pub fn light() -> Self {
        LIGHT
    }

    /// Built-in dark palette.
    pub fn dark() -> Self {
        DARK
    }
}

pub const LIGHT: CanvasTheme = CanvasTheme {
    grid_color: Cow::Borrowed("#E0E0E0"),
    grid_separator_color: Cow::Borrowed("#E0E0E0"),
    header_bg: Cow::Borrowed("#FFF"),
    header_border_color: Cow::Borrowed("#E0E0E0"),
    header_text_color: Cow::Borrowed("#333"),
    header_selected_bg: Cow::Borrowed("#EEEEEE"),
    header_selected_color: Cow::Borrowed("#333"),
    default_text_color: Cow::Borrowed("#2E414D"),
    error_text_color: Cow::Borrowed("#CC0000"),
    selection_color: Cow::Borrowed("#17A2D3"),
    cell_bg: Cow::Borrowed("#FFFFFF"),
    pointing: Cow::Borrowed("#1E6FD9"),
    selection_fill: Cow::Borrowed("rgba(23,162,211,0.12)"),
    pointing_tint: Cow::Borrowed("rgba(30,111,217,0.08)"),
};

pub const DARK: CanvasTheme = CanvasTheme {
    grid_color: Cow::Borrowed("#3A3A3A"),
    grid_separator_color: Cow::Borrowed("#3A3A3A"),
    header_bg: Cow::Borrowed("#1E1E1E"),
    header_border_color: Cow::Borrowed("#3A3A3A"),
    header_text_color: Cow::Borrowed("#CCC"),
    header_selected_bg: Cow::Borrowed("#2D2D2D"),
    header_selected_color: Cow::Borrowed("#CCC"),
    default_text_color: Cow::Borrowed("#D4D4D4"),
    error_text_color: Cow::Borrowed("#FF6B6B"),
    selection_color: Cow::Borrowed("#17A2D3"),
    cell_bg: Cow::Borrowed("#121212"),
    pointing: Cow::Borrowed("#1E6FD9"),
    selection_fill: Cow::Borrowed("rgba(23,162,211,0.18)"),
    pointing_tint: Cow::Borrowed("rgba(30,111,217,0.08)"),
};

/// Host-page input shape for a `CanvasTheme`. Each field is `Option<String>`;
/// any field left `None` falls back to `CanvasTheme::light()` when converted.
///
/// Bridges IronCalc upstream's CSS-var contract (`--palette-sheet-*`,
/// `--palette-primary-main`, etc.) to the renderer's resolved palette.
/// `from_css_reader` (and the wasm32 `iron_canvas_canvas2d::theme_from_element`
/// helpers that wrap it) populate these fields from a DOM element's computed
/// style; the `From<ThemeVariables> for CanvasTheme` impl performs the
/// per-field fallback to `LIGHT`.
///
/// # Derived fields
///
/// Most fields read a single matching CSS var, but the "selection blue" group
/// has no dedicated keys — one `--palette-primary-main` lookup fans out to all
/// four:
///
/// - `selection_color`, `pointing` from `--palette-primary-main`.
/// - `selection_fill` from `--palette-primary-main` at ~12% alpha.
/// - `pointing_tint` from `--palette-primary-main` at ~8% alpha.
#[derive(Default, Clone, Debug)]
pub struct ThemeVariables {
    pub grid_color: Option<String>,
    pub grid_separator_color: Option<String>,
    pub header_bg: Option<String>,
    pub header_border_color: Option<String>,
    pub header_text_color: Option<String>,
    pub header_selected_bg: Option<String>,
    pub header_selected_color: Option<String>,
    pub default_text_color: Option<String>,
    pub error_text_color: Option<String>,
    pub selection_color: Option<String>,
    pub cell_bg: Option<String>,
    pub pointing: Option<String>,
    pub selection_fill: Option<String>,
    pub pointing_tint: Option<String>,
}

impl ThemeVariables {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_grid_color(mut self, v: impl Into<String>) -> Self {
        self.grid_color = Some(v.into());
        self
    }

    pub fn with_grid_separator_color(mut self, v: impl Into<String>) -> Self {
        self.grid_separator_color = Some(v.into());
        self
    }

    pub fn with_header_bg(mut self, v: impl Into<String>) -> Self {
        self.header_bg = Some(v.into());
        self
    }

    pub fn with_header_border_color(mut self, v: impl Into<String>) -> Self {
        self.header_border_color = Some(v.into());
        self
    }

    pub fn with_header_text_color(mut self, v: impl Into<String>) -> Self {
        self.header_text_color = Some(v.into());
        self
    }

    pub fn with_header_selected_bg(mut self, v: impl Into<String>) -> Self {
        self.header_selected_bg = Some(v.into());
        self
    }

    pub fn with_header_selected_color(mut self, v: impl Into<String>) -> Self {
        self.header_selected_color = Some(v.into());
        self
    }

    pub fn with_default_text_color(mut self, v: impl Into<String>) -> Self {
        self.default_text_color = Some(v.into());
        self
    }

    pub fn with_error_text_color(mut self, v: impl Into<String>) -> Self {
        self.error_text_color = Some(v.into());
        self
    }

    pub fn with_selection_color(mut self, v: impl Into<String>) -> Self {
        self.selection_color = Some(v.into());
        self
    }

    pub fn with_cell_bg(mut self, v: impl Into<String>) -> Self {
        self.cell_bg = Some(v.into());
        self
    }

    pub fn with_pointing(mut self, v: impl Into<String>) -> Self {
        self.pointing = Some(v.into());
        self
    }

    pub fn with_selection_fill(mut self, v: impl Into<String>) -> Self {
        self.selection_fill = Some(v.into());
        self
    }

    pub fn with_pointing_tint(mut self, v: impl Into<String>) -> Self {
        self.pointing_tint = Some(v.into());
        self
    }

    /// Set the four fields IronCalc upstream derives from `--palette-primary-main`
    /// in one call: `selection_color`, `pointing`, `selection_fill` (12% alpha),
    /// `pointing_tint` (8% alpha). Mirrors `from_css_reader`'s primary-main path
    /// so a builder-driven theme matches a CSS-reader-driven one.
    ///
    /// Non-hex inputs (`rgb(...)`, named colors) propagate verbatim to
    /// `selection_color` / `pointing` but leave the alpha tints unset, matching
    /// the reader's `malformed_primary_main_skips_alpha_derivation` behavior.
    /// Granular setters (`with_selection_fill`, `with_pointing_tint`) called
    /// afterwards still override.
    pub fn with_primary(mut self, v: impl Into<String>) -> Self {
        let v = v.into();
        self.selection_fill = derive_rgba(&v, 0.12);
        self.pointing_tint = derive_rgba(&v, 0.08);
        self.selection_color = Some(v.clone());
        self.pointing = Some(v);
        self
    }

    pub fn build(self) -> CanvasTheme {
        self.into()
    }

    /// Build `ThemeVariables` from an opaque CSS-var reader.
    ///
    /// `reader` is invoked once per upstream `--palette-*` key. Returning
    /// `None` (or an empty string after trim, which the helper treats as
    /// `None`) leaves the corresponding field unset; the `From` impl then
    /// falls back to `CanvasTheme::light()`. The DOM-bridge wrappers
    /// (`iron_canvas_canvas2d::theme_from_element::{from_element, from_root}`)
    /// close over a `CssStyleDeclaration::get_property_value` call; tests pass an
    /// in-memory `HashMap` lookup so the derivation logic stays
    /// host-testable.
    ///
    /// Derivation: a single `--palette-primary-main` lookup populates
    /// `selection_color`, `pointing`, and (via `derive_rgba`)
    /// `selection_fill` (12% alpha) and `pointing_tint` (8% alpha). This
    /// matches IronCalc upstream's contract — primary-main is the only
    /// "selection blue" key consumers are expected to set.
    pub fn from_css_reader(reader: impl Fn(&str) -> Option<String>) -> Self {
        let read = |key: &str| reader(key).filter(|s| !s.trim().is_empty());

        let primary = read("--palette-primary-main");
        let primary_with_alpha =
            |alpha: f64| primary.as_deref().and_then(|p| derive_rgba(p, alpha));

        Self {
            grid_color: read("--palette-sheet-grid-color"),
            grid_separator_color: read("--palette-sheet-grid-separator-color"),
            header_bg: read("--palette-sheet-header-background"),
            header_border_color: read("--palette-sheet-header-border-color"),
            header_text_color: read("--palette-sheet-header-text-color"),
            header_selected_bg: read("--palette-sheet-header-selected-background"),
            header_selected_color: read("--palette-sheet-header-selected-color"),
            default_text_color: read("--palette-sheet-default-text-color"),
            error_text_color: read("--palette-error-main"),
            selection_color: primary.clone(),
            cell_bg: read("--palette-common-white"),
            pointing: primary.clone(),
            selection_fill: primary_with_alpha(0.12),
            pointing_tint: primary_with_alpha(0.08),
        }
    }
}

/// Hex `#RRGGBB` (or `#RGB`) -> `rgba(r,g,b,alpha)`. Returns `None` for any
/// input that isn't a recognized hex literal (`rgb()`, `hsl()`, named colors,
/// or a malformed string), so the caller falls back to the LIGHT default
/// instead of synthesising a broken fill string.
fn derive_rgba(hex: &str, alpha: f64) -> Option<String> {
    // `is_ascii` guard: `hex.len()` is a byte count, but the arms below slice
    // by byte index — a multibyte value of byte-length 3 or 6 would slice
    // through a UTF-8 char boundary and panic. ASCII hex is the only valid form.
    let hex = hex.trim().strip_prefix('#').filter(|h| h.is_ascii())?;
    let (r, g, b) = match hex.len() {
        3 => (
            u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?,
            u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?,
            u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?,
        ),
        6 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        ),
        _ => return None,
    };
    Some(format!("rgba({},{},{},{:.2})", r, g, b, alpha))
}

impl From<ThemeVariables> for CanvasTheme {
    fn from(v: ThemeVariables) -> Self {
        let light = CanvasTheme::light();
        // Each `Some(s)` becomes `Cow::Owned`; missing fields keep the
        // `Cow::Borrowed` from `light` so the painter cache fast path stays
        // alive for the unspecified portion of the palette. Comments below
        // pin each canvas field to its upstream CSS-var key.
        CanvasTheme {
            // `--palette-sheet-grid-color`
            grid_color: v.grid_color.map(Cow::Owned).unwrap_or(light.grid_color),
            // `--palette-sheet-grid-separator-color`
            grid_separator_color: v
                .grid_separator_color
                .map(Cow::Owned)
                .unwrap_or(light.grid_separator_color),
            // `--palette-sheet-header-background`
            header_bg: v.header_bg.map(Cow::Owned).unwrap_or(light.header_bg),
            // `--palette-sheet-header-border-color`
            header_border_color: v
                .header_border_color
                .map(Cow::Owned)
                .unwrap_or(light.header_border_color),
            // `--palette-sheet-header-text-color`
            header_text_color: v
                .header_text_color
                .map(Cow::Owned)
                .unwrap_or(light.header_text_color),
            // `--palette-sheet-header-selected-background`
            header_selected_bg: v
                .header_selected_bg
                .map(Cow::Owned)
                .unwrap_or(light.header_selected_bg),
            // `--palette-sheet-header-selected-color`
            header_selected_color: v
                .header_selected_color
                .map(Cow::Owned)
                .unwrap_or(light.header_selected_color),
            // `--palette-sheet-default-text-color`
            default_text_color: v
                .default_text_color
                .map(Cow::Owned)
                .unwrap_or(light.default_text_color),
            // `--palette-error-main`
            error_text_color: v
                .error_text_color
                .map(Cow::Owned)
                .unwrap_or(light.error_text_color),
            // `--palette-primary-main`
            selection_color: v
                .selection_color
                .map(Cow::Owned)
                .unwrap_or(light.selection_color),
            // `--palette-common-white` (background-default)
            cell_bg: v.cell_bg.map(Cow::Owned).unwrap_or(light.cell_bg),
            // `--palette-primary-main`
            pointing: v.pointing.map(Cow::Owned).unwrap_or(light.pointing),
            // `--palette-primary-main` × 12% alpha (derived by
            // `from_css_reader` / `with_primary`; this impl is pass-through).
            selection_fill: v
                .selection_fill
                .map(Cow::Owned)
                .unwrap_or(light.selection_fill),
            // `--palette-primary-main` × 8% alpha (derived by
            // `from_css_reader` / `with_primary`; this impl is pass-through).
            pointing_tint: v
                .pointing_tint
                .map(Cow::Owned)
                .unwrap_or(light.pointing_tint),
        }
    }
}
