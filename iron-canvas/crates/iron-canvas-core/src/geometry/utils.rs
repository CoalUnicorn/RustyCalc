//! Compatibility forwarder for the old `geometry::utils` path.
//!
//! The column-label builder lives in [`super::labels`]; this keeps
//! `geometry::utils::col_name` (re-exported by `iron-canvas-web`) working.

pub use super::labels::col_name;
