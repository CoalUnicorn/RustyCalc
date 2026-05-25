//! Hand-rolled PDF writer + (in Commit 3) `PdfPainter` + `PdfSurface`.
//!
//! Commit 2 ships only the document writer (`doc::PdfDocument`) — enough
//! to assemble a valid single-page PDF by hand. The painter that emits
//! content-stream ops on top of it lives one commit further down.

pub mod doc;

pub use doc::PdfDocument;
