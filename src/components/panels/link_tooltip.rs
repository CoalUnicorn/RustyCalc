//! Hover tooltip for a committed cell hyperlink.
//!
//! A host DOM popover anchored to the link cell through
//! `IronCanvas::cell_rect` — no canvas layer. The pointer-adjacent cell is the
//! only cached fact (`WorkbookState::hover_link`); the link itself is read
//! from committed canvas state on every content/navigation/layout commit, so a
//! link deleted, retargeted, or scrolled away by the last frame cannot keep a
//! stale tooltip.

use leptos::html;
use leptos::prelude::*;

use crate::components::ui::popover::Popover;
use crate::input::link::activate_link;
use crate::input::mouse::{CanvasHandle, with_canvas};
use crate::state::{ModelStore, StatusMessage, WorkbookState};

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

        let anchor = state.hover_link.get().and_then(|(row, column)| {
            let rect = with_canvas(canvas_handle, |ic| {
                // Resolve the hovered physical cell to its logical cell: over a
                // merged range the anchor owns the link, and the tooltip is
                // placed against the visible fragment rather than the anchor's
                // own cell (which may be scrolled away entirely).
                let own = ic.cell_rect(row, column)?;
                let centre = own.center();
                let cell = ic.display_cell_at(f64::from(centre.x), f64::from(centre.y))?;
                // A link that no longer exists keeps no tooltip.
                cell.link.as_ref()?;
                Some(cell.fragment)
            })
            .flatten()?;
            let canvas_box = grid_ref.get_untracked()?.get_bounding_client_rect();
            Some((rect, canvas_box))
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
            .and_then(|(row, column)| {
                with_canvas(canvas_handle, |ic| {
                    let link = ic.link_at(row, column)?;
                    Some(match link.tooltip() {
                        Some(tooltip) => tooltip.to_string(),
                        None => link.target().as_str().to_string(),
                    })
                })
                .flatten()
            })
            .unwrap_or_default()
    };

    let on_open = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let Some((row, column)) = state.hover_link.get_untracked() else {
            return;
        };
        if let Err(e) = activate_link(model, &state, canvas_handle, row, column) {
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
