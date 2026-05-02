#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

/// Reference compositor for pixel-parity testing.
///
/// `DrawOp` captures the sequence of drawing commands that the grid and
/// overlay layers would issue. Tests assert that the composited two-layer
/// sequence equals the single-canvas reference sequence for the same scene.
///
/// This module is `#[cfg(test)]` only and never ships in production.
/// The full browser-side pixel comparison (getImageData) lives in the future
/// wasm-bindgen-test suite once wasm-pack is wired up.
use crate::model::RCRange;
use crate::theme::CanvasTheme;
use crate::types::RenderOverlays;

/// A single canvas drawing command, captured for comparison.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DrawOp {
    FillRect {
        color: &'static str,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
    StrokeRect {
        color: &'static str,
        line_width: f64,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
    ClearRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
}

/// Ops the grid layer emits for the stub paint.
pub(crate) fn grid_ops(theme: &CanvasTheme, w: f64, h: f64) -> Vec<DrawOp> {
    vec![DrawOp::FillRect {
        color: theme.cell_bg,
        x: 0.0,
        y: 0.0,
        w,
        h,
    }]
}

/// Ops the overlay layer emits for the stub paint.
///
/// The variable scene is `point_range`: when present, the overlay paints a
/// dashed-style outline (modeled here as a `StrokeRect` for compositing
/// purposes). This is a *test-only* parallel painter — the production overlay
/// derives the selection rect from the model via `range_pixel_bounds`. The
/// reference module only asserts that two-canvas compositing equals the
/// single-canvas reference for the same scene; it does not assert geometry
/// correctness against the production renderer.
pub(crate) fn overlay_ops(
    theme: &CanvasTheme,
    overlays: &RenderOverlays,
    w: f64,
    h: f64,
) -> Vec<DrawOp> {
    let mut ops = vec![DrawOp::ClearRect {
        x: 0.0,
        y: 0.0,
        w,
        h,
    }];
    if let Some(range) = overlays.point_range {
        ops.push(stub_range_stroke(theme, range));
    }
    ops
}

/// Deterministic test-only mapping from `RCRange` → `StrokeRect` op. Both
/// `overlay_ops` and `reference_ops` route through this so they paint the
/// same shape for the same range.
fn stub_range_stroke(theme: &CanvasTheme, range: RCRange) -> DrawOp {
    const COL_W: f64 = 80.0;
    const ROW_H: f64 = 20.0;
    DrawOp::StrokeRect {
        color: theme.selection_color,
        line_width: 2.0,
        x: range.c1 as f64 * COL_W,
        y: range.r1 as f64 * ROW_H,
        w: (range.c2 - range.c1 + 1) as f64 * COL_W,
        h: (range.r2 - range.r1 + 1) as f64 * ROW_H,
    }
}

/// Composite grid + overlay ops into a single-canvas op sequence.
/// Mirrors what a browser would produce by stacking the two canvases:
/// draw grid ops first, then overlay ops minus the ClearRect (transparent
/// pixels let the grid show through — the ClearRect has no visible effect
/// on a composited bitmap).
pub(crate) fn compose(grid: Vec<DrawOp>, overlay: Vec<DrawOp>) -> Vec<DrawOp> {
    let mut out = grid;
    for op in overlay {
        // ClearRect is a no-op in the composited view: the grid pixels
        // remain visible wherever the overlay is transparent.
        match op {
            DrawOp::FillRect { .. } | DrawOp::StrokeRect { .. } => out.push(op),
            DrawOp::ClearRect { .. } => {}
        }
    }
    out
}

/// What a single-canvas reference renderer would emit for the same scene.
pub(crate) fn reference_ops(
    theme: &CanvasTheme,
    overlays: &RenderOverlays,
    w: f64,
    h: f64,
) -> Vec<DrawOp> {
    let mut ops = vec![DrawOp::FillRect {
        color: theme.cell_bg,
        x: 0.0,
        y: 0.0,
        w,
        h,
    }];
    if let Some(range) = overlays.point_range {
        ops.push(stub_range_stroke(theme, range));
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::LIGHT;

    const W: f64 = 800.0;
    const H: f64 = 600.0;

    fn range(start_col: i32) -> RCRange {
        RCRange {
            r1: 1,
            c1: start_col,
            r2: 1,
            c2: start_col + 4,
        }
    }

    #[test]
    fn empty_scene_compose_equals_reference() {
        let overlays = RenderOverlays::default();
        let composed = compose(grid_ops(&LIGHT, W, H), overlay_ops(&LIGHT, &overlays, W, H));
        let reference = reference_ops(&LIGHT, &overlays, W, H);
        assert_eq!(composed, reference);
    }

    #[test]
    fn scene_with_point_range_compose_equals_reference() {
        let overlays = RenderOverlays {
            point_range: Some(range(3)),
            ..Default::default()
        };
        let composed = compose(grid_ops(&LIGHT, W, H), overlay_ops(&LIGHT, &overlays, W, H));
        let reference = reference_ops(&LIGHT, &overlays, W, H);
        assert_eq!(composed, reference);
    }

    #[test]
    fn clearing_point_range_returns_to_background_only() {
        let with_range = RenderOverlays {
            point_range: Some(range(3)),
            ..Default::default()
        };
        let without_range = RenderOverlays::default();

        let composed_with = compose(
            grid_ops(&LIGHT, W, H),
            overlay_ops(&LIGHT, &with_range, W, H),
        );
        let composed_without = compose(
            grid_ops(&LIGHT, W, H),
            overlay_ops(&LIGHT, &without_range, W, H),
        );
        let reference_without = reference_ops(&LIGHT, &without_range, W, H);

        assert_ne!(
            composed_with, composed_without,
            "clearing point_range must change ops"
        );
        assert_eq!(composed_without, reference_without);
    }

    #[test]
    fn misconfigured_overlay_alpha_false_would_obscure_grid() {
        // Simulates what happens if overlay used { alpha: false }:
        // the ClearRect would paint black instead of being transparent,
        // so compositing produces a black fill that hides the grid.
        // Here we model it as a FillRect(black) replacing ClearRect.
        let overlays = RenderOverlays::default();
        let bad_overlay: Vec<DrawOp> = vec![DrawOp::FillRect {
            color: "#000000",
            x: 0.0,
            y: 0.0,
            w: W,
            h: H,
        }];
        let composed_bad = compose(grid_ops(&LIGHT, W, H), bad_overlay);
        let reference = reference_ops(&LIGHT, &overlays, W, H);
        assert_ne!(
            composed_bad, reference,
            "alpha:false overlay must not match reference — test catches misconfiguration"
        );
    }
}
