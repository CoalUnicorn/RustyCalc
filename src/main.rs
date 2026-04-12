use leptos::{mount::mount_to_body, prelude::*};

mod app;
mod app_state;
mod canvas;
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

use app::App;

fn main() {
    mount_to_body(|| view! { <App /> })
}
