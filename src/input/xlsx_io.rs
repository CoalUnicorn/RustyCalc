//! Client-side .xlsx import and export.
//!
//! All operations run in WASM - no server required. Import reads bytes from a
//! browser File object; export writes bytes into a Vec and triggers a download.
//!
//! Every browser API call is fallible — WASM panics (with `panic = "abort"` in
//! release mode) kill the entire tab. All methods return `Result<_, String>` so
//! callers can surface errors as status messages instead of crashing.

/// Read a browser `File` object into bytes.
///
/// Must be called from an async context (e.g. inside `spawn_local`).
/// Returns an error string suitable for display if the browser API fails.
pub async fn read_file_bytes(file: web_sys::File) -> Result<Vec<u8>, String> {
    use wasm_bindgen_futures::JsFuture;
    let buffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(|e| format!("Failed to read file: {e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Trigger a browser download of `bytes` with the given `filename`.
///
/// `mime` overrides the Blob `type:` attribute; passing `None` keeps the legacy
/// xlsx MIME (preserves drop-target hints for spreadsheet workflows). Pass
/// `Some("application/octet-stream")` for opaque binary artifacts such as
/// `.icr` paint-level recordings.
///
/// Returns `Ok(())` on success or an error string suitable for display.
pub fn trigger_download(bytes: &[u8], filename: &str, mime: Option<&str>) -> Result<(), String> {
    use wasm_bindgen::JsCast;

    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array);

    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(
        mime.unwrap_or("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    );

    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts)
        .map_err(|e| format!("Failed to create download: {e:?}"))?;

    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|e| format!("Failed to create download URL: {e:?}"))?;

    let document = leptos::prelude::document();

    let a: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(|e| format!("Failed to create download element: {e:?}"))?
        .unchecked_into();

    a.set_href(&url);
    a.set_download(filename);
    document
        .body()
        .ok_or_else(|| "No document body".to_string())?
        .append_child(&a)
        .map_err(|e| format!("Failed to trigger download: {e:?}"))?;
    a.click();
    a.remove();
    web_sys::Url::revoke_object_url(&url).ok();
    Ok(())
}
