//! Right-click context-menu position and target.

/// Right-clicked header identity and the count of selected headers in that axis.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HeaderContextMenu {
    Column { col: i32, count: i32 },
    Row { row: i32, count: i32 },
}

#[derive(Clone, Copy)]
pub struct ContextMenuState {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) target: HeaderContextMenu,
}
