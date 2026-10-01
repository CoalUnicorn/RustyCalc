//! Hover tooltip for a committed cell hyperlink.
//!
//! Position, text, and activation resolve the pointer to the logical cell.
//! Each frame commit refreshes the query so the tooltip follows visible pixels.

use leptos::html;
use leptos::prelude::*;

use crate::components::ui::popover::Popover;
use crate::input::link::activate_link;
use crate::input::mouse::{CanvasHandle, with_canvas};
use crate::state::{ModelStore, StatusMessage, WorkbookState};

/// Resolve the pointer against the committed frame for every tooltip action.
pub(crate) fn hovered_link(
    canvas: CanvasHandle,
    pointer: Option<(f64, f64)>,
) -> Option<iron_canvas_core::DisplayCell> {
    let (x, y) = pointer?;
    with_canvas(canvas, |ic| ic.display_cell_at(x, y))
        .flatten()
        .filter(|cell| cell.link.is_some())
}

/// Hover tooltip with an explicit Open action. Mounted inside the worksheet so
/// `grid_ref` supplies the canvas-to-viewport offset for the anchor rect.
#[component]
pub fn LinkTooltip(grid_ref: NodeRef<html::Canvas>) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let canvas_handle = expect_context::<CanvasHandle>();

    let (open, set_open) = signal(false);
    let (pos, set_pos) = signal((0i32, 0i32));

    // Re-query on every commit: `committed_frame` is bumped by the render loop
    // once a frame is on screen, so the link and rect read here belong to that
    // frame. The content/navigation/format/structure event signals are *not*
    // tracked — they only schedule the paint, so an effect driven by them would
    // describe the frame before it.
    Effect::new(move |_| {
        let _ = state.committed_frame.get();

        let anchor = state.hover_link.get().and_then(|_| {
            let cell = hovered_link(canvas_handle, state.hover_pointer.get())?;
            let canvas_box = grid_ref.get_untracked()?.get_bounding_client_rect();
            Some((cell.fragment, canvas_box))
        });

        match anchor {
            Some((rect, canvas_box)) => {
                set_pos.set((
                    rect.left() + canvas_box.left() as i32,
                    rect.bottom() + canvas_box.top() as i32,
                ));
                set_open.set(true);
            }
            None => set_open.set(false),
        }
    });

    let link_text = move || {
        let _ = state.committed_frame.get();
        state
            .hover_link
            .get()
            .and_then(|_| {
                let cell = hovered_link(canvas_handle, state.hover_pointer.get())?;
                let link = cell.link?;
                Some(
                    link.tooltip()
                        .unwrap_or_else(|| link.target().as_str())
                        .to_string(),
                )
            })
            .unwrap_or_default()
    };

    let on_open = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let Some(cell) = hovered_link(canvas_handle, state.hover_pointer.get_untracked()) else {
            return;
        };
        if let Err(e) = activate_link(model, &state, canvas_handle, cell.anchor.r1, cell.anchor.c1)
        {
            state.status.set(Some(StatusMessage::Error(e.to_string())));
        }
        // Following the target moves the selection, so the hover no longer
        // applies; the effect would clear it on the next commit anyway.
        state.hover_link.set(None);
    };

    view! {
        <Popover open set_open pos dismiss_on_outside_click=false class="lt-pop">
            <div class="lt-body">
                <span class="lt-target">{link_text}</span>
                <button
                    class="lt-open"
                    title="Open link"
                    on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                    on:click=on_open
                >
                    "Open"
                </button>
            </div>
        </Popover>
    }
}
