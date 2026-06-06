//! "Manage Named Ranges" modal.
//!
//! Layout: list of all defined names on top, edit form below. Selecting a
//! row populates the form; the form's Save / Delete / New buttons drive the
//! CRUD methods on the model traits.
//!
//! The dialog mounts only while [`WorkbookState::named_ranges_modal_open`]
//! is `true`. Closing the modal goes through one channel: `set_open(false)`
//! — backdrop click, Esc, the X icon, and Cancel all funnel here.

use leptos::prelude::*;

use crate::components::ui::drawer::{Drawer, DrawerWidth};
use crate::state::WorkbookState;

pub mod form;
pub mod formula_input;
pub mod list;

use form::NamedRangeForm;
use list::NamedRangesList;

#[component]
pub fn NamedRangesDialog() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Closing funnels through one channel: Esc, the X icon, and Cancel all
    // call this. Esc precedence: if a range pick is in progress, the first Esc
    // only disarms it (the grid is live — the user is mid-selection); a second
    // Esc then closes. Otherwise clear the in-progress edit so reopening starts
    // on the empty state, not on a half-typed row.
    let close = Callback::new(move |_: ()| {
        if state.range_capture.get_untracked().is_some() {
            state.range_capture.set(None);
            return;
        }
        state.editing_named_range.set(None);
        state.named_ranges_modal_open.set(false);
    });

    view! {
        <Show when=move || state.named_ranges_modal_open.get()>
            <Drawer
                title="Manage Named Ranges"
                on_close=close
                width=DrawerWidth::Large
            >
                <div class="nrm">
                    <NamedRangesList />
                    <NamedRangeForm />
                </div>
            </Drawer>
        </Show>
    }
}
