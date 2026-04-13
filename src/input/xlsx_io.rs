//! Client-side .xlsx import and export.
//!
//! All operations run in WASM - no server required. Import reads bytes from a
//! browser File object; export writes bytes into a Vec and triggers a download.

use std::io::Cursor;

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::{Model, UserModel};

/// Build a `UserModel` from raw .xlsx bytes.
///
/// Uses the same locale / timezone as the app's default model ("en" / "UTC").
pub fn import_xlsx(bytes: &[u8], name: &str) -> Result<UserModel<'static>, String> {
    let workbook = load_from_xlsx_bytes(bytes, name, "en", "UTC").map_err(|e| e.to_string())?;
    let model = Model::from_workbook(workbook, "en").map_err(|e| e.to_string())?;
    Ok(UserModel::from_model(model))
}

/// Serialize the current workbook to .xlsx bytes.
pub fn export_xlsx(user_model: &UserModel<'static>) -> Result<Vec<u8>, String> {
    let model = user_model.get_model();
    let cursor = save_xlsx_to_writer(model, Cursor::new(Vec::new())).map_err(|e| e.to_string())?;
    Ok(cursor.into_inner())
}

/// Read a browser `File` object into bytes.
///
/// Must be called from an async context (e.g. inside `spawn_local`).
#[allow(clippy::expect_used)]
pub async fn read_file_bytes(file: web_sys::File) -> Vec<u8> {
    use wasm_bindgen_futures::JsFuture;
    let buffer = JsFuture::from(file.array_buffer())
        .await
        .expect("File::array_buffer() is always resolvable in the browser");
    js_sys::Uint8Array::new(&buffer).to_vec()
}

/// Trigger a browser download of `bytes` with the given `filename`.
#[allow(clippy::expect_used)]
pub fn trigger_download(bytes: &[u8], filename: &str) {
    use wasm_bindgen::JsCast;

    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array);

    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet");

    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts)
        .expect("Blob construction from a Uint8Array always succeeds");

    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .expect("createObjectURL is always available in a secure context");

    let document = leptos::prelude::document();

    let a: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .expect("createElement('a') always succeeds")
        .unchecked_into();

    a.set_href(&url);
    a.set_download(filename);
    document
        .body()
        .expect("document has a body")
        .append_child(&a)
        .expect("append_child always succeeds");
    a.click();
    a.remove();
    web_sys::Url::revoke_object_url(&url).ok();
}
