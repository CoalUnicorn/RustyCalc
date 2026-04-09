// Shared browser utilities

use wasm_bindgen::JsCast;

// UUID generation

/// Produce a UUID v4 string using `window.crypto.getRandomValues` (CSPRNG).
///
/// Fills 16 bytes via the Web Crypto API (122 bits of cryptographic randomness),
/// then stamps the version-4 and variant bits per RFC 9562 5.4.
#[allow(clippy::expect_used)]
pub fn new_uuid() -> String {
    let mut buf = [0u8; 16];
    let crypto = web_sys::window()
        .expect("window must exist in WASM context")
        .crypto()
        .expect("crypto must be available");
    crypto
        .get_random_values_with_u8_array(&mut buf)
        .expect("getRandomValues must not fail for 16 bytes");

    buf[6] = (buf[6] & 0x0f) | 0x40; // version 4
    buf[8] = (buf[8] & 0x3f) | 0x80; // variant 10xx

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        buf[0], buf[1], buf[2], buf[3],
        buf[4], buf[5],
        buf[6], buf[7],
        buf[8], buf[9],
        buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
    )
}

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
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("workbook"))
    {
        el.unchecked_into::<web_sys::HtmlElement>().focus().ok();
    }
}
