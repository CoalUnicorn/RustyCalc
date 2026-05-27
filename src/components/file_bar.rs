use std::io::Cursor;

use ironcalc::{export, import};
use ironcalc_base::{Model, UserModel};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::app_state::AppState;
use crate::components::context_menu::{ContextMenu, ContextMenuItem, ContextMenuSeparator};
use crate::components::share_popover::SharePopover;
use crate::input::workbook::{WorkbookAction, execute_workbook};
use crate::input::xlsx_io;
use crate::state::StatusMessage;
use crate::state::{ModelStore, WorkbookState};
use crate::storage;
use crate::theme::Theme;

#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
enum FileChangeError {
    #[error("change event has no target")]
    NoTarget,
    #[error("file input has no FileList")]
    NoFileList,
}

/// Extract the input element and first selected file from a change event.
///
/// Returns the `HtmlInputElement` alongside the file so callers can clear
/// the value (`input.set_value("")`) after the async import completes -
/// allowing the same file to be re-imported without a second pick.
#[allow(dead_code)]
fn extract_file_input(
    ev: &web_sys::Event,
) -> Result<(web_sys::HtmlInputElement, Option<web_sys::File>), FileChangeError> {
    let target = ev.target().ok_or(FileChangeError::NoTarget)?;
    let input = target.unchecked_into::<web_sys::HtmlInputElement>();
    let files = input.files().ok_or(FileChangeError::NoFileList)?;
    Ok((input, files.get(0)))
}

#[component]
pub fn FileBar() -> impl IntoView {
    #[allow(unused)]
    let state = expect_context::<WorkbookState>();
    let app = expect_context::<AppState>();
    #[allow(unused)]
    let model = expect_context::<ModelStore>();

    // Sidebar
    let on_sidebar = move |_| app.sidebar_open.set(!app.sidebar_open.get_untracked());

    // Trust state for the active workbook. The badge appears whenever the
    // current entry's `shared_from_link` flag is set; clicking promotes the
    // workbook and bumps the registry so the sidebar 🔗 badge clears too.
    // We re-read on registry_version so promotion / workbook switch refresh.
    let is_shared_current = move || {
        let _ = app.registry_version.get();
        let uuid = state.current_uuid.get()?;
        storage::load_registry()
            .get(&uuid)
            .map(|m| m.shared_from_link)
    };
    let on_trust = move |_: web_sys::MouseEvent| {
        let Some(uuid) = state.current_uuid.get_untracked() else {
            return;
        };
        storage::promote_from_shared(&uuid);
        app.bump_registry();
    };
    // File menu - owned signals + button anchor ref for positioning.
    let (menu_open, set_menu_open) = signal(false);
    let (menu_pos, set_menu_pos) = signal((0i32, 0i32));
    let file_btn_ref = NodeRef::<leptos::html::Button>::new();

    let on_file_click = move |_: web_sys::MouseEvent| {
        if let Some(el) = file_btn_ref.get() {
            let rect = el.get_bounding_client_rect();
            // Position menu at the bottom-left of the File button.
            set_menu_pos.set((rect.left() as i32, rect.bottom() as i32));
        }
        set_menu_open.update(|v| *v = !*v);
    };

    // Hidden file input - triggered by the Import menu item.
    let file_input_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    let on_import = move || {
        if let Some(input) = file_input_ref.get() {
            input.click();
        }
    };

    let on_file_change = move |ev: web_sys::Event| {
        let (input, file) = match extract_file_input(&ev) {
            Ok(result) => result,
            Err(e) => {
                web_sys::console::warn_1(&format!("[FileBar] {e}").into());
                return;
            }
        };
        let Some(file) = file else { return };

        spawn_local(async move {
            let file_name = file.name();
            let stem = std::path::Path::new(&file_name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("workbook")
                .to_string();
            let bytes = match xlsx_io::read_file_bytes(file).await {
                Ok(b) => b,
                Err(e) => {
                    state.status.set(Some(StatusMessage::Error(e)));
                    return;
                }
            };
            let result = import::load_from_xlsx_bytes(&bytes, &stem, "en", "UTC")
                .map_err(|e| e.to_string())
                .and_then(|wb| Model::from_workbook(wb, "en").map_err(|e| e.to_string()))
                .map(UserModel::from_model);

            match result {
                Ok(new_model) => {
                    execute_workbook(WorkbookAction::Import(new_model), model, &state, app);
                }
                Err(e) => {
                    web_sys::console::warn_1(&format!("xlsx import failed: {e}").into());
                }
            }
            // Allow the same file to be re-imported next time.
            input.set_value("");
        });
    };

    let on_export = move || {
        model.with_value(|m| {
            match export::save_xlsx_to_writer(m.get_model(), Cursor::new(Vec::new())) {
                Ok(cursor) => {
                    let bytes = cursor.into_inner();
                    if let Err(e) =
                        xlsx_io::trigger_download(&bytes, &format!("{}.xlsx", m.get_name()), None)
                    {
                        state.status.set(Some(StatusMessage::Error(e)));
                    }
                }
                Err(e) => {
                    web_sys::console::warn_1(&format!("xlsx export failed: {e}").into());
                }
            }
        });
        crate::util::refocus_workbook();
    };

    // Share popover state. The URL is built eagerly on click (cheap) so the
    // popover gets a static String prop — keeps SharePopover non-reactive
    // and lets the host control refresh by re-mounting via <Show>.
    let (share_open, set_share_open) = signal(false);
    let share_url = RwSignal::new(String::new());
    let share_error = RwSignal::new(String::new());
    // Verification word for optional share-gating. Empty = no verification.
    let verify_word = RwSignal::new(String::new());

    let on_share = move || {
        let word: Option<String> = {
            let w = verify_word.get();
            let trimmed = w.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };
        let result = model.with_value(|m| storage::encode_for_share_url(m, word.as_deref()));
        match result {
            Ok(encoded) => {
                let loc = window().location();
                let origin = loc.origin().unwrap_or_default();
                // Include pathname so share links work on sub-path deploys
                // (e.g. https://host/calc/#share=… not https://host/#share=…).
                // For opaque origins (sandboxed iframes, file://), fall back
                // to the full href minus any existing hash.
                let base = if origin.is_empty() {
                    loc.href()
                        .unwrap_or_default()
                        .split('#')
                        .next()
                        .unwrap_or("/")
                        .to_string()
                } else {
                    let pathname = loc.pathname().unwrap_or_else(|_| "/".into());
                    format!("{origin}{pathname}")
                };
                share_url.set(format!("{base}#share={encoded}"));
                share_error.set(String::new());
                set_share_open.set(true);
            }
            Err(storage::ShareError::TooLarge { size_kb }) => {
                share_error.set(format!(
                    "This workbook is too large to share via link ({size_kb} KB). \
                     Use File → Download .xlsx instead."
                ));
            }
        }
    };

    // let on_toggle_perf = move || {
    //     app.show_perf_panel.update(|v| *v = !*v);
    // };

    // let perf_label = move || {
    //     if app.show_perf_panel.get() {
    //         "Hide perf panel"
    //     } else {
    //         "Show perf panel"
    //     }
    // };
    // Theme toggle - right-aligned icon button.
    // DOM update and localStorage persistence are handled by the
    // use_rusty_calc_theme sync Effect in App.
    let on_toggle_theme = move |_: web_sys::MouseEvent| {
        app.toggle_light_dark();
    };

    // Resolve Auto to the concrete system value so the icon is always accurate.
    let theme_icon = move || match app.get_theme() {
        Theme::Dark => "☀️",
        _ => "🌙",
    };

    let theme_title = move || match app.get_theme() {
        Theme::Dark => "Dark mode (click for Light)",
        _ => "Light mode (click for Dark)",
    };

    view! {
        <div class="fl">
            <button
                class="fl-hamburger"
                title="Workbooks sidebar"
                on:click=on_sidebar
            >
                "≡"
            </button>
            // Hidden file picker - triggered programmatically by Import item.
            <input
                type="file"
                accept=".xlsx"
                style="display:none"
                node_ref=file_input_ref
                on:change=on_file_change
            />

            // Left: menu bar trigger - stop pointerdown so on_click_outside
            // in ContextMenu doesn't immediately re-close the menu.
            <button
                node_ref=file_btn_ref
                class="fl-menu-btn"
                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                on:click=on_file_click
            >
                "File"
            </button>
            <ContextMenu open=menu_open set_open=set_menu_open pos=menu_pos>
                <ContextMenuItem on_click=on_import icon="⬆">"Import .xlsx"</ContextMenuItem>
                <ContextMenuItem on_click=on_export icon="⬇">"Download .xlsx"</ContextMenuItem>
                <ContextMenuItem on_click=on_share icon="🔗">"Share"</ContextMenuItem>
                <ContextMenuSeparator />
                /*
                <ContextMenuItem on_click=on_toggle_perf icon="⏱">
                    {perf_label}
                </ContextMenuItem>
                */
            </ContextMenu>

            <Show when=move || is_shared_current().unwrap_or(false)>
                <button
                    class="fl-trust-btn"
                    title="This workbook was loaded from a shared link. Click to trust it and clear the badge."
                    on:click=on_trust
                >
                    "🛡 Trust this workbook"
                </button>
            </Show>

            <Show when=move || share_open.get()>
                <SharePopover
                    share_url=share_url.get()
                    verify_word=verify_word.get()
                    on_close=Callback::new(move |_| set_share_open.set(false))
                />
            </Show>

            <Show when=move || !share_error.get().is_empty()>
                <div class="sp-error-banner">
                    {move || share_error.get()}
                </div>
            </Show>

            // Verification word input — shown inline after Share is clicked.
            // Clears when the popover closes.
            <Show when=move || share_open.get()>
                <div class="sp-word-row">
                    <input
                        type="text"
                        class="sp-word-input"
                        placeholder="Verification word (optional, 3+ letters)"
                        prop:value=verify_word
                        on:input=move |ev| {
                            verify_word.set(event_target_value(&ev));
                        }
                    />
                </div>
            </Show>

            // Right: theme toggle
            <div class="fl-right">
                <button
                    class="fl-theme"
                    on:click=on_toggle_theme
                    title=theme_title
                >
                    {theme_icon}
                </button>
            </div>
            /*
            <Show when=move || app.show_perf_panel.get()>
                <PerfPanel />
            </Show>
            */
        </div>
    }
}
