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
