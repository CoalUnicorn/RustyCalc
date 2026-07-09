//! Persistent user-visible status message (errors shown until dismissed).

/// A user-visible message set by the input pipeline when an engine operation fails.
///
/// Stored on [`super::workbook_state::WorkbookState`] rather than the
/// EventBus — errors are persistent UI state (shown until dismissed), not
/// fire-and-forget domain events.
#[derive(Clone, Debug, PartialEq)]
pub enum StatusMessage {
    Error(String),
}
