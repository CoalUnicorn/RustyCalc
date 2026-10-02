//! Shared unit-box definitions for the conditional-formatting glyphs.
//!
//! One place defines every glyph shape. The CF renderer keeps the exhaustive
//! `IconGlyph` dispatch and selects a definition here plus resolved color and
//! placement; the backends never see `IconGlyph` or a shape table. All
//! coordinates are unit-box fractions (0..1, Y-down), placed by
//! [`super::place`].
//!
//! The arrow definitions are still the pre-unification tables: cardinal and
//! diagonal arrows keep their distinct head proportions and segment shafts.
//! The single rotated arrow in D4 of the design is deferred until a saved
//! Excel reference exists to compare all five glyphs at the same size; this
//! module landing does not depend on that gate.
//!
//! The gate was attempted: the only in-tree conditional-formatting fixture
//! (`IronCalc/xlsx/tests/conditional_formatting/cf_tests.xlsx`, sheet
//! `IconSets`) was rendered with LibreOffice. That is not an Excel reference,
//! it draws its own colored icon artwork, and its diagonal glyph is not a
//! rotation of its cardinal glyph. It therefore does not support replacing
//! the definitions, and D4 keeps them as they are.

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

/// A complete glyph: one or more parts, painted in order.
#[derive(Debug, Clone, Copy)]
pub struct GlyphDef {
    pub parts: &'static [GlyphPart],
}

/// Largest slot count any single built-in part needs. The renderer's stack
/// buffer is this size; a definition that exceeds it must fail a test rather
/// than truncate a path.
pub const MAX_PART_CMDS: usize = 11;

// Arrow glyphs: a triangular head plus a rectangular shaft, in a unit box
// (x right, y down).
const ARROW_UP_HEAD: [(f64, f64); 3] = [(0.5, 0.02), (0.98, 0.52), (0.02, 0.52)];
const ARROW_UP_SHAFT: [(f64, f64); 4] = [(0.38, 0.42), (0.62, 0.42), (0.62, 0.98), (0.38, 0.98)];
const ARROW_DOWN_HEAD: [(f64, f64); 3] = [(0.5, 0.98), (0.98, 0.48), (0.02, 0.48)];
const ARROW_DOWN_SHAFT: [(f64, f64); 4] = [(0.38, 0.02), (0.62, 0.02), (0.62, 0.58), (0.38, 0.58)];
const ARROW_RIGHT_HEAD: [(f64, f64); 3] = [(0.98, 0.5), (0.48, 0.02), (0.48, 0.98)];
const ARROW_RIGHT_SHAFT: [(f64, f64); 4] = [(0.02, 0.38), (0.58, 0.38), (0.58, 0.62), (0.02, 0.62)];
const ARROW_ANGLE_UP_HEAD: [(f64, f64); 3] = [(1.0, 0.02), (0.40, 0.06), (0.96, 0.60)];
const ARROW_ANGLE_DOWN_HEAD: [(f64, f64); 3] = [(1.0, 0.98), (0.40, 0.94), (0.96, 0.40)];

const TRIANGLE_UP_FILLED_VERTS: [(f64, f64); 3] = [(0.5, 0.02), (0.98, 0.98), (0.02, 0.98)];
const TRIANGLE_DOWN_FILLED_VERTS: [(f64, f64); 3] = [(0.02, 0.02), (0.98, 0.02), (0.5, 0.98)];
const FLAT_RECTANGLE_VERTS: [(f64, f64); 4] =
    [(0.02, 0.36), (0.98, 0.36), (0.98, 0.64), (0.02, 0.64)];
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

// One definition per `IconGlyph` variant. The renderer's exhaustive match
// selects among these; keep the variants in the same order as `IconGlyph`.
pub const ARROW_UP: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&ARROW_UP_HEAD),
        GlyphPart::Poly(&ARROW_UP_SHAFT),
    ],
};
pub const ARROW_RIGHT: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&ARROW_RIGHT_HEAD),
        GlyphPart::Poly(&ARROW_RIGHT_SHAFT),
    ],
};
pub const ARROW_DOWN: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&ARROW_DOWN_HEAD),
        GlyphPart::Poly(&ARROW_DOWN_SHAFT),
    ],
};
pub const ARROW_ANGLE_UP: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&ARROW_ANGLE_UP_HEAD),
        GlyphPart::Segment {
            from: (0.06, 0.94),
            to: (0.56, 0.44),
            half: 0.11,
        },
    ],
};
pub const ARROW_ANGLE_DOWN: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&ARROW_ANGLE_DOWN_HEAD),
        GlyphPart::Segment {
            from: (0.06, 0.06),
            to: (0.56, 0.56),
            half: 0.11,
        },
    ],
};
pub const CIRCLE: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Circle { radius: 0.5 }],
};
pub const TRIANGLE_UP: GlyphDef = GlyphDef {
    parts: &[
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
    ],
};
pub const TRIANGLE_DOWN: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Segment {
            from: (0.20, 0.35),
            to: (0.50, 0.65),
            half: 0.05,
        },
        GlyphPart::Segment {
            from: (0.50, 0.65),
            to: (0.80, 0.35),
            half: 0.05,
        },
    ],
};
pub const TRIANGLE_UP_FILLED: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&TRIANGLE_UP_FILLED_VERTS)],
};
pub const TRIANGLE_DOWN_FILLED: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&TRIANGLE_DOWN_FILLED_VERTS)],
};
pub const FLAT_RECTANGLE: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&FLAT_RECTANGLE_VERTS)],
};
pub const RHOMBUS: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&RHOMBUS_VERTS)],
};
pub const FLAG: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&FLAG_POLE), GlyphPart::Poly(&FLAG_BANNER)],
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
};
pub const EXCLAMATION: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&EXCLAMATION_BAR),
        GlyphPart::Poly(&EXCLAMATION_DOT),
    ],
};
pub const STAR: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Star { inner: STAR_INNER }],
};
pub const HEART: GlyphDef = GlyphDef {
    parts: &[GlyphPart::Poly(&HEART_VERTS)],
};
pub const THUMBS_UP: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&THUMBS_UP_CUFF),
        GlyphPart::Poly(&THUMBS_UP_PALM),
        GlyphPart::Poly(&THUMBS_UP_FINGER),
    ],
};
pub const THUMBS_DOWN: GlyphDef = GlyphDef {
    parts: &[
        GlyphPart::Poly(&THUMBS_DOWN_CUFF),
        GlyphPart::Poly(&THUMBS_DOWN_PALM),
        GlyphPart::Poly(&THUMBS_DOWN_FINGER),
    ],
};

/// Every built-in definition, for capacity and coverage tests.
pub const ALL: &[&GlyphDef] = &[
    &ARROW_UP,
    &ARROW_RIGHT,
    &ARROW_DOWN,
    &ARROW_ANGLE_UP,
    &ARROW_ANGLE_DOWN,
    &CIRCLE,
    &TRIANGLE_UP,
    &TRIANGLE_DOWN,
    &TRIANGLE_UP_FILLED,
    &TRIANGLE_DOWN_FILLED,
    &FLAT_RECTANGLE,
    &RHOMBUS,
    &FLAG,
    &CHECK,
    &CROSS,
    &EXCLAMATION,
    &STAR,
    &HEART,
    &THUMBS_UP,
    &THUMBS_DOWN,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::path::PointF;
    use crate::shape::place::Placement;

    #[test]
    fn every_glyph_fits_the_placement_buffer() {
        for def in ALL {
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
        let largest = ALL
            .iter()
            .flat_map(|def| def.parts.iter())
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
        for def in ALL {
            assert!(!def.parts.is_empty());
        }
    }

    #[test]
    fn one_definition_per_icon_variant() {
        // `IconGlyph` has 20 variants; the renderer's exhaustive match maps
        // each to one definition here.
        assert_eq!(ALL.len(), 20);
    }

    // D4 construction check. These tests pin the *construction* relationship
    // between the current arrow definitions; they do not assert Excel
    // fidelity. The single-arrow proposal would replace the cardinal and
    // diagonal tables with one arrow rotated per variant, which requires an
    // Excel reference comparison that is not available (see the module doc).
    // The cardinal test passes today; the diagonal test fails the moment the
    // arrows are unified by rotation, so a change must come with the gate.

    fn rotated(vertices: &[(f64, f64)], angle: f64) -> Vec<(f64, f64)> {
        let place = Placement::fit(0.0, 0.0, 1.0, 1.0, angle).unwrap();
        vertices
            .iter()
            .map(|&(x, y)| {
                let p = place.point(PointF::new(x, y));
                (p.x, p.y)
            })
            .collect()
    }

    fn same_vertex_set(a: &[(f64, f64)], b: &[(f64, f64)]) -> bool {
        const EPS: f64 = 1e-9;
        a.len() == b.len()
            && a.iter().all(|pa| {
                b.iter()
                    .any(|pb| (pa.0 - pb.0).abs() < EPS && (pa.1 - pb.1).abs() < EPS)
            })
    }

    #[test]
    fn cardinal_arrows_are_rotations_of_each_other() {
        use std::f64::consts::{FRAC_PI_2, PI};
        assert!(same_vertex_set(
            &rotated(&ARROW_UP_HEAD, FRAC_PI_2),
            &ARROW_RIGHT_HEAD,
        ));
        assert!(same_vertex_set(
            &rotated(&ARROW_UP_SHAFT, FRAC_PI_2),
            &ARROW_RIGHT_SHAFT,
        ));
        assert!(same_vertex_set(
            &rotated(&ARROW_UP_HEAD, PI),
            &ARROW_DOWN_HEAD
        ));
        assert!(same_vertex_set(
            &rotated(&ARROW_UP_SHAFT, PI),
            &ARROW_DOWN_SHAFT,
        ));
    }

    #[test]
    fn diagonal_arrows_are_not_rotations_of_the_cardinal_arrow() {
        use std::f64::consts::FRAC_PI_4;
        for angle in [-FRAC_PI_4, FRAC_PI_4] {
            assert!(
                !same_vertex_set(&rotated(&ARROW_UP_HEAD, angle), &ARROW_ANGLE_UP_HEAD),
                "the diagonal head is a rotated cardinal head at {angle}; \
                 unifying the arrows changed their construction without the D4 gate",
            );
        }
    }
}
