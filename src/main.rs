use leptos::{mount::mount_to_body, prelude::*};

mod app;
// mod app_enhanced; // 🚀 Enhanced app with storage & debouncing (disabled for now)
mod canvas;
mod components;
mod events;
mod input;
// mod leptos_use_integration; // 🎨 leptos-use examples and patterns (disabled for now)
mod model;
pub mod perf;
mod state;
mod storage;
mod storage_enhanced; // 🚀 Enhanced storage with quotas & monitoring
// mod storage_leptos_use; // 🚀 leptos-use storage integration (disabled for now)
mod theme;
mod util;

use app::App;

fn main() {
    mount_to_body(|| view! { <App /> })
}
