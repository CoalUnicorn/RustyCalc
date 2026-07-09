// Engine-agnostic cell styling — mirrors the renderer's read-set exactly.
// Colors are CSS strings (resolved against the theme at paint time, not here).

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellStyle {
    pub fill_color: Option<String>,
    pub font: FontStyle,
    pub alignment: Option<Alignment>,
    pub border: Border,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontStyle {
    pub name: String, // "" => theme default family
    pub size: f64,
    pub color: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

impl Default for FontStyle {
    fn default() -> Self {
        FontStyle {
            name: String::new(),
            size: 11.0,
            color: None,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Alignment {
    pub horizontal: HAlign,
    pub vertical: VAlign,
    pub wrap_text: bool,
}

// Mirror IronCalc HorizontalAlignment 1:1 (8 variants) so the bridge From is total.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HAlign {
    Center,
    CenterContinuous,
    Distributed,
    Fill,
    #[default]
    General,
    Justify,
    Left,
    Right,
}

// Mirror IronCalc VerticalAlignment 1:1 (5 variants).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VAlign {
    #[default]
    Bottom,
    Center,
    Distributed,
    Justify,
    Top,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Border {
    pub left: Option<BorderItem>,
    pub right: Option<BorderItem>,
    pub top: Option<BorderItem>,
    pub bottom: Option<BorderItem>,
    pub diagonal_up: bool,
    pub diagonal_down: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BorderItem {
    pub style: BorderStyle,
    pub color: Option<String>,
}

// Verified against IronCalc/base/src/types.rs — exactly these 9 (no plain `Dashed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderStyle {
    Thin,
    Medium,
    Thick,
    Double,
    Dotted,
    SlantDashDot,
    MediumDashed,
    MediumDashDotDot,
    MediumDashDot,
}

/// Cell value class. The renderer right-aligns `Number` and colors `Error`;
/// everything else renders as `Text`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CellKind {
    #[default]
    Text,
    Number,
    Logical,
    Error,
}

/// Conditional-formatting decoration overlay. `IconSpec = String` is a v1
/// placeholder (the icon name) — the renderer resolves an icon to no pixels
/// yet (no glyph system), so no painted pixel depends on a richer icon enum.
pub type IconSpec = String;

/// CF decorations today, but `#[non_exhaustive]` so non-CF per-cell visuals
/// (sparklines, comment markers) can be added without breaking downstream
/// matches. Variants resolve into `Painter` primitives at the renderer, so a
/// new variant needs no new `Painter` method.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum CellDecoration {
    DataBar(DataBarSpec),
    Icon(IconSpec),
    Rating(RatingSpec),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DataBarSpec {
    pub fraction: f64, // 0.0..=1.0
    pub color: String, // CSS color
}

#[derive(Clone, Debug, PartialEq)]
pub struct RatingSpec {
    pub stars: u32,
    pub filled: u32,
}
