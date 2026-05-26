     1|//! Number format picker for the toolbar.
     2|//!
     3|//! Renders a "123 ▾" button that opens a dropdown listing common number
     4|//! formats. The active format is indicated with a checkmark. Selecting
     5|//! an entry applies `FormatAction::SetNumFmt` to the current selection.
     6|
     7|use leptos::prelude::*;
     8|use leptos_use::on_click_outside;
     9|
    10|use crate::input::keyboard::{execute, SpreadsheetAction};
    11|use crate::model::SheetQuery;
    12|use crate::state::{ModelStore, WorkbookState};
    13|use crate::util::refocus_workbook;
    14|
    15|/// (format_code, display_label, right-aligned preview hint)
    16|/// `None` entries render as visual separators between format groups.
    17|const FORMATS: &[Option<(&str, &str, &str)>] = &[
    18|    Some(("general", "Auto", "")),
    19|    Some(("#,##0.00", "Number", "1,234.57")),
    20|    Some(("0%", "Percentage", "10%")),
    21|    None,
    22|    Some(("#,##0.00 €", "Euro (EUR)", "€")),
    23|    Some(("$#,##0.00", "Dollar (USD)", "$")),
    24|    Some(("£#,##0.00", "British Pound (GBP)", "£")),
    25|    None,
    26|    Some(("dd/mm/yyyy", "Short date", "15/12/2025")),
    27|    Some(("d mmmm yyyy", "Long date", "15 December 2025")),
    28|];
    29|
    30|/// Toolbar number format picker.
    31|///
    32|/// Reads `active_num_fmt()` on the current cell to show a checkmark next to
    33|/// the matching preset. Selecting a preset calls `SpreadsheetAction::set_num_fmt`.
    34|#[component]
    35|pub fn NumberFormatPicker() -> impl IntoView {
    36|    let state = expect_context::<WorkbookState>();
    37|    let model = expect_context::<ModelStore>();
    38|
    39|    let (open, set_open) = signal(false);
    40|
    41|    // Reacts to both format events (after applying a format) and navigation
    42|    // (selection change may show a different cell's format).
    43|    let current_fmt = Memo::new(move |_| {
    44|        let _ = state.events.format.get();
    45|        let _ = state.events.navigation.get();
    46|        model.with_value(|m| m.active_num_fmt())
    47|    });
    48|
    49|    let container_ref = NodeRef::<leptos::html::Div>::new();
    50|    let _ = on_click_outside(container_ref, move |_| {
    51|        if open.get_untracked() {
    52|            set_open.set(false);
    53|        }
    54|    });
    55|
    56|    // `*entry` copies the Option<(&str,&str,&str)> (all types are Copy), giving
    57|    // code/label/preview as &'static str — sized and moveable into each item closure.
    58|    let items: Vec<_> = FORMATS
    59|        .iter()
    60|        .map(|entry| match *entry {
    61|            None => view! { <hr class="ctx-sep" /> }.into_any(),
    62|            Some((code, label, preview)) => {
    63|                let is_active = move || current_fmt.with(|f| f.eq_ignore_ascii_case(code));
    64|                let preview_class = if preview.is_empty() {
    65|                    "tb-num-fmt-preview tb-num-fmt-preview--empty"
    66|                } else {
    67|                    "tb-num-fmt-preview"
    68|                };
    69|
    70|                view! {
    71|                    <button
    72|                        class="tb-num-fmt-item"
    73|                        on:click=move |_| {
    74|                            set_open.set(false);
    75|                            execute(&SpreadsheetAction::set_num_fmt(code), model, &state);
    76|                            refocus_workbook();
    77|                        }
    78|                    >
    79|                        <span class="tb-num-fmt-check">
    80|                            {move || if is_active() { "✓" } else { "" }}
    81|                        </span>
    82|                        <span class="tb-num-fmt-label">{label}</span>
    83|                        <span class=preview_class>{preview}</span>
    84|                    </button>
    85|                }
    86|                .into_any()
    87|            }
    88|        })
    89|        .collect();
    90|
    91|    view! {
    92|        <div node_ref=container_ref class="tb-num-fmt">
    93|            <button
    94|                class="tb-btn"
    95|                title="Number format"
    96|                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
    97|                on:click=move |_| set_open.update(|o| *o = !*o)
    98|            >
    99|                "123 ▾"
   100|            </button>
   101|            // display:none is a runtime value (driven by open signal) — inline style is correct here
   102|            <div
   103|                class="tb-num-fmt-dropdown"
   104|                style=move || if open.get() { "" } else { "display:none;" }
   105|            >
   106|                {items}
   107|            </div>
   108|        </div>
   109|    }
   110|}
   111|