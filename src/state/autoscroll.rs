//! Edge-scroll JS interval handle plus direction/position state.

use leptos::prelude::*;

/// Non-reactive edge-scroll state: JS interval handle, scroll direction, last
/// mouse position. `StoredValue` (not `Split`) so start/stop never triggers
/// a reactive re-render.
#[derive(Clone, Copy)]
pub struct AutoscrollState {
    pub(crate) id: StoredValue<Option<i32>>,
    pub(crate) dir: StoredValue<(i32, i32)>,
    pub(crate) pos: StoredValue<(f64, f64)>,
}

impl AutoscrollState {
    pub(super) fn new() -> Self {
        Self {
            id: StoredValue::new(None),
            dir: StoredValue::new((0, 0)),
            pos: StoredValue::new((0.0, 0.0)),
        }
    }

    pub(crate) fn cancel(&self) {
        if let Some(id) = self.id.get_value() {
            leptos::prelude::window().clear_interval_with_handle(id);
            self.id.set_value(None);
        }
        self.dir.set_value((0, 0));
    }
}
