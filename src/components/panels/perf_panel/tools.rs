//! Tools view: sheet export, paint-replay recording, and playback.
//!
//! Composes the controls that used to sit in the status strip. Every action
//! reports through a callback; the panel publishes the same commands as
//! before, so playback command handling stays in one place.
//!
//! The two groups answer different questions and are labelled apart:
//! "Capture diagnostics" exports the current sheet view, while "Record paint
//! replay" records and replays the paint stream. A selected attempt is never
//! what SVG or PDF export.

use leptos::prelude::*;

use crate::components::panels::playback_panel::PlaybackPanel;

/// Sheet export, `.icr` recording, and playback controls.
#[component]
pub(super) fn InspectorTools(
    recording_active: ReadSignal<bool>,
    playback_loaded: ReadSignal<bool>,
    on_record: Callback<()>,
    on_export_svg: Callback<()>,
    on_export_pdf: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="pp-tools">
            <section class="pp-tools-group">
                <h3 class="pp-tools-title">"Capture diagnostics"</h3>
                <p class="pp-tools-note">
                    "Exports the current sheet view. Not the selected attempt."
                </p>
                <div class="pp-tools-row">
                    <button
                        class="pp-action-btn"
                        type="button"
                        title="Download the current sheet as SVG"
                        on:click=move |_| on_export_svg.run(())
                    >
                        "\u{21e9} SVG"
                    </button>
                    <button
                        class="pp-action-btn"
                        type="button"
                        title="Export the current sheet as PDF"
                        on:click=move |_| on_export_pdf.run(())
                    >
                        "\u{21e9} PDF"
                    </button>
                </div>
            </section>

            <section class="pp-tools-group">
                <h3 class="pp-tools-title">"Record paint replay"</h3>
                <p class="pp-tools-note">
                    "Records and replays the paint stream as an .icr file."
                </p>
                <div class="pp-tools-row">
                    <button
                        class="pp-action-btn"
                        class:active=move || recording_active.get()
                        type="button"
                        disabled=move || playback_loaded.get()
                        title="Capture a paint-level .icr recording"
                        on:click=move |_| on_record.run(())
                    >
                        {move || if recording_active.get() {
                            "\u{25a0} Stop"
                        } else {
                            "\u{25cf} Record"
                        }}
                    </button>
                    {move || recording_active.get().then(|| view! {
                        <span class="pp-recording-label">"Recording..."</span>
                    })}
                </div>
                <div class="pp-tools-playback">
                    <PlaybackPanel />
                </div>
            </section>
        </div>
    }
}
