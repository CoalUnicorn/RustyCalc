//! Idle-hover cursor style hint, derived from the hit-test under the pointer.

/// Cursor style hint derived from the idle hover position. Drives the
/// `class` on `.ws-grid` so the cursor previews the action a mousedown
/// here would start (resize, autofill, ref-drag, ...). Drag state wins
/// over this — the view composes both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorHint {
    #[default]
    Cell,
    ColResize,
    RowResize,
    Autofill,
    RefMove,
    RefExtendNS,
    RefExtendEW,
    RefCornerNwse,
    RefCornerNesw,
}

impl CursorHint {
    /// Extra class to append to `.ws-canvas.ws-grid`; empty string for
    /// the default cursor which is already set by `.ws-canvas`.
    pub fn class(self) -> &'static str {
        match self {
            CursorHint::Cell => "",
            CursorHint::ColResize => "resize-col",
            CursorHint::RowResize => "resize-row",
            CursorHint::Autofill => "cur-autofill",
            CursorHint::RefMove => "cur-ref-move",
            CursorHint::RefExtendNS => "cur-ref-ns",
            CursorHint::RefExtendEW => "cur-ref-ew",
            CursorHint::RefCornerNwse => "cur-ref-nwse",
            CursorHint::RefCornerNesw => "cur-ref-nesw",
        }
    }
}
