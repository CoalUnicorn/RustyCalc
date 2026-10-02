//! Float-precision path geometry for shape rendering.
//!
//! Grid geometry stays integer ([`Point`](super::prim::Point), `PixelRect`,
//! `Line`, `Span`). Shape paths use this module's [`PointF`] and [`PathCmd`],
//! so a curve reaches the backend as a curve instead of a rounded polygon.
//! [`Path`] borrows a command slice: static shape definitions borrow static
//! slices, and a capture owns a `Vec<PathCmd>`.

use serde::{Deserialize, Serialize};
use std::f64::consts::{FRAC_PI_2, TAU};

/// A point in logical pixels (Y-down) with float precision.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PointF {
    pub x: f64,
    pub y: f64,
}

impl PointF {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Both coordinates are finite.
    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// One circular arc: centre, radius, start angle, and signed sweep.
///
/// Angles are radians. Zero points along +x. A positive sweep turns clockwise
/// in the Y-down coordinate space. The sweep interval is `[-TAU, TAU]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ArcSpec {
    pub center: PointF,
    pub radius: f64,
    pub start_angle: f64,
    pub sweep_angle: f64,
}

impl ArcSpec {
    pub const fn new(center: PointF, radius: f64, start_angle: f64, sweep_angle: f64) -> Self {
        Self {
            center,
            radius,
            start_angle,
            sweep_angle,
        }
    }

    /// All fields are finite, radius is positive, and the sweep is at most a
    /// full turn.
    pub fn is_valid(&self) -> bool {
        self.center.is_finite()
            && self.radius.is_finite()
            && self.radius > 0.0
            && self.start_angle.is_finite()
            && self.sweep_angle.is_finite()
            && self.sweep_angle.abs() <= TAU
    }

    /// Point on the circle at `self.start_angle`.
    pub fn start_point(&self) -> PointF {
        self.point_at(self.start_angle)
    }

    /// Point on the circle at `self.start_angle + self.sweep_angle`.
    pub fn end_point(&self) -> PointF {
        self.point_at(self.start_angle + self.sweep_angle)
    }

    /// Point on the circle at `angle` radians.
    pub fn point_at(&self, angle: f64) -> PointF {
        PointF {
            x: self.center.x + self.radius * angle.cos(),
            y: self.center.y + self.radius * angle.sin(),
        }
    }
}

/// One drawing command in a float path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PathCmd {
    /// Start a new subpath.
    Move(PointF),
    /// Line to the point.
    Line(PointF),
    /// Quadratic curve: control point, end point.
    Quad(PointF, PointF),
    /// Cubic curve: two control points, end point.
    Cubic(PointF, PointF, PointF),
    /// Circular arc.
    Arc(ArcSpec),
    /// Close the current subpath to its start point.
    Close,
}

/// Why a path failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// A coordinate, radius, or angle is not finite.
    NonFinite,
    /// A segment has no current subpath.
    NoSubpath,
    /// The arc radius is not positive or the sweep is larger than a full turn.
    InvalidArc,
}

/// A borrowed float path.
///
/// Fill uses the nonzero winding rule. Fill implicitly closes each open
/// subpath; stroke closes only subpaths with [`PathCmd::Close`]. An empty path
/// and a subpath with only a `Move` paint nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Path<'a> {
    cmds: &'a [PathCmd],
}

impl<'a> Path<'a> {
    pub const fn new(cmds: &'a [PathCmd]) -> Self {
        Self { cmds }
    }

    /// An empty path. Paints nothing.
    pub const EMPTY: Path<'static> = Path { cmds: &[] };

    pub fn cmds(&self) -> &'a [PathCmd] {
        self.cmds
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// Validate the complete path before a backend changes state or draws.
    ///
    /// Checks: every coordinate, radius, and angle is finite; every segment
    /// has a current subpath; and each arc has a positive radius and a sweep
    /// of at most a full turn.
    pub fn validate(&self) -> Result<(), PathError> {
        let mut cur: Option<PointF> = None;
        let mut subpath_start: Option<PointF> = None;
        for cmd in self.cmds {
            match cmd {
                PathCmd::Move(p) => {
                    if !p.is_finite() {
                        return Err(PathError::NonFinite);
                    }
                    cur = Some(*p);
                    subpath_start = Some(*p);
                }
                PathCmd::Line(p) => {
                    if !p.is_finite() {
                        return Err(PathError::NonFinite);
                    }
                    if cur.is_none() {
                        return Err(PathError::NoSubpath);
                    }
                    cur = Some(*p);
                }
                PathCmd::Quad(c, e) => {
                    if !c.is_finite() || !e.is_finite() {
                        return Err(PathError::NonFinite);
                    }
                    if cur.is_none() {
                        return Err(PathError::NoSubpath);
                    }
                    cur = Some(*e);
                }
                PathCmd::Cubic(a, b, e) => {
                    if !a.is_finite() || !b.is_finite() || !e.is_finite() {
                        return Err(PathError::NonFinite);
                    }
                    if cur.is_none() {
                        return Err(PathError::NoSubpath);
                    }
                    cur = Some(*e);
                }
                PathCmd::Arc(spec) => {
                    if !spec.is_valid() {
                        return Err(PathError::InvalidArc);
                    }
                    // A zero sweep has no effect on the path or current point.
                    if spec.sweep_angle == 0.0 {
                        continue;
                    }
                    if cur.is_none() {
                        subpath_start = Some(spec.start_point());
                    }
                    cur = Some(spec.end_point());
                }
                PathCmd::Close => {
                    let Some(start) = subpath_start else {
                        return Err(PathError::NoSubpath);
                    };
                    // The subpath start becomes the current point; a later
                    // segment can continue from it.
                    cur = Some(start);
                }
            }
        }
        Ok(())
    }

    /// Convenience form of [`Self::validate`].
    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }
}

/// Convert a quadratic curve to the two cubic control points.
///
/// `P0` is the current point, `q` the quadratic control point, and `p1` the
/// endpoint. Both backends without native quadratics (PDF, Cairo) share this.
pub fn quad_to_cubic(p0: PointF, q: PointF, p1: PointF) -> (PointF, PointF) {
    let c1 = PointF::new(
        p0.x + 2.0 / 3.0 * (q.x - p0.x),
        p0.y + 2.0 / 3.0 * (q.y - p0.y),
    );
    let c2 = PointF::new(
        p1.x + 2.0 / 3.0 * (q.x - p1.x),
        p1.y + 2.0 / 3.0 * (q.y - p1.y),
    );
    (c1, c2)
}

/// The largest sweep, in radians, that one cubic approximation covers.
pub const MAX_ARC_SEGMENT: f64 = FRAC_PI_2;

/// Split an arc into cubic Bézier segments and emit `(control1, control2,
/// end)` for each.
///
/// Each emitted segment covers at most a quarter turn, through the tangent
/// factor `4/3 * tan(theta/4)`. The sign of the sweep is preserved and the
/// arc endpoints stay exact. The caller must have moved to
/// [`ArcSpec::start_point`] already; this function does not emit the
/// connecting line.
pub fn arc_to_cubics(spec: ArcSpec, mut emit: impl FnMut(PointF, PointF, PointF)) {
    if spec.sweep_angle == 0.0 {
        return;
    }
    let segments = (spec.sweep_angle.abs() / MAX_ARC_SEGMENT).ceil() as usize;
    let segments = segments.max(1);
    let theta = spec.sweep_angle / segments as f64;
    // Dimensionless tangent offset: `4/3 * tan(theta/4)`. Negative for a
    // negative sweep, which flips each control point onto the correct side.
    let k = 4.0 / 3.0 * (theta / 4.0).tan();

    for i in 0..segments {
        let a0 = spec.start_angle + theta * i as f64;
        let a1 = a0 + theta;
        let p0 = spec.point_at(a0);
        let p1 = spec.point_at(a1);
        let c1 = PointF::new(
            p0.x + k * spec.radius * -a0.sin(),
            p0.y + k * spec.radius * a0.cos(),
        );
        let c2 = PointF::new(
            p1.x - k * spec.radius * -a1.sin(),
            p1.y - k * spec.radius * a1.cos(),
        );
        emit(c1, c2, p1);
    }
}

/// Split an arc into clockwise-or-counter-clockwise segments of at most
/// `max_sweep` radians and emit `(segment_start_angle, segment_sweep)`.
///
/// SVG uses this to split a full turn, which one arc command cannot express.
pub fn arc_segments(spec: ArcSpec, max_sweep: f64, mut emit: impl FnMut(f64, f64)) {
    if spec.sweep_angle == 0.0 || max_sweep <= 0.0 {
        return;
    }
    let segments = (spec.sweep_angle.abs() / max_sweep).ceil() as usize;
    let segments = segments.max(1);
    let theta = spec.sweep_angle / segments as f64;
    for i in 0..segments {
        emit(spec.start_angle + theta * i as f64, theta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn circle_quarter() -> ArcSpec {
        ArcSpec::new(PointF::new(0.0, 0.0), 10.0, 0.0, FRAC_PI_2)
    }

    #[test]
    fn valid_polygon_path_passes() {
        let cmds = [
            PathCmd::Move(PointF::new(0.0, 0.0)),
            PathCmd::Line(PointF::new(10.0, 0.0)),
            PathCmd::Line(PointF::new(5.0, 8.0)),
            PathCmd::Close,
        ];
        assert_eq!(Path::new(&cmds).validate(), Ok(()));
    }

    #[test]
    fn segment_before_move_is_rejected() {
        let cmds = [PathCmd::Line(PointF::new(1.0, 1.0))];
        assert_eq!(Path::new(&cmds).validate(), Err(PathError::NoSubpath));
    }

    #[test]
    fn non_finite_coordinate_is_rejected() {
        let cmds = [
            PathCmd::Move(PointF::new(0.0, 0.0)),
            PathCmd::Line(PointF::new(f64::NAN, 1.0)),
        ];
        assert_eq!(Path::new(&cmds).validate(), Err(PathError::NonFinite));
    }

    #[test]
    fn invalid_arc_radius_is_rejected() {
        let cmds = [PathCmd::Arc(ArcSpec::new(
            PointF::new(0.0, 0.0),
            0.0,
            0.0,
            1.0,
        ))];
        assert_eq!(Path::new(&cmds).validate(), Err(PathError::InvalidArc));
    }

    #[test]
    fn oversize_sweep_is_rejected() {
        let cmds = [PathCmd::Arc(ArcSpec::new(
            PointF::new(0.0, 0.0),
            1.0,
            0.0,
            TAU + 0.1,
        ))];
        assert_eq!(Path::new(&cmds).validate(), Err(PathError::InvalidArc));
    }

    #[test]
    fn arc_starts_a_subpath_and_zero_sweep_does_not() {
        let cmds = [PathCmd::Arc(circle_quarter())];
        assert_eq!(Path::new(&cmds).validate(), Ok(()));

        let zero = [PathCmd::Arc(ArcSpec::new(
            PointF::new(0.0, 0.0),
            1.0,
            0.0,
            0.0,
        ))];
        assert_eq!(Path::new(&zero).validate(), Ok(()));

        // A zero-sweep arc must not start a subpath for a later segment.
        let trailing = [
            PathCmd::Arc(ArcSpec::new(PointF::new(0.0, 0.0), 1.0, 0.0, 0.0)),
            PathCmd::Line(PointF::new(1.0, 1.0)),
        ];
        assert_eq!(Path::new(&trailing).validate(), Err(PathError::NoSubpath));
    }

    #[test]
    fn arc_endpoints_are_exact() {
        let spec = circle_quarter();
        let start = spec.start_point();
        let end = spec.end_point();
        assert!((start.x - 10.0).abs() < EPS && start.y.abs() < EPS);
        assert!(end.x.abs() < EPS && (end.y - 10.0).abs() < EPS);
    }

    #[test]
    fn quad_to_cubic_matches_known_controls() {
        let p0 = PointF::new(0.0, 0.0);
        let q = PointF::new(0.0, 10.0);
        let p1 = PointF::new(10.0, 10.0);
        let (c1, c2) = quad_to_cubic(p0, q, p1);
        assert!((c1.x - 0.0).abs() < EPS && (c1.y - 20.0 / 3.0).abs() < EPS);
        // c2 = p1 + 2/3 * (q - p1) = (10, 10) + 2/3 * (-10, 0) = (10/3, 10).
        assert!((c2.x - 10.0 / 3.0).abs() < EPS && (c2.y - 10.0).abs() < EPS);
    }

    #[test]
    fn quarter_arc_is_one_cubic_with_kappa_controls() {
        let mut segs = Vec::new();
        arc_to_cubics(circle_quarter(), |c1, c2, end| segs.push((c1, c2, end)));
        assert_eq!(segs.len(), 1);
        let (c1, _, end) = segs[0];
        const KAPPA: f64 = 0.552_284_749_8;
        assert!((c1.x - 10.0).abs() < 1e-6);
        assert!((c1.y - KAPPA * 10.0).abs() < 1e-6);
        assert!(end.x.abs() < 1e-6 && (end.y - 10.0).abs() < 1e-6);
    }

    #[test]
    fn full_turn_splits_into_four_cubics_ending_at_start() {
        let spec = ArcSpec::new(PointF::new(3.0, 4.0), 2.0, 0.5, TAU);
        let mut segs = Vec::new();
        arc_to_cubics(spec, |c1, c2, end| segs.push((c1, c2, end)));
        assert_eq!(segs.len(), 4);
        let end = segs.last().unwrap().2;
        let start = spec.start_point();
        assert!((end.x - start.x).abs() < 1e-6 && (end.y - start.y).abs() < 1e-6);
    }

    #[test]
    fn negative_sweep_preserves_direction() {
        let spec = ArcSpec::new(PointF::new(0.0, 0.0), 5.0, 0.0, -FRAC_PI_2);
        let mut segs = Vec::new();
        arc_to_cubics(spec, |c1, c2, end| segs.push((c1, c2, end)));
        assert_eq!(segs.len(), 1);
        let end = segs[0].2;
        // -90 degrees lands on (0, -5) in Y-down space.
        assert!(end.x.abs() < 1e-6 && (end.y + 5.0).abs() < 1e-6);
    }

    #[test]
    fn full_turn_splits_into_two_svg_segments() {
        let spec = ArcSpec::new(PointF::new(0.0, 0.0), 1.0, 0.0, TAU);
        let mut segs = Vec::new();
        arc_segments(spec, std::f64::consts::PI, |start, sweep| {
            segs.push((start, sweep))
        });
        assert_eq!(segs.len(), 2);
        assert!((segs[0].1 - std::f64::consts::PI).abs() < EPS);
        assert!((segs[1].0 - std::f64::consts::PI).abs() < EPS);
    }
}
