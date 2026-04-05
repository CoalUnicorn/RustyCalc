use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::events::*;
use crate::input::xlsx_io;
use crate::state::{ModelStore, WorkbookState};
use crate::theme::Theme;

#[component]
pub fn FileBar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Apply the initial theme to <html> on mount, so CSS variables match
    // the stored preference from the start.
    Effect::new(move |_| {
        apply_theme_to_dom(state.get_theme());
    });

    let on_toggle_theme = move |_: web_sys::MouseEvent| {
        state.toggle_theme(); // Use new toggle method with Auto support
        apply_theme_to_dom(state.get_theme());

        // Theme change event automatically fired by toggle_theme() → set_theme() → notify_theme_changed()
        // DEBUG
        // web_sys::console::log_1(&format!("Theme changed to: {:?}", state.get_theme()).into());
    };

    let on_toggle_perf = move |_: web_sys::MouseEvent| {
        state.show_perf_panel.update(|v| *v = !*v);
    };

    // Show icon based on theme preference with Auto detection
    let theme_icon = move || {
        let preference = state.theme.get();
        let resolved = state.get_theme();
        match preference {
            Theme::Auto => {
                if resolved == Theme::Dark {
                    "🌙 Auto (Dark)"
                } else {
                    "☀️ Auto (Light)"
                }
            }
            Theme::Light => "☾ Dark", // Currently light, click to go dark
            Theme::Dark => "☀ Light", // Currently dark, click to go light
        }
    };

    let perf_icon = move || {
        if state.show_perf_panel.get() {
            "⏱ Hide Perf"
        } else {
            "⏱ Show Perf"
        }
    };

    view! {
        <div class="file-bar">
            <div class="toolbar-sep" />
            <ImportExport />
            <button class="file-bar-btn" on:click=on_toggle_theme title="Toggle theme">
                {theme_icon}
            </button>
            <button class="file-bar-btn" on:click=on_toggle_perf title="Toggle performance panel">
                {perf_icon}
            </button>
        </div>
    }
}

/// Set `data-theme` on `<html>` so CSS variables switch.
fn apply_theme_to_dom(theme: Theme) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(el) = doc.document_element() {
            el.unchecked_ref::<web_sys::HtmlElement>()
                .dataset()
                .set("theme", theme.as_str())
                .ok();
        }
    }
}

// Import / Export .xlsx
#[component]
fn ImportExport() -> impl IntoView {
    use leptos::html;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::spawn_local;

    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let file_input_ref: NodeRef<html::Input> = NodeRef::new();

    let on_import_click = move |_: web_sys::MouseEvent| {
        if let Some(input) = file_input_ref.get() {
            let _ = input.click();
        }
    };

    let on_file_change = move |ev: web_sys::Event| {
        let input = ev
            .target()
            .expect("change event always has a target")
            .unchecked_into::<web_sys::HtmlInputElement>();
        let files = input.files().expect("file input always has a FileList");
        let Some(file) = files.get(0) else { return };

        spawn_local(async move {
            let bytes = xlsx_io::read_file_bytes(file).await;
            match xlsx_io::import_xlsx(&bytes, "workbook") {
                Ok(new_model) => {
                    model.set_value(new_model);
                    let sheet = model.with_value(|m| m.get_selected_view().sheet);
                    state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
                    state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                        sheet,
                        col: None,
                        row: None,
                    }));
                }
                Err(e) => {
                    web_sys::console::warn_1(&format!("xlsx import failed: {e}").into());
                }
            }
            // Clear the value so the same file can be re-selected next time.
            input.set_value("");
        });
    };

    let on_export_click = move |_: web_sys::MouseEvent| {
        model.with_value(|m| match xlsx_io::export_xlsx(m) {
            Ok(bytes) => xlsx_io::trigger_download(&bytes, &format!("{}.xlsx", m.get_name())),
            Err(e) => {
                web_sys::console::warn_1(&format!("xlsx export failed: {e}").into());
            }
        });
        crate::util::refocus_workbook();
    };

    view! {
        // Hidden file picker — triggered by the Import button below.
        <input
            type="file"
            accept=".xlsx"
            style="display:none"
            node_ref=file_input_ref
            on:change=on_file_change
        />
        <button class="toolbar-btn" title="Import .xlsx" on:click=on_import_click>
            "Load"
        </button>
        <button class="toolbar-btn" title="Export .xlsx" on:click=on_export_click>
            "Save"
        </button>
    }
}
