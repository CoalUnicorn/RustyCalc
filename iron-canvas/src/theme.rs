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

/// Concrete color strings for the Canvas 2D rendering context.
/// One static instance per theme; passed into `CanvasRenderer::new()`.
#[derive(Copy, Clone, PartialEq)]
pub struct CanvasTheme {
    pub grid_color: &'static str,
    pub grid_separator_color: &'static str,
    pub header_bg: &'static str,
    pub header_border_color: &'static str,
    pub header_text_color: &'static str,
    pub header_selected_bg: &'static str,
    pub header_selected_color: &'static str,
    pub default_text_color: &'static str,
    pub selection_color: &'static str,
    pub cell_bg: &'static str,
    pub pointing: &'static str,
    /// rgba() string for the semi-transparent range selection fill.
    pub selection_fill: &'static str,
    /// 8% alpha tint of `pointing`, used as the point-mode range fill.
    pub pointing_tint: &'static str,
}

pub static LIGHT: CanvasTheme = CanvasTheme {
    grid_color: "#E0E0E0",
    grid_separator_color: "#E0E0E0",
    header_bg: "#FFF",
    header_border_color: "#E0E0E0",
    header_text_color: "#333",
    header_selected_bg: "#EEEEEE",
    header_selected_color: "#333",
    default_text_color: "#2E414D",
    selection_color: "#17A2D3",
    cell_bg: "#FFFFFF",
    pointing: "#1E6FD9",
    selection_fill: "rgba(23,162,211,0.12)",
    pointing_tint: "rgba(30,111,217,0.08)",
};

pub static DARK: CanvasTheme = CanvasTheme {
    grid_color: "#3A3A3A",
    grid_separator_color: "#3A3A3A",
    header_bg: "#1E1E1E",
    header_border_color: "#3A3A3A",
    header_text_color: "#CCC",
    header_selected_bg: "#2D2D2D",
    header_selected_color: "#CCC",
    default_text_color: "#D4D4D4",
    selection_color: "#17A2D3",
    cell_bg: "#121212",
    pointing: "#1E6FD9",
    selection_fill: "rgba(23,162,211,0.18)",
    pointing_tint: "rgba(30,111,217,0.08)",
};
