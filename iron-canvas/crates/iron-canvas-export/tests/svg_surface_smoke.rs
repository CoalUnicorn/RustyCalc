//! Stage-2 smoke test: `SvgSurface` satisfies the `Surface` trait bound
//! so `Orchestrator<SvgSurface>` can be constructed. End-to-end paint
//! verification lives in the browser test (Stage 5); building a stub
//! `CanvasModel` here would be ~200 LOC for marginal extra coverage over
//! the existing `MemSurface`-driven integration tests.

#![cfg(feature = "svg")]

use iron_canvas_core::Orchestrator;
use iron_canvas_export::SvgSurface;

#[test]
fn orchestrator_accepts_svg_surface() {
    let grid = SvgSurface::new(100, 50);
    let overlay = SvgSurface::new(100, 50);
    // The whole point of Stage 2: this type resolves.
    let _orch: Orchestrator<SvgSurface> = Orchestrator::new(grid, overlay);
}

#[test]
fn empty_surface_finishes_to_bare_svg() {
    let surface = SvgSurface::new(100, 50);
    let svg = surface.finish();
    assert!(svg.starts_with("<svg "));
    assert!(svg.ends_with("</svg>"));
    assert!(svg.contains("viewBox=\"0 0 100 50\""));
}
