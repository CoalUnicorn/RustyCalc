//! Web-side layer re-exports. The Surface trait, `LayerBase`, `PaintGate`,
//! and the layer-specialization paint methods all live in
//! `iron_canvas_core::layer`. This shim keeps the historical
//! `iron_canvas::layer::RenderOverlays` import path stable until Stage 4
//! renames this crate to `iron-canvas-web`.

pub use iron_canvas_core::RenderOverlays;
