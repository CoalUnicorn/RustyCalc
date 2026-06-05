//! "Conditional Formatting" modal dialog.
//!
//! Layout: rule list on the left, rule editor on the right. Selecting a row
//! populates the editor; the editor's Save / Delete / New buttons drive the
//! CRUD methods on `ironcalc_base::UserModel`.
//!
//! The dialog mounts only while [`WorkbookState::cf_dialog_open`] is `true`.
//! Closing the dialog funnels through one channel: `set_open(false)` —
//! backdrop click, Esc, the X icon, and Cancel all call this.

use leptos::prelude::*;

use crate::components::ui::modal::{Modal, ModalSize};
use crate::state::WorkbookState;

pub mod editor;
pub mod list;

use editor::CfRuleEditor;
use list::CfRuleList;

#[component]
pub fn ConditionalFormattingDialog() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Closing the modal funnels through one channel: backdrop, Esc, X, and
    // Cancel all call this. Clears the in-progress edit so reopening the
    // dialog starts on the empty state.
    let close = Callback::new(move |_: ()| {
        state.editing_cf_rule.set(None);
        state.cf_dialog_open.set(false);
    });

    view! {
        <Show when=move || state.cf_dialog_open.get()>
            <Modal title="Conditional Formatting" on_close=close size=ModalSize::Large>
                <div class="cfm">
                    <CfRuleList />
                    <CfRuleEditor />
                </div>
            </Modal>
        </Show>
    }
}
