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
//!   (`handle_*_click`) called from `mousedown` once a `HitTest` is known.
//! - [`cursor_hint`] — `compute_cursor_hint`, which must mirror
//!   `mousedown`'s hit-test priority exactly (see `// Why:` in that file).
//! - [`formula_ref`] — the formula-reference drag sub-grammar
//!   (handle_formula_ref_mousedown, dragged_ref_range, commit_formula_ref_drag).

use iron_canvas_web::IronCanvas;
use leptos::prelude::*;

mod click;
mod contextmenu;
mod cursor_hint;
mod dblclick;
pub(crate) mod formula_ref;
mod mousedown;
mod mousemove;
mod mouseup;
mod wheel;

pub use contextmenu::handle_contextmenu;
pub use dblclick::handle_dblclick;
pub use mousedown::handle_mousedown;
pub use mousemove::handle_mousemove;
pub use mouseup::handle_mouseup;
pub use wheel::handle_wheel;

/// Storage type for the IronCanvas orchestrator handle. `LocalStorage`
/// because `IronCanvas` is `!Send` (holds web_sys handles); `StoredValue`
/// because we don't want event listeners to subscribe to changes — the
/// handle is created once on mount, dropped on unmount.
pub type CanvasHandle = StoredValue<Option<IronCanvas>, LocalStorage>;

/// Read a value from the canvas handle. Returns `None` until both
/// `<canvas>` elements mount and the lazy rAF construction runs.
pub(super) fn with_canvas<R>(
    handle: CanvasHandle,
    f: impl FnOnce(&IronCanvas) -> R,
) -> Option<R> {
    handle.with_value(|slot| slot.as_ref().map(f))
}

