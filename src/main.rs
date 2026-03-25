use leptos::{mount::mount_to_body, prelude::*};

mod action;
mod app;
mod canvas;
mod components;
mod formula_input;
mod model;
mod state;
mod storage;
mod theme;
mod util;

use app::App;

fn main() {
    mount_to_body(|| view! { <App /> })
}
