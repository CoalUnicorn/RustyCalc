//! Placement of unit-box shape definitions into a target pixel box.
//!
//! One [`Placement`] carries the uniform scale, the target centre, and the
//! rotation. It maps unit-box coordinates (0..1, Y-down) into logical pixels.
//! The scale is the smaller axis of the target box, so a circle stays round;
//! the placed shape is centred in the box. All functions are allocation-free:
//! the caller owns the command buffer.

use crate::geometry::path::{ArcSpec, PathCmd, PointF};

/// Place the unit box into a target box: uniform scale, then rotation around
/// the unit-box centre, then translation into the target box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Uniform scale: the smaller of the two target axes.
    pub scale: f64,
    /// Target box centre in logical pixels.
    pub center: PointF,
    /// Rotation in radians. Positive turns clockwise in Y-down space.
    pub angle: f64,
}

impl Placement {
    /// Fit the unit box into `(x, y, w, h)`, centred, with rotation `angle`.
    /// `None` when a target dimension is nonpositive or non-finite, so a
    /// degenerate box paints nothing.
    pub fn fit(x: f64, y: f64, w: f64, h: f64, angle: f64) -> Option<Self> {
        if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() {
            return None;
        }
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        Some(Self {
            // The smaller axis keeps the shape uniform instead of stretching
            // it into an ellipse.
            scale: w.min(h),
            center: PointF::new(x + w / 2.0, y + h / 2.0),
            angle,
        })
    }

    /// Place one unit-box point.
    pub fn point(&self, p: PointF) -> PointF {
        let dx = (p.x - 0.5) * self.scale;
        let dy = (p.y - 0.5) * self.scale;
        let (sin, cos) = self.angle.sin_cos();
        PointF::new(
            self.center.x + dx * cos - dy * sin,
            self.center.y + dx * sin + dy * cos,
        )
    }

    /// Place a unit-space length (a radius or a stroke half-thickness).
    pub fn length(&self, unit_len: f64) -> f64 {
        unit_len * self.scale
    }

    /// Place one command. Endpoints and curve controls transform as points.
    /// An arc keeps its sweep; its centre transforms, its radius scales, and
    /// its start angle gains the placement rotation.
    pub fn transform(&self, cmd: &PathCmd) -> PathCmd {
        match cmd {
            PathCmd::Move(p) => PathCmd::Move(self.point(*p)),
            PathCmd::Line(p) => PathCmd::Line(self.point(*p)),
            PathCmd::Quad(c, e) => PathCmd::Quad(self.point(*c), self.point(*e)),
            PathCmd::Cubic(a, b, e) => {
                PathCmd::Cubic(self.point(*a), self.point(*b), self.point(*e))
            }
            PathCmd::Arc(spec) => PathCmd::Arc(ArcSpec::new(
                self.point(spec.center),
                spec.radius * self.scale,
                spec.start_angle + self.angle,
                spec.sweep_angle,
            )),
            PathCmd::Close => PathCmd::Close,
        }
    }
}

/// Emit a closed polygon from unit-space vertices into `out`.
///
/// Returns the command count, or `None` when `out` is too small. Emits
/// `Move` + `Line` per remaining vertex + `Close`, so `vertices.len() + 1`
/// slots are needed. `None` (never a truncated path) when the buffer is too
/// small.
pub fn emit_poly(vertices: &[(f64, f64)], place: &Placement, out: &mut [PathCmd]) -> Option<usize> {
    if vertices.is_empty() {
        return None;
    }
    let needed = vertices.len() + 1;
    if out.len() < needed {
        return None;
    }
    out[0] = PathCmd::Move(place.point(PointF::new(vertices[0].0, vertices[0].1)));
    for (slot, (x, y)) in out[1..vertices.len()].iter_mut().zip(&vertices[1..]) {
        *slot = PathCmd::Line(place.point(PointF::new(*x, *y)));
    }
    out[vertices.len()] = PathCmd::Close;
    Some(needed)
}

/// Unit-space vertices of a five-pointed star inscribed in the unit box, with
/// the given inner-radius ratio. `inner` selects the build; a consumer that
/// needs a different look supplies another ratio, but the built-in glyph keeps
/// its historical `0.382`.
pub fn star_vertices(inner: f64) -> [(f64, f64); 10] {
    let mut vertices = [(0.0, 0.0); 10];
    for (k, slot) in vertices.iter_mut().enumerate() {
        let r = if k % 2 == 0 { 0.5 } else { 0.5 * inner };
        let angle = -std::f64::consts::FRAC_PI_2 + (k as f64) * std::f64::consts::PI / 5.0;
        *slot = (0.5 + r * angle.cos(), 0.5 + r * angle.sin());
    }
    vertices
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    const EPS: f64 = 1e-9;

    fn square() -> [(f64, f64); 4] {
        [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
    }

    #[test]
    fn fit_uses_the_smaller_axis_and_centers() {
        let place = Placement::fit(10.0, 20.0, 40.0, 20.0, 0.0).unwrap();
        assert_eq!(place.scale, 20.0);
        assert_eq!(place.center, PointF::new(30.0, 30.0));
        // Unit box inside a 40x20 target: 20 wide, centred horizontally.
        let tl = place.point(PointF::new(0.0, 0.0));
        let br = place.point(PointF::new(1.0, 1.0));
        assert!((tl.x - 20.0).abs() < EPS && (tl.y - 20.0).abs() < EPS);
        assert!((br.x - 40.0).abs() < EPS && (br.y - 40.0).abs() < EPS);
    }

    #[test]
    fn nonpositive_target_paints_nothing() {
        assert!(Placement::fit(0.0, 0.0, 0.0, 10.0, 0.0).is_none());
        assert!(Placement::fit(0.0, 0.0, 10.0, -1.0, 0.0).is_none());
        assert!(Placement::fit(0.0, 0.0, f64::NAN, 10.0, 0.0).is_none());
    }

    #[test]
    fn radius_is_half_the_smaller_axis_so_a_circle_is_not_stretched() {
        let place = Placement::fit(0.0, 0.0, 100.0, 30.0, 0.0).unwrap();
        // A half-unit radius scales to half the smaller axis, both axes equal.
        assert!((place.length(0.5) - 15.0).abs() < EPS);
    }

    #[test]
    fn stroke_half_thickness_scales_by_the_uniform_factor() {
        let place = Placement::fit(0.0, 0.0, 10.0, 10.0, 0.0).unwrap();
        assert!((place.length(0.11) - 1.1).abs() < EPS);
    }

    #[test]
    fn quarter_turn_maps_the_x_axis_onto_the_y_axis() {
        let place = Placement::fit(0.0, 0.0, 10.0, 10.0, FRAC_PI_2).unwrap();
        // Unit +x from the centre rotates to +y (down) in Y-down space.
        let p = place.point(PointF::new(1.0, 0.5));
        assert!((p.x - 5.0).abs() < EPS, "x {}", p.x);
        assert!((p.y - 10.0).abs() < EPS, "y {}", p.y);
    }

    #[test]
    fn a_placed_polygon_stays_within_its_box() {
        // A 30-wide, 10-tall box: the min-side fit keeps the unit square 10
        // wide, centred, fully inside the box.
        let place = Placement::fit(5.0, 7.0, 30.0, 10.0, 0.0).unwrap();
        let mut out = [PathCmd::Close; 5];
        assert_eq!(emit_poly(&square(), &place, &mut out), Some(5));
        for cmd in &out {
            let p = match cmd {
                PathCmd::Move(p) | PathCmd::Line(p) => *p,
                PathCmd::Close => continue,
                _ => unreachable!(),
            };
            assert!((5.0..=35.0).contains(&p.x) && (7.0..=17.0).contains(&p.y));
        }
    }

    #[test]
    fn rotation_cycles_the_square_corners() {
        let base = Placement::fit(0.0, 0.0, 8.0, 8.0, 0.0).unwrap();
        let turned = Placement::fit(0.0, 0.0, 8.0, 8.0, FRAC_PI_2).unwrap();
        let corners: Vec<PointF> = square()
            .iter()
            .map(|&(x, y)| base.point(PointF::new(x, y)))
            .collect();
        let rotated: Vec<PointF> = square()
            .iter()
            .map(|&(x, y)| turned.point(PointF::new(x, y)))
            .collect();
        // A quarter turn sends corner i to the position of corner (i+1) mod 4.
        for i in 0..4 {
            let got = rotated[i];
            let want = corners[(i + 1) % 4];
            assert!(
                (got.x - want.x).abs() < EPS && (got.y - want.y).abs() < EPS,
                "{i}: {got:?} vs {want:?}"
            );
        }
    }

    #[test]
    fn arc_transform_scales_radius_and_adds_rotation() {
        let place = Placement::fit(0.0, 0.0, 100.0, 100.0, FRAC_PI_2).unwrap();
        let spec = ArcSpec::new(PointF::new(0.5, 0.5), 0.1, 0.0, 1.5);
        let PathCmd::Arc(out) = place.transform(&PathCmd::Arc(spec)) else {
            panic!("expected an arc");
        };
        assert!((out.radius - 10.0).abs() < EPS);
        assert!((out.start_angle - FRAC_PI_2).abs() < EPS);
        assert!((out.sweep_angle - 1.5).abs() < EPS);
        assert!((out.center.x - 50.0).abs() < EPS && (out.center.y - 50.0).abs() < EPS);
    }

    #[test]
    fn emit_poly_refuses_a_small_buffer_instead_of_truncating() {
        let place = Placement::fit(0.0, 0.0, 10.0, 10.0, 0.0).unwrap();
        let mut small = [PathCmd::Close; 4]; // needs 5
        assert_eq!(emit_poly(&square(), &place, &mut small), None);
        let mut exact = [PathCmd::Close; 5];
        assert_eq!(emit_poly(&square(), &place, &mut exact), Some(5));
        assert!(matches!(exact[0], PathCmd::Move(_)));
        assert!(matches!(exact[4], PathCmd::Close));
    }

    #[test]
    fn star_vertices_are_ten_alternating_radii_in_the_unit_box() {
        let v = star_vertices(0.382);
        assert_eq!(v.len(), 10);
        for (x, y) in v {
            assert!((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y));
        }
    }
}
