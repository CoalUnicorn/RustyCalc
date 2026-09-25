//! Compatibility forwarder for the old `layer` path.
//!
//! The module lives at [`crate::surface`] now; this keeps existing
//! `iron_canvas_core::layer::{Surface, LayerBase}` imports working.

pub use crate::surface::{LayerBase, Surface};
