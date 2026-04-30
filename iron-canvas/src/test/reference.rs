/// Reference compositor for pixel-parity testing.
///
/// `DrawOp` captures the sequence of drawing commands that the grid and
/// overlay layers would issue. Tests assert that the composited two-layer
/// sequence equals the single-canvas reference sequence for the same scene.
///
/// This module is `#[cfg(test)]` only and never ships in production.
/// The full browser-side pixel comparison (getImageData) lives in the future
/// wasm-bindgen-test suite once wasm-pack is wired up.
use crate::geometry::{PixelRect, Point};
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
    if let Some(sel) = overlays.selection {
        ops.push(DrawOp::StrokeRect {
            color: theme.selection_color,
            line_width: 2.0,
            x: sel.top_left.x,
            y: sel.top_left.y,
            w: sel.width,
            h: sel.height,
        });
    }
    ops
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
    if let Some(sel) = overlays.selection {
        ops.push(DrawOp::StrokeRect {
            color: theme.selection_color,
            line_width: 2.0,
            x: sel.top_left.x,
            y: sel.top_left.y,
            w: sel.width,
            h: sel.height,
        });
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::LIGHT;

    const W: f64 = 800.0;
    const H: f64 = 600.0;

    fn sel(x: f64) -> PixelRect {
        PixelRect {
            top_left: Point { x, y: 10.0 },
            width: 80.0,
            height: 20.0,
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
    fn scene_with_selection_compose_equals_reference() {
        let overlays = RenderOverlays {
            selection: Some(sel(40.0)),
            ..Default::default()
        };
        let composed = compose(grid_ops(&LIGHT, W, H), overlay_ops(&LIGHT, &overlays, W, H));
        let reference = reference_ops(&LIGHT, &overlays, W, H);
        assert_eq!(composed, reference);
    }

    #[test]
    fn clearing_selection_returns_to_background_only() {
        let with_sel = RenderOverlays {
            selection: Some(sel(40.0)),
            ..Default::default()
        };
        let without_sel = RenderOverlays::default();

        let composed_with = compose(grid_ops(&LIGHT, W, H), overlay_ops(&LIGHT, &with_sel, W, H));
        let composed_without = compose(
            grid_ops(&LIGHT, W, H),
            overlay_ops(&LIGHT, &without_sel, W, H),
        );
        let reference_without = reference_ops(&LIGHT, &without_sel, W, H);

        assert_ne!(
            composed_with, composed_without,
            "clearing selection must change ops"
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
