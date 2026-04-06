//! Typed errors for the input action pipeline.
//!
//! Each category module returns its own error type so the single log point in
//! `action::execute` carries domain context. All variants currently wrap the
//! opaque `String` that ironcalc returns; add discriminated variants here as
//! upstream gains structured errors or as this codebase adds its own validation.

use thiserror::Error;

/// Error from a formatting mutation (bold, color, font size, etc.).
#[derive(Debug, Error)]
pub enum FormatError {
    #[error("format: {0}")]
    Engine(String),
}

/// Error from a structural mutation (delete, insert rows/cols, undo/redo).
#[derive(Debug, Error)]
pub enum StructError {
    #[error("structure: {0}")]
    Engine(String),
}

/// Error from a navigation mutation (sheet switch).
#[derive(Debug, Error)]
pub enum NavError {
    #[error("navigation: {0}")]
    Engine(String),
}

/// Error from committing a cell edit to the model.
#[derive(Debug, Error)]
pub enum EditError {
    #[error("edit: {0}")]
    Engine(String),
}

// ironcalc returns `Result<(), String>` - `From<String>` lets `?` convert directly.
impl From<String> for FormatError {
    fn from(s: String) -> Self {
        Self::Engine(s)
    }
}
impl From<String> for StructError {
    fn from(s: String) -> Self {
        Self::Engine(s)
    }
}
impl From<String> for NavError {
    fn from(s: String) -> Self {
        Self::Engine(s)
    }
}
impl From<String> for EditError {
    fn from(s: String) -> Self {
        Self::Engine(s)
    }
}
