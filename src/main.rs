//! RustyCalc — a browser spreadsheet built on Leptos and the `iron-canvas`
//! grid renderer, with `ironcalc_base` as the formula engine.
//!
//! This binary is the wasm entry point: it mounts [`App`] into the document
//! body. UI layers live under [`components`], input handling under
//! [`input`], the model bridge under [`model`], and reactive state under
//! [`state`].

use leptos::{mount::mount_to_body, prelude::*};

mod app;
mod app_state;
mod components;
mod coord;
mod events;
mod input;
mod model;
mod scene;

mod state;
mod storage;
mod theme;
mod util;
mod verify;

#[cfg(test)]
mod test;

use app::App;

fn main() {
    if let Some(body) = document().body() {
        body.set_text_content(Some("Opening workbooks…"));
    }
    wasm_bindgen_futures::spawn_local(async {
        let initial = async {
            storage::initialize().await?;
            match storage::load_selected().await? {
                Some(initial) => Ok(initial),
                None => storage::create_new().await,
            }
        }
        .await;
        if let Some(body) = document().body() {
            body.set_text_content(None);
        }
        match initial {
            Ok(initial) => mount_to_body(move || view! { <App initial /> }),
            Err(error) => mount_to_body(move || {
                view! {
                    <main role="alert">
                        <p>{error}</p>
                        <button on:click=move |_| { let _ = window().location().reload(); }>"Retry"</button>
                    </main>
                }
            }),
        }
    });
}
