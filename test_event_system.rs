/*!
# Event System Integration Test

Quick test to verify that the event-driven toolbar components are working correctly.
Run this in your browser console or add as a temporary test component.
*/

use crate::state::WorkbookState;
use leptos::prelude::*;

/// Quick test component to verify event system integration
#[component]
pub fn EventSystemTest() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Test counters
    let (content_events_count, set_content_events_count) = signal(0);
    let (format_events_count, set_format_events_count) = signal(0);
    let (theme_events_count, set_theme_events_count) = signal(0);

    // Track event counts
    create_effect(move |_| {
        let content_events = state.subscribe_to_content_events();
        let format_events = state.subscribe_to_format_events();
        let theme_events = state.subscribe_to_theme_events();

        set_content_events_count.set(content_events().len());
        set_format_events_count.set(format_events().len());
        set_theme_events_count.set(theme_events().len());
    });

    view! {
        <div style="position: fixed; bottom: 16px; left: 16px; background: rgba(0,0,0,0.8); color: white; padding: 12px; border-radius: 4px; font-family: monospace; font-size: 12px;">
            <div><strong>"🧪 Event System Test"</strong></div>
            <div>"Content events: " {content_events_count}</div>
            <div>"Format events: " {format_events_count}</div>
            <div>"Theme events: " {theme_events_count}</div>

            <div style="margin-top: 8px; display: flex; gap: 8px;">
                <button
                    style="padding: 4px 8px; font-size: 11px;"
                    on:click=move |_| {
                        state.notify_cell_changed(1, 1, 1);
                        web_sys::console::log_1(&"Emitted content event".into());
                    }
                >
                    "Cell Edit"
                </button>

                <button
                    style="padding: 4px 8px; font-size: 11px;"
                    on:click=move |_| {
                        state.add_recent_color("#ff0000");
                        web_sys::console::log_1(&"Emitted format event".into());
                    }
                >
                    "Color Change"
                </button>

                <button
                    style="padding: 4px 8px; font-size: 11px;"
                    on:click=move |_| {
                        let current = state.get_theme_untracked();
                        let new_theme = match current {
                            crate::theme::Theme::Light => crate::theme::Theme::Dark,
                            crate::theme::Theme::Dark => crate::theme::Theme::Light,
                        };
                        state.notify_theme_changed(new_theme);
                        web_sys::console::log_1(&"Emitted theme event".into());
                    }
                >
                    "Toggle Theme"
                </button>
            </div>
        </div>
    }
}

// Add this to your main App component to test:
/*
<EventSystemTest />
*/
