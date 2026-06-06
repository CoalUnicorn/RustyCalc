//! "Conditional Formatting" right-drawer panel.
//!
//! Layout: rule list on the left, rule editor on the right. Selecting a row
//! populates the editor; the editor's Save / Delete / New buttons drive the
//! CRUD methods on `ironcalc_base::UserModel`.
//!
//! Mounts only while [`WorkbookState::active_drawer`] is
//! `Some(ActiveDrawer::ConditionalFormatting)`.

use leptos::prelude::*;

use crate::components::ui::drawer::{Drawer, DrawerWidth};
use crate::state::ActiveDrawer;
use crate::state::WorkbookState;

pub mod editor;
pub mod list;

use editor::CfRuleEditor;
use list::CfRuleList;

#[component]
pub fn ConditionalFormattingDialog() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    let close = Callback::new(move |_: ()| {
        if state.range_capture.get_untracked().is_some() {
            state.range_capture.set(None);
            return;
        }
        state.editing_cf_rule.set(None);
        state.active_drawer.set(None);
    });

    view! {
        <Show when=move || matches!(state.active_drawer.get(), Some(ActiveDrawer::ConditionalFormatting))>
            <Drawer title="Conditional Formatting" on_close=close width=DrawerWidth::Large>
                <div class="cfm">
                    <CfRuleList />
                    <CfRuleEditor />
                </div>
            </Drawer>
        </Show>
    }
}
