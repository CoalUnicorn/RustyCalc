// Engine-agnostic cell styling — mirrors the renderer's read-set exactly.
// Colors are CSS strings (resolved against the theme at paint time, not here).

use crate::geometry::prim::Side;

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

impl Border {
    /// The border item stored on `side`, if any.
    pub fn get(&self, side: Side) -> Option<&BorderItem> {
        match side {
            Side::Left => self.left.as_ref(),
            Side::Top => self.top.as_ref(),
            Side::Right => self.right.as_ref(),
            Side::Bottom => self.bottom.as_ref(),
        }
    }
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

/// Backend-independent identity of one icon glyph. Mirrors IronCalc's `Icon`
/// enum 1:1 so the bridge mapping is total; the renderer owns the geometry of
/// each variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconGlyph {
    ArrowUp,
    ArrowRight,
    ArrowDown,
    ArrowAngleUp,
    ArrowAngleDown,
    Circle,
    TriangleUp,
    TriangleDown,
    TriangleUpFilled,
    TriangleDownFilled,
    FlatRectangle,
    Rhombus,
    Flag,
    Check,
    Cross,
    Exclamation,
    Star,
    Heart,
    ThumbsUp,
    ThumbsDown,
}

/// One evaluated icon-set decoration: the engine-selected glyph, its resolved
/// color, and whether the cell value stays visible.
#[derive(Clone, Debug, PartialEq)]
pub struct IconSpec {
    pub glyph: IconGlyph,
    /// Resolved CSS color. `None` when the engine color is unresolved; the
    /// renderer then picks its default.
    pub color: Option<String>,
    /// When false, the painted cell value is hidden.
    pub show_value: bool,
}

/// One evaluated data-bar decoration.
#[derive(Clone, Debug, PartialEq)]
pub struct DataBarSpec {
    /// Resolved CSS color on the positive side of the axis. Always present —
    /// the bridge substitutes Excel's default blue for an unresolved color.
    pub positive_color: String,
    /// Resolved CSS color on the negative side; `None` when unresolved.
    pub negative_color: Option<String>,
    /// Gradient fill when true; solid fill when false.
    pub is_gradient: bool,
    /// Normalized endpoint position in `[0, 1]`. The bar runs from
    /// `axis_position` to `value`; it is not a width measured from the left.
    pub value: f64,
    /// Normalized position in `[0, 1]` where the zero axis falls.
    pub axis_position: f64,
    /// When false, the painted cell value is hidden.
    pub show_value: bool,
}

/// One evaluated rating decoration. Most glyphs repeat `count` times out of
/// `max`; a circle uses `count` as its rank in the scale.
#[derive(Clone, Debug, PartialEq)]
pub struct RatingSpec {
    pub glyph: IconGlyph,
    /// Resolved CSS color; `None` when the engine color is unresolved.
    pub color: Option<String>,
    /// Engine `count`: number of glyphs, or the one-based circle rank.
    pub count: u32,
    /// Engine `max`: total glyphs or circle ranks in the scale.
    pub max: u32,
    /// When false, the painted cell value is hidden.
    pub show_value: bool,
}

/// Evaluated conditional-formatting overlay for one cell. The three categories
/// are independent: the engine resolves icons, data bars, and ratings
/// separately, so all of them can apply to the same cell at once. A cell with
/// no decoration is [`CellDecoration::is_empty`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellDecoration {
    pub icon: Option<IconSpec>,
    pub data_bar: Option<DataBarSpec>,
    pub rating: Option<RatingSpec>,
}

impl CellDecoration {
    /// True when no category applies — the value a model reports as `Absent`.
    pub fn is_empty(&self) -> bool {
        self.icon.is_none() && self.data_bar.is_none() && self.rating.is_none()
    }
}
