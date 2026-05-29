//! Number format picker for the toolbar.
//!
//! Renders a "123 ▾" button that opens a Popover listing common number
//! formats. The active format is indicated with a checkmark. Selecting
//! an entry applies `FormatAction::SetNumFmt` to the current selection.

use leptos::prelude::*;

use crate::components::ui::popover::Popover;
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::model::SheetQuery;
use crate::state::{ModelStore, WorkbookState};
use crate::util::refocus_workbook;

/// (format_code, display_label, right-aligned preview hint)
/// `None` entries render as visual separators between format groups.
const FORMATS: &[Option<(&str, &str, &str)>] = &[
    Some(("general", "Auto", "")),
    Some(("#,##0.00", "Number", "1,234.57")),
    Some(("0%", "Percentage", "10%")),
    None,
    Some(("#,##0.00 €", "Euro (EUR)", "€")),
    Some(("$#,##0.00", "Dollar (USD)", "$")),
    Some(("£#,##0.00", "British Pound (GBP)", "£")),
    None,
    Some(("dd/mm/yyyy", "Short date", "15/12/2025")),
    Some(("d mmmm yyyy", "Long date", "15 December 2025")),
];

/// Toolbar number format picker.
///
/// Reads `active_num_fmt()` on the current cell to show a checkmark next to
/// the matching preset. Selecting a preset calls `SpreadsheetAction::set_num_fmt`.
#[component]
pub fn NumberFormatPicker() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let (open, set_open) = signal(false);
    let (pos, set_pos) = signal((0i32, 0i32));

    // Reacts to both format events (after applying a format) and navigation
    // (selection change may show a different cell's format).
    let current_fmt = Memo::new(move |_| {
        let _ = state.events.format.get();
        let _ = state.events.navigation.get();
        model.with_value(|m| m.active_num_fmt())
    });

    let trigger_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        set_pos.set((ev.client_x(), ev.client_y()));
        set_open.update(|v| *v = !*v);
    };

    // `*entry` copies the Option<(&str,&str,&str)> (all types are Copy), giving
    // code/label/preview as &'static str — sized and moveable into each item closure.
    let items: Vec<_> = FORMATS
        .iter()
        .map(|entry| match *entry {
            None => view! { <hr class="ctx-sep" /> }.into_any(),
            Some((code, label, preview)) => {
                let is_active = move || current_fmt.with(|f| f.eq_ignore_ascii_case(code));
                let preview_class = if preview.is_empty() {
                    "tb-num-fmt-preview tb-num-fmt-preview--empty"
                } else {
                    "tb-num-fmt-preview"
                };

                view! {
                    <button
                        class="tb-num-fmt-item"
                        on:click=move |_| {
                            set_open.set(false);
                            execute(&SpreadsheetAction::set_num_fmt(code), model, &state);
                            refocus_workbook();
                        }
                    >
                        <span class="tb-num-fmt-check">
                            {move || if is_active() { "✓" } else { "" }}
                        </span>
                        <span class="tb-num-fmt-label">{label}</span>
                        <span class=preview_class>{preview}</span>
                    </button>
                }
                .into_any()
            }
        })
        .collect();

    view! {
        <div class="tb-num-fmt">
            <button
                class="tb-btn"
                title="Number format"
                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                on:click=trigger_click
            >
                "123 ▾"
            </button>
            <Popover open set_open pos class="tb-num-fmt-dropdown">
                {items}
            </Popover>
        </div>
    }
}
