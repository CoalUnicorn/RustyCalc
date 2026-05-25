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

    /// Construct a surface whose painter writes into an externally
    /// owned `ContentStream`. The Commit 4 web facade uses this to
    /// point both the grid and overlay `PdfSurface`s at one buffer.
    pub fn with_stream(stream: Rc<RefCell<ContentStream>>, width: u32, height: u32) -> Self {
        Self {
            painter: Rc::new(PdfPainter::with_stream(stream, width, height)),
        }
    }

    /// Hand back the shared stream so the matching overlay surface can
    /// share the same buffer.
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
    /// Exposed so callers that don't hold the surface itself can still
    /// assemble — `IronCanvas::exportPdf` (in `iron-canvas-web`) hands
    /// both grid and overlay `PdfSurface`s into the throwaway
    /// `Orchestrator`, which takes ownership; the only handle that
    /// survives the orchestrator drop is the shared
    /// `Rc<RefCell<ContentStream>>`.
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
    fn resize(&mut self, css: CanvasSize, _dpr: i32) {
        debug_assert_eq!(
            (css.w.round() as u32, css.h.round() as u32),
            (self.painter.width, self.painter.height),
            "PdfSurface::resize disagrees with PdfPainter dimensions baked at construction",
        );
    }

    fn present(&self) {}
}
