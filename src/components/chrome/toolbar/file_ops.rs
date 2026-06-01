//! File tab: import and export of `.xlsx` workbooks.

use std::io::Cursor;

use ironcalc::{export, import};
use ironcalc_base::{Model, UserModel};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::app_state::AppState;
use crate::input::workbook::{WorkbookAction, execute_workbook};
use crate::input::xlsx_io;
use crate::state::{ModelStore, StatusMessage, WorkbookState};

use super::icon::{FileIcon, Icon};

#[derive(Debug, thiserror::Error)]
enum FileChangeError {
    #[error("change event has no target")]
    NoTarget,
    #[error("file input has no FileList")]
    NoFileList,
}

/// Extract the input element and first selected file from a change event.
///
/// Returns the input alongside the file so the caller can clear its value
/// (`input.set_value("")`) after the async import, allowing the same file to be
/// re-imported without a second pick.
fn extract_file_input(
    ev: &web_sys::Event,
) -> Result<(web_sys::HtmlInputElement, Option<web_sys::File>), FileChangeError> {
    let target = ev.target().ok_or(FileChangeError::NoTarget)?;
    let input = target.unchecked_into::<web_sys::HtmlInputElement>();
    let files = input.files().ok_or(FileChangeError::NoFileList)?;
    Ok((input, files.get(0)))
}

#[component]
pub fn FileOps() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let app = expect_context::<AppState>();
    let model = expect_context::<ModelStore>();

    // Hidden file input — triggered programmatically by the Import button.
    let file_input_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    let on_import = move |_: web_sys::MouseEvent| {
        if let Some(input) = file_input_ref.get() {
            input.click();
        }
    };

    let on_file_change = move |ev: web_sys::Event| {
        let (input, file) = match extract_file_input(&ev) {
            Ok(result) => result,
            Err(e) => {
                web_sys::console::warn_1(&format!("[FileOps] {e}").into());
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

    let on_export = move |_: web_sys::MouseEvent| {
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

    view! {
        <input
            type="file"
            accept=".xlsx"
            style="display:none"
            node_ref=file_input_ref
            on:change=on_file_change
        />
        <button class="tb-btn" title="Import .xlsx" on:click=on_import>
            <Icon icon=FileIcon::Import /> " Import"
        </button>
        <button class="tb-btn" title="Download .xlsx" on:click=on_export>
            <Icon icon=FileIcon::Download /> " Download"
        </button>
    }
}
