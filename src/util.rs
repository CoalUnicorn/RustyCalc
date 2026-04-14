// Shared browser utilities

use wasm_bindgen::JsCast;

// Focus management

/// Move keyboard focus back to the `#workbook` container.
///
/// Called after an edit is committed or cancelled so subsequent keystrokes
/// reach the `Workbook` keydown handler without the user needing to click.
pub fn refocus_workbook() {
    if let Some(el) = leptos::prelude::document().get_element_by_id("workbook") {
        el.unchecked_into::<web_sys::HtmlElement>().focus().ok();
    }
}
