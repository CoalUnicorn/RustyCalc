// Shared browser utilities

use wasm_bindgen::JsCast;

// Error logging

/// Log a `Result::Err` to the browser console and discard it.
///
/// Replaces bare `.ok()` on `UserModel` mutations so failures become visible
/// in DevTools instead of vanishing silently.  The `ctx` string identifies
/// the call site in the warning message.
///
/// ```ignore
/// warn_if_err(m.insert_rows(sheet, row, 1), "insert_rows");
/// ```
pub fn warn_if_err<E: std::fmt::Display>(result: Result<(), E>, ctx: &str) {
    if let Err(e) = result {
        web_sys::console::warn_1(&format!("[ironcalc] {ctx}: {e}").into());
    }
}

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
