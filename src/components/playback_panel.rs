//! `.icr` recording playback controls.
//!
//! Status-bar sibling of [`PerfPanel`]. Renders only when the wasm was built
//! with `--features dev-tools` (runtime-checked via
//! `IronCanvas::recordingSupported()`). The panel itself is stateless — every
//! interaction emits a [`PlaybackCmd`] which the Worksheet dispatch Effect
//! drains onto the live `IronCanvas`.
//!
//! Two visual modes:
//! - **Idle** (no recording loaded): a single "📂 Load .icr" file picker.
//! - **Loaded**: scrubber + play/pause + frame counter + exit.
//!
//! All wall-clock timing for play cadence is generated inside the rAF tick;
//! this component never reads the clock.

use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Event, HtmlInputElement, js_sys};

use crate::app_state::{AppState, PlaybackCmd};

#[component]
pub fn PlaybackPanel() -> impl IntoView {
    let app = expect_context::<AppState>();
    let recording_supported = iron_canvas_web::IronCanvas::recordingSupported();
    if !recording_supported {
        return view! { <span /> }.into_any();
    }

    let file_input: NodeRef<html::Input> = NodeRef::new();

    // FileReader is async; spawn_local + JsFuture is simpler than wiring an
    // onload Closure. The resulting bytes flow through PlaybackCmd::Load.
    let on_file_change = move |_ev: Event| {
        let Some(input) = file_input.get() else {
            return;
        };
        let Some(files) = input.files() else { return };
        let Some(file) = files.get(0) else { return };
        // Re-select-same-file fix: clear input value so onchange fires again.
        input.set_value("");
        spawn_local(async move {
            let Ok(buf_js) = JsFuture::from(file.array_buffer()).await else {
                return;
            };
            let Ok(buf) = buf_js.dyn_into::<js_sys::ArrayBuffer>() else {
                return;
            };
            let bytes = js_sys::Uint8Array::new(&buf).to_vec();
            app.playback_cmd.set(Some(PlaybackCmd::Load(bytes)));
        });
    };

    let on_play_pause = move |_| {
        let cmd = if app.playback_playing.get() {
            PlaybackCmd::Pause
        } else {
            PlaybackCmd::Play
        };
        app.playback_cmd.set(Some(cmd));
    };

    let on_exit = move |_| app.playback_cmd.set(Some(PlaybackCmd::Exit));

    let on_scrub = move |ev: Event| {
        let Some(target) = ev.target() else { return };
        let Ok(input) = target.dyn_into::<HtmlInputElement>() else {
            return;
        };
        if let Ok(idx) = input.value().parse::<u32>() {
            app.playback_cmd.set(Some(PlaybackCmd::Seek(idx)));
        }
    };

    view! {
        <span class="pp-sep">"|"</span>
        {move || {
            if app.playback_loaded.get() {
                view! {
                    <span class="pb-label">"▶ Playback"</span>
                    <button
                        class="pb-btn"
                        class:active=move || app.playback_playing.get()
                        title="Play / pause"
                        on:click=on_play_pause
                    >
                        {move || if app.playback_playing.get() { "⏸" } else { "▶" }}
                    </button>
                    <input
                        class="pb-scrub"
                        type="range"
                        min="0"
                        max=move || app.playback_frame_count.get().saturating_sub(1).to_string()
                        prop:value=move || app.playback_frame.get().to_string()
                        on:input=on_scrub
                    />
                    <span class="pb-counter">
                        {move || format!(
                            "{} / {}",
                            app.playback_frame.get(),
                            app.playback_frame_count.get().saturating_sub(1),
                        )}
                    </span>
                    <button class="pb-btn" title="Close playback" on:click=on_exit>"✕"</button>
                }
                .into_any()
            } else {
                view! {
                    <label class="pb-load" title="Load an .icr recording">
                        "📂 Load .icr"
                        <input
                            node_ref=file_input
                            type="file"
                            style="display:none"
                            accept=".icr,application/octet-stream"
                            on:change=on_file_change
                        />
                    </label>
                }
                .into_any()
            }
        }}
    }
    .into_any()
}
