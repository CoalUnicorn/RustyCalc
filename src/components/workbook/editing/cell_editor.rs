//! Thin wrapper around [`FormulaTextArea`] that preserves the historical
//! `<CellEditor />` mount name used by `worksheet.rs`.
//!
//! All behavior (auto-focus, positioning, input sync, keydown filtering)
//! lives in `FormulaTextArea` — this file exists only so the mount site
//! stays readable and future per-editor concerns have a named home.

use leptos::prelude::*;

use crate::components::workbook::editing::formula_text_area::FormulaTextArea;

#[component]
pub fn CellEditor() -> impl IntoView {
    view! { <FormulaTextArea /> }
}
