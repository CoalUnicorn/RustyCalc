//! File tab: import and export of `.xlsx` workbooks, plus one-shot SVG and
//! PDF snapshots of the worksheet canvas.

use std::io::Cursor;

use iron_canvas::{CanvasSession, RenderRequest, RevisionToken};
use iron_canvas_core::{CanvasTheme, CellCoord, scene_geometry::GridRange};
#[cfg(feature = "export")]
use iron_canvas_export::PdfSceneBackend;
use iron_canvas_export::SvgSceneBackend;
use ironcalc::{export, import};
use ironcalc_base::{Model, UserModel};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::app_state::AppState;
use crate::input::mouse::CanvasHandle;
use crate::input::workbook::{WorkbookAction, execute_workbook};
use crate::input::xlsx_io;
use crate::model::AppClipboard;
use crate::scene::request_for;
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

/// Document format for a canvas snapshot download.
#[derive(Clone, Copy)]
enum ExportKind {
    Svg,
    #[cfg(feature = "export")]
    Pdf,
}

impl ExportKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            #[cfg(feature = "export")]
            Self::Pdf => "pdf",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Svg => "image/svg+xml",
            #[cfg(feature = "export")]
            Self::Pdf => "application/pdf",
        }
    }
}

/// Build the render request both document exports share: the visible sheet
/// origin and theme, the live overlays plus the app clipboard, and the
/// worksheet canvas's logical size. Mirrors the rAF loop's request so the
/// document matches the pixels on screen.
fn export_request(
    model: ModelStore,
    state: WorkbookState,
    canvas_handle: CanvasHandle,
    clipboard: StoredValue<Option<AppClipboard>, LocalStorage>,
) -> Result<RenderRequest, String> {
    let size = canvas_handle
        .with_value(|slot| slot.as_ref().map(|handle| handle.size()))
        .ok_or_else(|| "the worksheet canvas is not ready".to_string())?;
    let (sheet, top_row, left_column) = model.with_value(|m| {
        let view = m.get_selected_view();
        (view.sheet, view.top_row, view.left_column)
    });
    let theme = window()
        .document()
        .and_then(|document| document.document_element())
        .map(|element| iron_canvas_canvas2d::theme_from_element::from_element(&element))
        .unwrap_or_else(CanvasTheme::light);
    let mut overlays = state.overlays.get_untracked();
    overlays.clipboard = clipboard.with_value(|clipboard| {
        clipboard
            .as_ref()
            .map(|clipboard| GridRange::from(clipboard.range))
    });
    Ok(request_for(
        sheet,
        CellCoord {
            row: top_row,
            col: left_column,
        },
        size,
        theme,
        overlays,
        RevisionToken {
            workbook_id: 0,
            revision: state.render_revision.get_untracked(),
        },
    ))
}

/// Render one SVG/PDF document through a throwaway scene session, borrowing
/// the workbook only for the duration of the render. Returns the download
/// filename and the encoded bytes.
fn export_document(
    kind: ExportKind,
    model: ModelStore,
    state: WorkbookState,
    canvas_handle: CanvasHandle,
    clipboard: StoredValue<Option<AppClipboard>, LocalStorage>,
) -> Result<(String, Vec<u8>), String> {
    let request = export_request(model, state, canvas_handle, clipboard)?;
    let extension = kind.extension();
    model.with_value(|m| match kind {
        ExportKind::Svg => {
            let mut session = CanvasSession::new(SvgSceneBackend::new());
            session.render(m, &request).map_err(|e| e.to_string())?;
            let document = session
                .backend()
                .document()
                .ok_or("SVG export produced no document")?
                .to_owned();
            Ok((
                format!("{}.{extension}", m.get_name()),
                document.into_bytes(),
            ))
        }
        #[cfg(feature = "export")]
        ExportKind::Pdf => {
            let mut session = CanvasSession::new(PdfSceneBackend::new());
            session.render(m, &request).map_err(|e| e.to_string())?;
            let document = session
                .backend()
                .document()
                .ok_or("PDF export produced no document")?
                .to_vec();
            Ok((format!("{}.{extension}", m.get_name()), document))
        }
    })
}

/// Export one document and hand it to the browser as a download, surfacing
/// any failure as a status-bar message.
fn run_export(
    kind: ExportKind,
    model: ModelStore,
    state: WorkbookState,
    canvas_handle: CanvasHandle,
    clipboard: StoredValue<Option<AppClipboard>, LocalStorage>,
) {
    match export_document(kind, model, state, canvas_handle, clipboard) {
        Ok((filename, bytes)) => {
            if let Err(e) = xlsx_io::trigger_download(&bytes, &filename, Some(kind.mime())) {
                state.status.set(Some(StatusMessage::Error(e)));
            }
        }
        Err(e) => {
            state
                .status
                .set(Some(StatusMessage::Error(format!("Export failed: {e}"))));
        }
    }
    crate::util::refocus_workbook();
}

#[component]
pub fn FileOps() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let app = expect_context::<AppState>();
    let model = expect_context::<ModelStore>();
    // Scene handle + app clipboard, shared with the rAF loop, so a snapshot
    // exports the same viewport and overlays the canvas is painting.
    let canvas_handle = expect_context::<CanvasHandle>();
    let clipboard = expect_context::<StoredValue<Option<AppClipboard>, LocalStorage>>();

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
                    state
                        .status
                        .set(Some(StatusMessage::Error(format!("Import failed: {e}"))));
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
                    state
                        .status
                        .set(Some(StatusMessage::Error(format!("Export failed: {e}"))));
                }
            }
        });
        crate::util::refocus_workbook();
    };

    let on_export_svg = move |_: web_sys::MouseEvent| {
        run_export(ExportKind::Svg, model, state, canvas_handle, clipboard);
    };

    #[cfg(feature = "export")]
    let on_export_pdf = move |_: web_sys::MouseEvent| {
        run_export(ExportKind::Pdf, model, state, canvas_handle, clipboard);
    };

    // PDF is compiled only when `export` is on; the SVG button is always
    // present, so the slot renders either way.
    let pdf_button = {
        #[cfg(feature = "export")]
        {
            view! {
                <button class="tb-btn" title="Download .pdf" on:click=on_export_pdf>
                    <Icon icon=FileIcon::Download /> " Download .pdf"
                </button>
            }
            .into_any()
        }
        #[cfg(not(feature = "export"))]
        {
            ().into_any()
        }
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
        <button class="tb-btn" title="Download .svg" on:click=on_export_svg>
            <Icon icon=FileIcon::Download /> " Download .svg"
        </button>
        {pdf_button}
    }
}
