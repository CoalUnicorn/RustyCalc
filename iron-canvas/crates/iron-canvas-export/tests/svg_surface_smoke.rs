//! Stage-2 smoke test: `SvgSurface` satisfies the `Surface` trait bound
//! so `Orchestrator<SvgSurface>` can be constructed. End-to-end paint
//! verification lives in the browser test (Stage 5); building a stub
//! `CanvasModel` here would be ~200 LOC for marginal extra coverage over
//! the existing `MemSurface`-driven integration tests.

#![cfg(feature = "svg")]

use std::rc::Rc;

use iron_canvas_core::forward_methods;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{
    CanvasModel, CanvasTheme, CanvasView, CellContentQuery, CellDecoration, CellKind, CellLink,
    CellStyle, Fetched, LinkTarget, Orchestrator, RCRange,
};
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
    )
    .expect("a one-shot export of a readable model commits a frame");

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

/// A `DataGrid` plus committed link state, so an export render exercises the
/// link text rule through the shared painter path.
struct LinkModel {
    grid: DataGrid,
    links: Vec<CellLink>,
}

impl LinkModel {
    fn grid(&self) -> &DataGrid {
        &self.grid
    }
}

impl CellContentQuery for LinkModel {
    forward_methods!(grid, {
        fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellStyle>;
        fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellKind>;
        fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Fetched<String>;
        fn get_extended_cell_style(
            &self,
            sheet: u32,
            row: i32,
            column: i32,
        ) -> Fetched<CellDecoration>;
        fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellStyle>>);
        fn get_formatted_cell_values_in(
            &self,
            sheet: u32,
            range: RCRange,
            out: &mut Vec<Fetched<String>>,
        );
        fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellKind>>);
        fn get_cell_decorations_in(
            &self,
            sheet: u32,
            range: RCRange,
            out: &mut Vec<Fetched<CellDecoration>>,
        );
    });
}

impl CanvasModel for LinkModel {
    forward_methods!(grid, {
        fn get_selected_sheet(&self) -> Option<u32>;
        fn get_selected_view(&self) -> Option<CanvasView>;
        fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32>;
        fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32>;
        fn get_row_height(&self, sheet: u32, row: i32) -> Fetched<f64>;
        fn get_column_width(&self, sheet: u32, column: i32) -> Fetched<f64>;
        fn get_show_grid_lines(&self, sheet: u32) -> Fetched<bool>;
        fn get_show_selection(&self) -> bool;
        fn last_row(&self, sheet: u32) -> i32;
        fn last_column(&self, sheet: u32) -> i32;
        fn get_show_row_headers(&self, sheet: u32) -> Option<bool>;
        fn get_show_col_headers(&self, sheet: u32) -> Option<bool>;
        fn get_row_header_text(&self, sheet: u32, row: i32) -> Option<String>;
        fn get_column_header_text(&self, sheet: u32, col: i32) -> Option<String>;
    });

    fn get_sheet_links(&self, _sheet: u32) -> Option<Vec<CellLink>> {
        Some(self.links.clone())
    }
}

/// Acceptance 5 (visual half): a link's resolved color and underline must
/// reach the SVG. PDF replays the same `Painter::fill_text` and
/// `stroke_text_hline` calls, so the shared rule covers both; the PDF
/// primitives themselves are pinned in `pdf_painter_smoke`.
#[test]
fn svg_export_passes_link_color_and_underline_to_the_painter() {
    let grid = DataGrid::builder()
        .column(Column::new("A"))
        .row(vec!["link".to_string()])
        .build();
    let model = LinkModel {
        grid,
        links: vec![CellLink::new(
            RCRange::from_cell(1, 1),
            LinkTarget::External("https://example.com".to_string()),
            None,
            true,
            Some("#0563c1".to_string()),
        )],
    };

    let svg = SvgSurface::render(
        Rc::new(model),
        &CanvasTheme::light(),
        CanvasSize { w: 300.0, h: 200.0 },
    )
    .expect("a one-shot export of a readable model commits a frame");

    assert!(
        svg.contains("fill=\"#0563c1\""),
        "the link's resolved color must reach the SVG text: {svg}"
    );
    assert!(
        svg.contains("stroke=\"#0563c1\""),
        "the link underline must reach the SVG: {svg}"
    );
}
