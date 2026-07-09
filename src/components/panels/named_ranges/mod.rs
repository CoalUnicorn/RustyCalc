//! "Manage Named Ranges" right-drawer panel.
//!
//! Layout: list of all defined names on top, edit form below. Selecting a
//! row populates the form; the form's Save / Delete / New buttons drive the
//! CRUD methods on the model traits.
//!
//! Mounts only while [`WorkbookState::active_drawer`] is
//! `Some(ActiveDrawer::NamedRanges)`.

use leptos::prelude::*;

use crate::components::ui::drawer::{Drawer, DrawerWidth};
use crate::state::ActiveDrawer;
use crate::state::WorkbookState;

pub mod form;
pub mod formula_input;
pub mod list;

use form::NamedRangeForm;
use list::NamedRangesList;

#[component]
pub fn NamedRangesDialog() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    let close = Callback::new(move |_: ()| {
        if state.range_capture.get_untracked().is_some() {
            state.range_capture.set(None);
            return;
        }
        state.editing_named_range.set(None);
        state.active_drawer.set(None);
    });

    view! {
        <Show when=move || matches!(state.active_drawer.get(), Some(ActiveDrawer::NamedRanges))>
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
