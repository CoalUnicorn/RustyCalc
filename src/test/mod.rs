//! Centralized test modules. Single audit point for "what is covered?".
//!
//! Stage 8 of the front-end modularity refactor relocated previously-inline
//! `#[cfg(test)] mod tests { … }` blocks to here. Source files keep only
//! their production code; the corresponding test file in this directory
//! re-imports the items under test via `crate::…` paths.

mod clipboard_bridge;
mod color_picker;
mod coord;
mod formula_analysis;
mod formula_input;
mod formula_overlay;
mod keyboard;
mod model_frontend;
mod model_frontend_types;
mod model_style;
mod mouse;
mod state;
mod toolbar_section;
