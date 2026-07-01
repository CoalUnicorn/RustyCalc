//! `Surface` adapter wrapping `PdfPainter`. Drives `Orchestrator` for
//! one-shot PDF export.
//!
//! Same `Rc<P>` ownership shape as [`crate::svg::SvgSurface`] —
//! `clone_painter` hands the orchestrator's `RendererCore` a clone of
//! the same painter the surface holds, so paint ops accumulate in one
//! place. `resize` and `present` are no-ops: PDF document dimensions
//! are baked into the page `/MediaBox` at construction and there's no
//! backing pixel buffer to flush.

use std::cell::RefCell;
use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::Surface;
use iron_canvas_core::{CanvasModel, CanvasTheme};

use crate::pdf::doc::{ContentStream, PdfDocument};
use crate::pdf::painter::PdfPainter;
use crate::pdf::{doc::font, doc::object, doc::page};

pub struct PdfSurface {
    painter: Rc<PdfPainter>,
}

impl PdfSurface {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            painter: Rc::new(PdfPainter::new(width, height)),
        }
    }

    /// Hand back the painter's content-stream handle so it survives the
    /// orchestrator drop — `render` grabs it before `drive_once`.
    pub fn stream(&self) -> Rc<RefCell<ContentStream>> {
        self.painter.stream()
    }

    /// Drain the painter's content stream and assemble a complete
    /// single-page PDF document. Safe to invoke after the orchestrator's
    /// `Rc<PdfPainter>` clones have been dropped.
    pub fn finish(&self) -> Vec<u8> {
        self.painter.assert_balanced();
        let stream = self.painter.stream();
        let borrowed = stream.borrow();
        Self::build_document(&borrowed, self.painter.width, self.painter.height)
    }

    /// Stream-first assembly: wrap an already-populated `ContentStream`
    /// in the page-open CTM and emit a complete single-page PDF.
    ///
    /// Split from `finish` so `render` can assemble after the throwaway
    /// `Orchestrator` (and its `Rc<PdfPainter>` clones) have dropped:
    /// `render` grabs the grid surface's `stream()` handle up front,
    /// drives the paint, then feeds that surviving stream here. The
    /// overlay surface is never assembled — that is the overlay discard.
    ///
    /// The CTM `1 0 0 -1 0 H cm` flips painter Y-down into PDF Y-up —
    /// emitted exactly once here rather than once per paint call.
    pub fn build_document(stream: &ContentStream, width: u32, height: u32) -> Vec<u8> {
        let mut page_stream = ContentStream::new();
        page_stream.write(format!("1 0 0 -1 0 {height} cm\n").as_bytes());
        page_stream.write(stream.bytes());

        // Object numbering matches Commit 2's smoke test:
        // 1=Catalog, 2=Pages, 3=Page, 4=Contents, 5=Resources.
        let mut doc = PdfDocument::new();
        doc.add_object(1, 0, object::catalog_object(2));
        doc.add_object(2, 0, object::pages_object(3));
        doc.add_object(3, 0, page::page_object(2, 4, 5, width, height));
        doc.add_object(4, 0, page_stream.into_object());
        doc.add_object(5, 0, font::resources_object_with_helvetica());
        doc.finish()
    }

    /// One-shot render of `model` into a single-page PDF document.
    ///
    /// Mirrors [`crate::svg::SvgSurface::render`]'s overlay-discard
    /// policy: both grid and overlay surfaces are driven by a throwaway
    /// `Orchestrator`, but only the grid stream feeds `build_document`,
    /// so the PDF carries no selection, marching ants, autofill handle,
    /// or formula refs.
    pub fn render(model: Rc<dyn CanvasModel>, theme: &CanvasTheme, size: CanvasSize) -> Vec<u8> {
        let width = size.w.round() as u32;
        let height = size.h.round() as u32;

        let grid = PdfSurface::new(width, height);
        let overlay = PdfSurface::new(width, height);
        let grid_stream = grid.stream();

        crate::drive_once(grid, overlay, model, theme, size);

        let stream = grid_stream.borrow();
        Self::build_document(&stream, width, height)
    }
}

impl Surface for PdfSurface {
    type P = PdfPainter;

    fn painter(&self) -> &PdfPainter {
        self.painter.as_ref()
    }

    fn clone_painter(&self) -> Rc<PdfPainter> {
        Rc::clone(&self.painter)
    }

    /// PDF document dimensions are baked at `PdfSurface::new`; a later
    /// `resize` that disagrees would silently produce a mismatched
    /// `/MediaBox`. The assertion mirrors `SvgSurface::resize`.
    fn resize(&mut self, css: CanvasSize, _dpr: f64) {
        debug_assert_eq!(
            (css.w.round() as u32, css.h.round() as u32),
            (self.painter.width, self.painter.height),
            "PdfSurface::resize disagrees with PdfPainter dimensions baked at construction",
        );
    }

    fn present(&self) {}
}
