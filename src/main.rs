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
pub mod perf;

mod state;
mod storage;
mod theme;
mod util;
mod verify;

#[cfg(test)]
mod test;

use app::App;

fn main() {
    //console_error_panic_hook::set_once();
    mount_to_body(|| view! { <App /> })
}
