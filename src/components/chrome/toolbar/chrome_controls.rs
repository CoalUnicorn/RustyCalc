//! Persistent chrome controls on the toolbar header row: the sidebar hamburger
//! (left) and the repo/version link + theme toggle (right cluster). Pure chrome
//! — these act on the app/window, not the document.

use leptos::prelude::*;

use crate::app_state::AppState;
use crate::theme::Theme;

use super::icon::{ChromeIcon, Icon};

const REPO_URL: &str = "https://github.com/CoalUnicorn/RustyCalc";

/// Crate version for display in the header. A development build should be
/// visually distinct from a release so a tester can tell them apart at a glance.
pub fn app_version() -> String {
    let base = env!("CARGO_PKG_VERSION");
    if cfg!(debug_assertions) {
        format!("{base}-dev")
    } else {
        base.to_string()
    }
}

#[component]
pub fn HamburgerButton() -> impl IntoView {
    let app = expect_context::<AppState>();
    let on_sidebar =
        move |_: web_sys::MouseEvent| app.sidebar_open.set(!app.sidebar_open.get_untracked());
    view! {
        <button class="tb-ham" title="Workbooks sidebar" on:click=on_sidebar>
            <Icon icon=ChromeIcon::Menu />
        </button>
    }
}

#[component]
pub fn ChromeCluster() -> impl IntoView {
    let app = expect_context::<AppState>();
    let on_toggle_theme = move |_: web_sys::MouseEvent| app.toggle_light_dark();
    let theme_is_dark = move || matches!(app.get_theme(), Theme::Dark);
    let theme_title = move || {
        if theme_is_dark() {
            "Dark mode (click for Light)"
        } else {
            "Light mode (click for Dark)"
        }
    };
    view! {
        <a
            class="tb-repo"
            href=REPO_URL
            target="_blank"
            rel="noopener"
            title="Open the RustyCalc repository"
        >
            <Icon icon=ChromeIcon::GitHub />
            <span class="tb-ver">{app_version()}</span>
        </a>
        <button class="tb-theme" on:click=on_toggle_theme title=theme_title>
            {move || {
                if theme_is_dark() {
                    view! { <Icon icon=ChromeIcon::Sun /> }.into_any()
                } else {
                    view! { <Icon icon=ChromeIcon::Moon /> }.into_any()
                }
            }}
        </button>
    }
}
