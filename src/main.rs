use leptos::{mount::mount_to_body, prelude::*};

mod app;
mod canvas;
mod components;
mod input;
mod model;
mod state;
mod storage;
mod util;

use app::App;

fn main() {
    mount_to_body(|| view! { <App /> })
}
