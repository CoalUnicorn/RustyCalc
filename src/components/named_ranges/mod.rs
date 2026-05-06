//! "Manage Named Ranges" modal.
//!
//! Layout: list of all defined names on top, edit form below. Selecting a
//! row populates the form; the form's Save / Delete / New buttons drive the
//! CRUD methods on [`crate::model::FrontendModel`].
//!
//! The dialog mounts only while [`WorkbookState::named_ranges_modal_open`]
//! is `true`. Closing the modal goes through one channel: `set_open(false)`
//! — backdrop click, Esc, the X icon, and Cancel all funnel here.

use leptos::prelude::*;

use crate::components::modal::{Modal, ModalSize};
use crate::state::WorkbookState;

pub mod form;
pub mod formula_input;
pub mod list;

use form::NamedRangeForm;
use list::NamedRangesList;

#[component]
pub fn NamedRangesDialog() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Closing the modal funnels through one channel: backdrop, Esc, X, and
    // Cancel all call this. Clears the in-progress edit so reopening the
    // dialog starts on the empty state, not on a half-typed row.
    let close = Callback::new(move |_: ()| {
        state.editing_named_range.set(None);
        state.named_ranges_modal_open.set(false);
    });

    view! {
        <Show when=move || state.named_ranges_modal_open.get()>
            <Modal
                title="Manage Named Ranges"
                on_close=close
                size=ModalSize::Large
            >
                <div class="nrm">
                    <NamedRangesList />
                    <NamedRangeForm />
                </div>
            </Modal>
        </Show>
    }
}
