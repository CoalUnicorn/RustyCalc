//! IronCalc-free Canvas2D surface for the iron-canvas grid renderer.
//!
//! Hosts the Canvas2D `Painter` impl (`CanvasPainter`), the `WebSurface`
//! adapter, and the CSS-var theme bridge (`theme_from_element`). Every
//! component here touches `web-sys`, `wasm-bindgen`, or `js-sys`, but none
//! depends on `ironcalc_base` — so this crate is reusable by a pure
//! data-grid facade with no spreadsheet semantics.

mod canvas_painter;
pub mod theme_from_element;
mod web_surface;

pub use canvas_painter::CanvasPainter;
pub use web_surface::WebSurface;
