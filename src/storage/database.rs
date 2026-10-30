//! Browser IndexedDB bridge. JavaScript owns transactions and the metadata cache.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/storage/indexeddb.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = initializeStorage)]
    pub(super) async fn initialize() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = storageRegistry)]
    pub(super) fn registry() -> String;
    #[wasm_bindgen(catch, js_name = loadWorkbook)]
    pub(super) async fn load(id: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = saveWorkbook)]
    pub(super) async fn save(
        id: &str,
        name: &str,
        bytes: &[u8],
        origin: &str,
    ) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = updateWorkbook)]
    pub(super) async fn update(id: &str, patch: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = deleteWorkbook)]
    pub(super) async fn delete(id: &str) -> Result<JsValue, JsValue>;
}

pub(super) fn error(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| {
        js_sys::Reflect::get(&value, &"message".into())
            .ok()
            .and_then(|message| message.as_string())
            .unwrap_or_else(|| format!("{value:?}"))
    })
}
