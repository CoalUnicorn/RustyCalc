use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::app_state::AppState;
use crate::components::context_menu::{ContextMenu, ContextMenuItem, ContextMenuSeparator};
use crate::events::*;
use crate::input::xlsx_io;
use crate::state::{ModelStore, WorkbookState};
use crate::theme::Theme;

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
    let state = expect_context::<WorkbookState>();
    let app = expect_context::<AppState>();
    let model = expect_context::<ModelStore>();

    // Sidebar
    let on_sidebar = move |_| app.sidebar_open.set(!app.sidebar_open.get_untracked());
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
        let Some(file) = file else { return }; // no file selected

        spawn_local(async move {
            let bytes = xlsx_io::read_file_bytes(file).await;
            match xlsx_io::import_xlsx(&bytes, "workbook") {
                Ok(new_model) => {
                    model.set_value(new_model);
                    let sheet = model.with_value(|m| m.get_selected_view().sheet);
                    state.emit_events([
                        SpreadsheetEvent::Content(ContentEvent::GenericChange),
                        SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                            sheet,
                            col: None,
                            row: None,
                        }),
                    ]);
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
        model.with_value(|m| match xlsx_io::export_xlsx(m) {
            Ok(bytes) => xlsx_io::trigger_download(&bytes, &format!("{}.xlsx", m.get_name())),
            Err(e) => {
                web_sys::console::warn_1(&format!("xlsx export failed: {e}").into());
            }
        });
        crate::util::refocus_workbook();
    };

    // Theme toggle - right-aligned icon button.
    // DOM update and localStorage persistence are handled by the
    // use_rusty_calc_theme sync Effect in App.
    let on_toggle_theme = move |_: web_sys::MouseEvent| {
        app.toggle_theme();
    };

    let theme_icon = move || match app.theme.get() {
        Theme::Auto => {
            if app.get_theme() == Theme::Dark {
                "🌙"
            } else {
                "☀️"
            }
        }
        Theme::Light => "🌙",
        Theme::Dark => "☀️",
    };

    let theme_title = move || match app.theme.get() {
        Theme::Auto => "Theme: Auto (click to switch)",
        Theme::Light => "Theme: Light (click for Dark)",
        Theme::Dark => "Theme: Dark (click for Auto)",
    };

    view! {
        <div class="file-bar">
            <button
            on:click=on_sidebar
            >
                "<"
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
                class="file-menu-btn"
                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                on:click=on_file_click
            >
                "File"
            </button>
            <ContextMenu open=menu_open set_open=set_menu_open pos=menu_pos>
                <ContextMenuItem on_click=on_import icon="⬆">"Import .xlsx"</ContextMenuItem>
                <ContextMenuItem on_click=on_export icon="⬇">"Download .xlsx"</ContextMenuItem>
                <ContextMenuSeparator />
                /*<ContextMenuItem on_click=on_toggle_perf icon="⏱">
                    {perf_label}
                </ContextMenuItem>*/
            </ContextMenu>

            // Right: theme toggle
            <div class="file-bar-right">
                <button
                    class="theme-btn"
                    on:click=on_toggle_theme
                    title=theme_title
                >
                    {theme_icon}
                </button>
            </div>
        </div>
    }
}
