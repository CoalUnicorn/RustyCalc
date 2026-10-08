//! Mouse event handlers for the worksheet canvas.
//!
//! Most public functions follow the pattern used throughout `src/input/`:
//! pure logic that takes `(model: ModelStore, state: WorkbookState)` and
//! returns `()`. The two resize-begin helpers return `bool` to signal
//! whether a resize was started. The worksheet component holds thin
//! closures that delegate here.
//!
//! - [`mousedown`], [`mousemove`], [`mouseup`], [`wheel`], [`dblclick`],
//!   [`contextmenu`] — one file per DOM event handler.
//! - [`click`] — the four hit-test-resolved click helpers
//!   (`handle_*_click`) called from `mousedown` once a `GridHit` is known.
//! - [`cursor_hint`] — `compute_cursor_hint`, which must mirror
//!   `mousedown`'s hit-test priority exactly (see its module doc) and also
//!   reports the hovered link cell.

use leptos::prelude::*;

use crate::scene::SceneHandle;
use crate::state::{ModelStore, WorkbookState};

mod autofill;
mod click;
mod contextmenu;
mod cursor_hint;
mod dblclick;
mod formula_ref;
pub(crate) mod header_span;
mod mousedown;
mod mousemove;
mod mouseup;
mod wheel;

pub(crate) use autofill::{preview_fill_extension, resolved_fill_target};
pub use contextmenu::handle_contextmenu;
pub(crate) use cursor_hint::{clear_hover, revalidate_hover};
pub use dblclick::handle_dblclick;
pub use mousedown::handle_mousedown;
pub use mousemove::handle_mousemove;
pub use mouseup::handle_mouseup;
pub use wheel::handle_wheel;

/// Storage type for the scene render handle. `LocalStorage` because the
/// handle is `!Send` (holds web_sys handles); `StoredValue` because we don't
/// want event listeners to subscribe to changes — the handle is created once
/// on mount, dropped on unmount.
pub type CanvasHandle = StoredValue<Option<SceneHandle>, LocalStorage>;

/// Read a value from the canvas handle. Returns `None` until both
/// `<canvas>` elements mount and the lazy rAF construction runs.
pub(crate) fn with_canvas<R>(
    handle: CanvasHandle,
    model: ModelStore,
    state: WorkbookState,
    f: impl FnOnce(&SceneHandle) -> R,
) -> Option<R> {
    let (sheet, workbook_id) = model.with_value(|m| {
        (
            m.get_selected_view().sheet,
            state.workbook_generation.get_value(),
        )
    });
    handle.with_value(|slot| {
        slot.as_ref()
            .filter(|scene| scene.matches_context(sheet, workbook_id))
            .map(f)
    })
}
