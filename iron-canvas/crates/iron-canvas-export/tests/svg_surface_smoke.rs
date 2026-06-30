//! Stage-2 smoke test: `SvgSurface` satisfies the `Surface` trait bound
//! so `Orchestrator<SvgSurface>` can be constructed. End-to-end paint
//! verification lives in the browser test (Stage 5); building a stub
//! `CanvasModel` here would be ~200 LOC for marginal extra coverage over
//! the existing `MemSurface`-driven integration tests.

#![cfg(feature = "svg")]

use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{CanvasModel, CanvasTheme, Orchestrator};
use iron_canvas_datagrid::{Column, DataGrid};
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

#[test]
fn svg_render_discards_overlay() {
    // A grid with content and an explicit selection. The selection paints only
    // through the *overlay* surface, which `render` drops.
    let mut grid = DataGrid::builder()
        .column(Column::new("A"))
        .row(vec!["hello".to_string()])
        .build();
    grid.set_selection(1, 1, 3, 3);
    let model: Rc<dyn CanvasModel> = Rc::new(grid);

    let svg = SvgSurface::render(
        model,
        &CanvasTheme::light(),
        CanvasSize { w: 300.0, h: 200.0 },
    );

    assert!(
        svg.starts_with("<svg "),
        "render did not return an svg document"
    );
    assert!(
        svg.contains("<g class=\"cells\">"),
        "grid cell group missing from render output"
    );
    assert!(
        !svg.contains("class=\"overlay\""),
        "overlay group leaked into the grid-only SVG export"
    );
}
