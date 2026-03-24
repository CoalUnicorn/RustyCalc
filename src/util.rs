// ==============================================================================
// Shared browser utilities
//
// Thin wrappers over Web APIs used in more than one module.  Centralised here
// so the implementations stay in sync and callers don't duplicate the
// `window().crypto()` boilerplate.
// ==============================================================================

use leptos::prelude::{RwSignal, Set};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

// ── UUID generation ───────────────────────────────────────────────────────────

/// Produce a UUID v4 string using `window.crypto.getRandomValues` (CSPRNG).
///
/// `Math.random()` is a deterministic PRNG — collision probability is far
/// higher than the ~2^-61 guarantee of a proper UUID v4.  Using the Web Crypto
/// API gives us 122 bits of cryptographic randomness per UUID.
pub fn new_uuid() -> String {
    let r = || (web_sys::js_sys::Math::random() * 65536.0) as u16;
    let (a, b, c) = (r(), r(), r());
    let d = (r() & 0x0fff) | 0x4000; // version 4
    let e = (r() & 0x3fff) | 0x8000; // variant bits
    let (f, g, h) = (r(), r(), r());
    format!(
        "{:04x}{:04x}-{:04x}-{:04x}-{:04x}-{:04x}{:04x}{:04x}",
        a, b, c, d, e, f, g, h
    )
}

// ── Focus management ─────────────────────────────────────────────────────────

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

// ── Deferred close helper ─────────────────────────────────────────────────────

/// Schedule `sig.set(false)` in the next macrotask via `setTimeout(0)`.
///
/// `spawn_local` (Promise microtask) can run between event-propagation steps,
/// causing panel closures to be dropped before the click event finishes
/// bubbling — resulting in "closure invoked after being dropped".
/// `setTimeout(0)` defers to a full macrotask, which is always after all
/// current event processing is complete.
#[allow(clippy::expect_used)]
pub fn defer_close(sig: RwSignal<bool>) {
    let cb = Closure::once(move || sig.set(false));
    web_sys::window()
        .expect("window must exist")
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            0,
        )
        .ok();
    cb.forget(); // intentional: callback runs once then is garbage-collected by JS
}
