//! Shared unit-box definitions for the conditional-formatting glyphs.
//!
//! One place defines every glyph shape. The CF renderer keeps the exhaustive
//! `IconGlyph` dispatch and selects a definition here plus resolved color and
//! placement; the backends never see `IconGlyph` or a shape table. All
//! coordinates are unit-box fractions (0..1, Y-down), placed by
//! [`super::place`].
//!
//! # Arrows (D4)
//!
//! One arrow points along +x; every arrow variant is that definition rotated
//! clockwise in Y-down space: `ArrowRight` 0°, `ArrowDown` 90°, `ArrowUp`
//! -90°, `ArrowAngleDown` 45°, `ArrowAngleUp` -45°.
//!
//! The gate passed on a user-supplied comparison set: the same `IconSets`
//! sheet rendered by IronCalc and by LibreOffice draws all five arrow glyphs
//! as one arrow shape rotated, and the rotated definition matches the
//! LibreOffice construction. That is the reference D4 asked for, so the
//! pre-unification cardinal and diagonal tables are replaced by this one
//! definition and its five rotations.

/// One drawing part of a glyph, in unit-box coordinates.
#[derive(Debug, Clone, Copy)]
pub enum GlyphPart {
    /// Closed filled polygon.
    Poly(&'static [(f64, f64)]),
    /// Stroked open segment with a unit-space half-thickness.
    Segment {
        from: (f64, f64),
        to: (f64, f64),
        half: f64,
    },
    /// Filled disc centred in the unit box; `radius` is a unit fraction.
    Circle { radius: f64 },
    /// Five-pointed star inscribed in the unit box.
    Star { inner: f64 },
}

/// Geometry used for the triangle-style CF icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriangleStyle {
    /// Open chevron made from two strokes.
    Chevron,
    /// Solid triangle.
    Filled,
}

impl GlyphPart {
    /// Command slots this part needs in the placement buffer. A circle needs
    /// none: it draws through the native `fill_circle` primitive.
    pub const fn cmd_count(&self) -> usize {
        match self {
            GlyphPart::Poly(v) => v.len() + 1,
            GlyphPart::Segment { .. } => 2,
            GlyphPart::Star { .. } => 11,
            GlyphPart::Circle { .. } => 0,
        }
    }
}

/// A complete glyph: one or more parts, painted in order, rotated around the
/// unit-box centre by `rotation` radians (clock-wise in Y-down space).
#[derive(Debug, Clone, Copy)]
pub struct GlyphDef {
    pub parts: &'static [GlyphPart],
    pub rotation: f64,
}

/// Largest slot count any single built-in part needs. The renderer's stack
/// buffer is this size; a definition that exceeds it must fail a test rather
/// than truncate a path.
pub const MAX_PART_CMDS: usize = 11;

// The single arrow points along +x (x right, y down). Every arrow variant in
// the definitions below is this shape, rotated.
const ARROW_HEAD: [(f64, f64); 3] = [(0.98, 0.5), (0.48, 0.02), (0.48, 0.98)];
const ARROW_SHAFT: [(f64, f64); 4] = [(0.02, 0.38), (0.58, 0.38), (0.58, 0.62), (0.02, 0.62)];
static ARROW_PARTS: [GlyphPart; 2] = [GlyphPart::Poly(&ARROW_HEAD), GlyphPart::Poly(&ARROW_SHAFT)];

const TRIANGLE_UP_FILLED_VERTS: [(f64, f64); 3] = [(0.5, 0.02), (0.98, 0.98), (0.02, 0.98)];
static TRIANGLE_CHEVRON_PARTS: [GlyphPart; 2] = [
    GlyphPart::Segment {
        from: (0.20, 0.65),
        to: (0.50, 0.35),
        half: 0.05,
    },
    GlyphPart::Segment {
        from: (0.50, 0.35),
        to: (0.80, 0.65),
        half: 0.05,
    },
];
static TRIANGLE_FILLED_PARTS: [GlyphPart; 1] = [GlyphPart::Poly(&TRIANGLE_UP_FILLED_VERTS)];
// The flat bar used by the `3Triangles` middle icon and, padded, by the
// rating sets (`Ratings4`, `Ratings5`, `Bozes5`, …). The icon-set bar spans
// the box; the rating bar leaves a gap because the renderer tiles one copy
// per rating point and neighbours would otherwise merge.
const FLAT_RECTANGLE_VERTS: [(f64, f64); 4] =
    [(0.02, 0.36), (0.98, 0.36), (0.98, 0.64), (0.02, 0.64)];
// ~0.66 of the slot, centred, so the gap splits evenly between neighbours.
// The IronCalc app reference draws each rating dash at 0.66 of its slot
// (measured 9.5 px dash, 4.8 px gap) with the same relative thickness.
const RATING_BAR_VERTS: [(f64, f64); 4] = [(0.17, 0.36), (0.83, 0.36), (0.83, 0.64), (0.17, 0.64)];
const RHOMBUS_VERTS: [(f64, f64); 4] = [(0.5, 0.02), (0.98, 0.5), (0.5, 0.98), (0.02, 0.5)];
const FLAG_POLE: [(f64, f64); 4] = [(0.12, 0.02), (0.24, 0.02), (0.24, 0.98), (0.12, 0.98)];
const FLAG_BANNER: [(f64, f64); 5] = [
    (0.24, 0.06),
    (0.96, 0.18),
    (0.78, 0.42),
    (0.96, 0.66),
    (0.24, 0.66),
];
const EXCLAMATION_BAR: [(f64, f64); 4] = [(0.42, 0.06), (0.58, 0.06), (0.58, 0.64), (0.42, 0.64)];
const EXCLAMATION_DOT: [(f64, f64); 4] = [(0.42, 0.76), (0.58, 0.76), (0.58, 0.94), (0.42, 0.94)];
const HEART_VERTS: [(f64, f64); 10] = [
    (0.5, 0.94),
    (0.08, 0.52),
    (0.04, 0.30),
    (0.16, 0.10),
    (0.36, 0.08),
    (0.5, 0.26),
    (0.64, 0.08),
    (0.84, 0.10),
    (0.96, 0.30),
    (0.92, 0.52),
];
const THUMBS_UP_CUFF: [(f64, f64); 4] = [(0.08, 0.36), (0.26, 0.36), (0.26, 0.94), (0.08, 0.94)];
const THUMBS_UP_PALM: [(f64, f64); 4] = [(0.32, 0.44), (0.74, 0.44), (0.74, 0.94), (0.32, 0.94)];
const THUMBS_UP_FINGER: [(f64, f64); 4] = [(0.44, 0.06), (0.62, 0.06), (0.62, 0.44), (0.44, 0.44)];
const THUMBS_DOWN_CUFF: [(f64, f64); 4] = [(0.08, 0.06), (0.26, 0.06), (0.26, 0.64), (0.08, 0.64)];
const THUMBS_DOWN_PALM: [(f64, f64); 4] = [(0.32, 0.06), (0.74, 0.06), (0.74, 0.56), (0.32, 0.56)];
const THUMBS_DOWN_FINGER: [(f64, f64); 4] =
    [(0.44, 0.56), (0.62, 0.56), (0.62, 0.94), (0.44, 0.94)];

/// The historical star inner-radius ratio. The built-in star keeps this value;
/// a consumer that needs a different star supplies another ratio.
pub const STAR_INNER: f64 = 0.382;

// The renderer selects the unit-box geometry and applies its rotation through
// `Placement`; keep the variants in the same order as `IconGlyph`.
/// Build the shared +x arrow with a clockwise rotation in Y-down space.
///
/// Placement applies the angle to the shared parts. This avoids one static
/// shape definition for each arrow direction.
pub const fn arrow(rotation: f64) -> GlyphDef {
    GlyphDef {
        parts: &ARROW_PARTS,
        rotation,
    }
}

/// Build an up- or down-facing triangle icon from shared geometry.
///
/// `rotation` is clockwise in Y-down space. Use a half turn for down-facing
/// variants.
pub const fn triangle(style: TriangleStyle, rotation: f64) -> GlyphDef {
    let parts: &'static [GlyphPart] = match style {
        TriangleStyle::Chevron => &TRIANGLE_CHEVRON_PARTS,
        TriangleStyle::Filled => &TRIANGLE_FILLED_PARTS,
    };
    GlyphDef { parts, rotation }
}

pub const ARROW: GlyphDef = GlyphDef {
    parts: &ARROW_PARTS,
    rotation: 0.0,
};
pub const CIRCLE: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Circle { radius: 0.5 }],
    rotation: 0.0,
};
pub const FLAT_RECTANGLE: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&FLAT_RECTANGLE_VERTS)],
    rotation: 0.0,
};

/// The rating-bar glyph: the padded bar the rating renderer tiles. Not an
/// `IconGlyph` on its own; the renderer selects it for a rating whose glyph
/// is `IconGlyph::FlatRectangle`.
pub const RATING_BAR: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&RATING_BAR_VERTS)],
    rotation: 0.0,
};
pub const RHOMBUS: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&RHOMBUS_VERTS)],
    rotation: 0.0,
};
pub const FLAG: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&FLAG_POLE), GlyphPart::Poly(&FLAG_BANNER)],
    rotation: 0.0,
};
pub const CHECK: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Segment {
            from: (0.12, 0.55),
            to: (0.40, 0.85),
            half: 0.11,
        },
        GlyphPart::Segment {
            from: (0.40, 0.85),
            to: (0.90, 0.16),
            half: 0.11,
        },
    ],
    rotation: 0.0,
};
pub const CROSS: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Segment {
            from: (0.16, 0.16),
            to: (0.84, 0.84),
            half: 0.12,
        },
        GlyphPart::Segment {
            from: (0.84, 0.16),
            to: (0.16, 0.84),
            half: 0.12,
        },
    ],
    rotation: 0.0,
};
pub const EXCLAMATION: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&EXCLAMATION_BAR),
        GlyphPart::Poly(&EXCLAMATION_DOT),
    ],
    rotation: 0.0,
};
pub const STAR: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Star { inner: STAR_INNER }],
    rotation: 0.0,
};
pub const HEART: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&HEART_VERTS)],
    rotation: 0.0,
};
pub const THUMBS_UP: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&THUMBS_UP_CUFF),
        GlyphPart::Poly(&THUMBS_UP_PALM),
        GlyphPart::Poly(&THUMBS_UP_FINGER),
    ],
    rotation: 0.0,
};
pub const THUMBS_DOWN: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&THUMBS_DOWN_CUFF),
        GlyphPart::Poly(&THUMBS_DOWN_PALM),
        GlyphPart::Poly(&THUMBS_DOWN_FINGER),
    ],
    rotation: 0.0,
};

/// Every built-in glyph variant, for capacity and coverage tests.
#[cfg(test)]
fn all_glyphs() -> [GlyphDef; 20] {
    [
        arrow(-std::f64::consts::FRAC_PI_2),
        arrow(0.0),
        arrow(std::f64::consts::FRAC_PI_2),
        arrow(-std::f64::consts::FRAC_PI_4),
        arrow(std::f64::consts::FRAC_PI_4),
        CIRCLE,
        triangle(TriangleStyle::Chevron, 0.0),
        triangle(TriangleStyle::Chevron, std::f64::consts::PI),
        triangle(TriangleStyle::Filled, 0.0),
        triangle(TriangleStyle::Filled, std::f64::consts::PI),
        FLAT_RECTANGLE,
        RHOMBUS,
        FLAG,
        CHECK,
        CROSS,
        EXCLAMATION,
        STAR,
        HEART,
        THUMBS_UP,
        THUMBS_DOWN,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::path::PointF;
    use crate::shape::place::Placement;
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

    #[test]
    fn every_glyph_fits_the_placement_buffer() {
        let all = all_glyphs();
        for def in all.iter().chain(std::iter::once(&RATING_BAR)) {
            for part in def.parts {
                assert!(
                    part.cmd_count() <= MAX_PART_CMDS,
                    "a part needs {} slots, capacity is {MAX_PART_CMDS}",
                    part.cmd_count(),
                );
            }
        }
    }

    #[test]
    fn the_largest_definition_matches_the_published_capacity() {
        let all = all_glyphs();
        let largest = all
            .iter()
            .flat_map(|def| def.parts.iter())
            .chain(RATING_BAR.parts.iter())
            .map(GlyphPart::cmd_count)
            .max()
            .expect("at least one glyph");
        assert_eq!(
            largest, MAX_PART_CMDS,
            "capacity must equal the largest definition; raise it only with the definitions",
        );
    }

    #[test]
    fn every_glyph_has_at_least_one_part() {
        for def in &all_glyphs() {
            assert!(!def.parts.is_empty());
        }
    }

    #[test]
    fn one_definition_per_icon_variant() {
        // `IconGlyph` has 20 variants; the renderer's exhaustive match maps
        // each to one definition here.
        assert_eq!(all_glyphs().len(), 20);
    }

    // D4 rotation-equivalence check: the five arrow variants must be one
    // definition rotated to the five D4 angles, and the placed tip must point
    // the expected way. This checks construction only; fidelity came from the
    // IronCalc/LibreOffice comparison recorded in the module doc.

    #[test]
    fn the_five_arrow_variants_share_one_definition() {
        let arrows = [
            arrow(-FRAC_PI_2),
            arrow(0.0),
            arrow(FRAC_PI_2),
            arrow(-FRAC_PI_4),
            arrow(FRAC_PI_4),
        ];
        for def in &arrows {
            assert!(std::ptr::eq(def.parts.as_ptr(), ARROW_PARTS.as_ptr()));
            assert_eq!(def.parts.len(), ARROW_PARTS.len());
        }
        assert_eq!(arrows[0].rotation, -FRAC_PI_2);
        assert_eq!(arrows[1].rotation, 0.0);
        assert_eq!(arrows[2].rotation, FRAC_PI_2);
        assert_eq!(arrows[3].rotation, -FRAC_PI_4);
        assert_eq!(arrows[4].rotation, FRAC_PI_4);
    }

    #[test]
    fn triangle_variants_share_geometry_and_rotate_for_down() {
        let chevron_up = triangle(TriangleStyle::Chevron, 0.0);
        let chevron_down = triangle(TriangleStyle::Chevron, std::f64::consts::PI);
        let filled_up = triangle(TriangleStyle::Filled, 0.0);
        let filled_down = triangle(TriangleStyle::Filled, std::f64::consts::PI);

        assert_eq!(chevron_up.parts.as_ptr(), chevron_down.parts.as_ptr());
        assert_eq!(chevron_up.parts.len(), 2);
        assert_eq!(filled_up.parts.as_ptr(), filled_down.parts.as_ptr());
        assert_eq!(filled_up.parts.len(), 1);
        assert_eq!(chevron_down.rotation, std::f64::consts::PI);
        assert_eq!(filled_down.rotation, std::f64::consts::PI);
    }

    #[test]
    fn each_rotation_points_the_arrow_the_expected_way() {
        // The unrotated tip is the +x point.
        let tip = PointF::new(0.98, 0.5);
        let place = |def: &GlyphDef| Placement::fit(0.0, 0.0, 1.0, 1.0, def.rotation).unwrap();
        let p = |def: &GlyphDef| place(def).point(tip);

        let right = p(&arrow(0.0));
        assert!(right.x > 0.9 && (right.y - 0.5).abs() < 1e-9);
        let down = p(&arrow(FRAC_PI_2));
        assert!((down.x - 0.5).abs() < 1e-9 && down.y > 0.9);
        let up = p(&arrow(-FRAC_PI_2));
        assert!((up.x - 0.5).abs() < 1e-9 && up.y < 0.1);
        let angle_down = p(&arrow(FRAC_PI_4));
        assert!(angle_down.x > 0.7 && angle_down.y > 0.7);
        let angle_up = p(&arrow(-FRAC_PI_4));
        assert!(angle_up.x > 0.7 && angle_up.y < 0.3);
    }
}
