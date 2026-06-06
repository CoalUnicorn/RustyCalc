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

use crate::components::ui::drawer::{Drawer, DrawerWidth};
use crate::state::WorkbookState;

pub mod editor;
pub mod list;

use editor::CfRuleEditor;
use list::CfRuleList;

#[component]
pub fn ConditionalFormattingDialog() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Closing funnels through one channel: Esc, the X icon, and Cancel all
    // call this. Esc precedence: if a range pick is in progress, the first Esc
    // only disarms it (the grid is live — the user is mid-selection); a second
    // Esc then closes. Otherwise clear the in-progress edit so reopening the
    // drawer starts on the empty state.
    let close = Callback::new(move |_: ()| {
        if state.range_capture.get_untracked().is_some() {
            state.range_capture.set(None);
            return;
        }
        state.editing_cf_rule.set(None);
        state.cf_dialog_open.set(false);
    });

    view! {
        <Show when=move || state.cf_dialog_open.get()>
            <Drawer title="Conditional Formatting" on_close=close width=DrawerWidth::Large>
                <div class="cfm">
                    <CfRuleList />
                    <CfRuleEditor />
                </div>
            </Drawer>
        </Show>
    }
}
