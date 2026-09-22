//! Dev-tools launcher for the performance inspector.
//!
//! The only control that lives outside the inspector window. It stays mounted
//! while the window is closed — otherwise the recording Stop action in the
//! Tools view would be unreachable. The button shows the `.icr` recording
//! state so a running recording is visible from the toolbar.
//!
//! `pointerdown` propagation is stopped so the button follows the same rule
//! as a `Popover` trigger: a press on it must not reach the surface behind.

use leptos::prelude::*;

use crate::app_state::AppState;

/// Toolbar button that toggles the inspector and mirrors the recording state.
#[component]
pub fn DevToolsLauncher() -> impl IntoView {
    let app = expect_context::<AppState>();

    let on_toggle = move |_: web_sys::MouseEvent| {
        app.inspector_open.update(|open| *open = !*open);
    };

    view! {
        <button
            id="dev-tools-launcher"
            class="tb-btn"
            class:active=move || app.inspector_open.get()
            title="Performance inspector"
            on:click=on_toggle
            on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
        >
            "\u{23f1} Perf"
            {move || app.recording_active.get().then(|| view! {
                <span class="tb-rec-dot" title="Recording a paint replay">
                    "\u{25cf}"
                </span>
            })}
        </button>
    }
}
