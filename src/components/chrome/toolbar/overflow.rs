//! Overflow: the pure width->count partition plus the `OverflowRow` component
//! that measures slot widths once and collapses the overflow into a `⋯` menu.

use leptos::html;
use leptos::prelude::*;
use leptos_use::{UseElementSizeReturn, use_element_size};
use wasm_bindgen::JsCast;

use super::section::ToolSlot;

const GAP: f64 = 4.0;
const MORE_W: f64 = 40.0;

/// Largest prefix of `widths` that fits in `avail` (px). When the full set
/// (with `gap` between items) already fits, returns `widths.len()` and reserves
/// nothing. Otherwise reserves `more_w` for the trailing `⋯` button and returns
/// how many leading slots fit in the remaining budget.
pub fn fit_count(widths: &[f64], gap: f64, avail: f64, more_w: f64) -> usize {
    let n = widths.len();
    let full = widths.iter().sum::<f64>() + gap * n.saturating_sub(1) as f64;
    if full <= avail {
        return n;
    }
    let budget = avail - more_w;
    let mut used = 0.0;
    let mut count = 0;
    for (i, w) in widths.iter().enumerate() {
        let add = if i == 0 { *w } else { gap + *w };
        if used + add > budget {
            break;
        }
        used += add;
        count += 1;
    }
    count
}

/// Renders `slots` in one row, collapsing trailing slots into a `⋯` menu when
/// they don't fit. Each slot is wrapped in `.tb-grp` so it collapses atomically
/// and exposes one measurable box.
#[component]
pub fn OverflowRow(slots: Vec<ToolSlot>) -> impl IntoView {
    let row_ref = NodeRef::<html::Div>::new();
    let widths = StoredValue::new(Vec::<f64>::new());
    let visible = RwSignal::new(usize::MAX);
    let len = slots.len();
    let slots = StoredValue::new_local(slots);

    let UseElementSizeReturn { width, .. } = use_element_size(row_ref);

    // Measure each `.tb-grp` box once (first frame after mount), then re-derive
    // the visible count on every container-width change.
    Effect::new(move |_| {
        let avail = width.get();
        if widths.with_value(Vec::is_empty)
            && let Some(el) = row_ref.get()
        {
            let kids = el.children();
            let mut ws = Vec::new();
            for i in 0..kids.length() {
                let Some(node) = kids.item(i) else { continue };
                let Ok(html_el) = node.dyn_into::<web_sys::HtmlElement>() else {
                    continue;
                };
                if !html_el.class_list().contains("tb-grp") {
                    continue;
                }
                ws.push(html_el.offset_width() as f64);
            }
            if !ws.is_empty() {
                widths.set_value(ws);
            }
        }

        widths.with_value(|ws| {
            if !ws.is_empty() && avail > 0.0 {
                visible.set(fit_count(ws, GAP, avail, MORE_W));
            }
        });
    });

    let inline = move || {
        let v = visible.get();
        slots.with_value(|s| {
            s.iter()
                .take(v)
                .map(|slot| view! { <div class="tb-grp">{(slot.view)()}</div> })
                .collect_view()
        })
    };

    let menu_items = move || {
        let v = visible.get();
        slots.with_value(|s| {
            s.iter()
                .skip(v)
                .map(|slot| {
                    view! {
                        <div class="tb-overflow-item">
                            {(slot.view)()}
                            <span class="tb-overflow-label">{slot.label}</span>
                        </div>
                    }
                })
                .collect_view()
        })
    };

    let open = RwSignal::new(false);

    view! {
        <div class="tb" node_ref=row_ref>
            {inline}
            <Show when=move || visible.get() < len>
                <div class="tb-overflow">
                    <button
                        class="tb-btn tb-overflow-btn"
                        title="More tools"
                        on:click=move |_| open.update(|o| *o = !*o)
                    >
                        "⋯"
                    </button>
                    <Show when=move || open.get()>
                        <div class="tb-overflow-menu">{menu_items}</div>
                    </Show>
                </div>
            </Show>
        </div>
    }
}
