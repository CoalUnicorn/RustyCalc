//! Hand-rolled PDF backend: writer (`doc`) + `Painter`/`Surface`
//! adapters (`painter`, `surface`).
//!
//! `PdfPainter` translates `Painter`/`BlitPainter`/`TextMetrics` calls
//! into PDF content-stream ops and accumulates them in a shared
//! `Rc<RefCell<ContentStream>>`. `PdfSurface` owns that stream and
//! assembles a complete single-page document via `finish()`.

pub mod doc;
pub mod painter;
pub mod surface;

pub use doc::PdfDocument;
pub use painter::PdfPainter;
pub use surface::PdfSurface;
