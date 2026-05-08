//! Canvas domain types - the authoritative type definitions for the canvas module.
//!
//! Types are split by visibility:
//! - `pub(crate)` - renderer-internal: text layout, pane geometry, drawing params
//! - `pub` - worksheet-visible: overlay state passed in from the Leptos component
//!
//! `*Paint` submodules hold renderer-ready snapshots resolved from the model.
//! Convention: resolve in `crate::types`, paint in `crate::renderer`.

pub mod coord;
pub mod ui;
