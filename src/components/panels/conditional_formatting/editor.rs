//! Conditional-formatting rule editor (right column of the CF dialog).
//!
//! Mirrors the in-progress [`CfRuleEditState`] into a set of local signals,
//! lets the user shape one rule (type, operator, formula, format), then writes
//! it back through `add_/update_conditional_formatting`. Save/Delete emit a
//! `FormatEvent::ConditionalFormattingChanged` so the canvas repaints and the
//! rule list refreshes.

use ironcalc_base::cf_types::{
    CfRuleInput, Cfvo, ColorScaleThreshold, TextOperator, ValueOperator,
};
use ironcalc_base::types::{Dxf, DxfFont, Fill};
use leptos::prelude::*;

use crate::components::ui::color_picker::{ColorPicker, ColorType};
use crate::components::ui::formula_field::FormulaField;
use crate::components::ui::range_picker::{RangeFormat, RangePickerInput};
use crate::coord::{CellAddress, TextRef, selection_a1_relative};
use crate::events::{FormatEvent, SpreadsheetEvent};
use crate::input::formula::{analyze_formula, splice_ref};
use crate::model::frontend_model::DefinedNameManager;
use crate::model::style_types::HexColor;
use crate::model::{EvaluationMode, SheetQuery, try_mutate};
use crate::state::{ModelStore, RangeCaptureTarget, WorkbookState};

const RULE_TYPES: &[(&str, &str)] = &[
    ("cell_is", "Cell Value"),
    ("formula", "Formula"),
    ("text", "Text"),
    ("color_scale", "Color Scale"),
    ("duplicate", "Duplicate Values"),
    ("blanks", "Blanks"),
];

const VALUE_OPERATORS: &[(&str, &str)] = &[
    ("greater_than", "Greater Than"),
    ("less_than", "Less Than"),
    ("equal", "Equal"),
    ("not_equal", "Not Equal"),
    ("greater_than_or_equal", "Greater Than or Equal"),
    ("less_than_or_equal", "Less Than or Equal"),
    ("between", "Between"),
    ("not_between", "Not Between"),
];

fn operator_from_str(s: &str) -> ValueOperator {
    match s {
        "greater_than" => ValueOperator::GreaterThan,
        "less_than" => ValueOperator::LessThan,
        "equal" => ValueOperator::Equal,
        "not_equal" => ValueOperator::NotEqual,
        "greater_than_or_equal" => ValueOperator::GreaterThanOrEqual,
        "less_than_or_equal" => ValueOperator::LessThanOrEqual,
        "between" => ValueOperator::Between,
        "not_between" => ValueOperator::NotBetween,
        _ => ValueOperator::GreaterThan,
    }
}

fn operator_to_str(op: &ValueOperator) -> &'static str {
    match op {
        ValueOperator::GreaterThan => "greater_than",
        ValueOperator::LessThan => "less_than",
        ValueOperator::Equal => "equal",
        ValueOperator::NotEqual => "not_equal",
        ValueOperator::GreaterThanOrEqual => "greater_than_or_equal",
        ValueOperator::LessThanOrEqual => "less_than_or_equal",
        ValueOperator::Between => "between",
        ValueOperator::NotBetween => "not_between",
    }
}

#[component]
pub fn CfRuleEditor() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let selected_rule_type = RwSignal::new("cell_is".to_string());
    let selected_operator = RwSignal::new("greater_than".to_string());
    let range_text = RwSignal::new(String::new());
    let formula_text = RwSignal::new(String::new());
    let formula2_text = RwSignal::new(String::new());
    let text_value = RwSignal::new(String::new());
    let stop_if_true = RwSignal::new(false);
    let fill_color = RwSignal::new("#a5d6a7".to_string());
    let font_color = RwSignal::new("#000000".to_string());
    let bold = RwSignal::new(false);
    let operator_visible = RwSignal::new(true);
    let formula2_visible = RwSignal::new(false);
    let formula_visible = RwSignal::new(true);
    let text_visible = RwSignal::new(false);
    let format_visible = RwSignal::new(true);

    // Bridge the `String` color signals to the `ColorPicker`'s `HexColor` API.
    // An empty string means "no color" (transparent / clear), which `build_rule`
    // reads back as `fill: None` / no font color.
    let fill_color_hex = Signal::derive(move || {
        let s = fill_color.get();
        if s.is_empty() {
            None
        } else {
            HexColor::new(s).ok()
        }
    });
    let on_fill_change = Callback::new(move |c: Option<HexColor>| {
        fill_color.set(c.map(|h| h.as_str().to_owned()).unwrap_or_default());
    });
    let font_color_hex = Signal::derive(move || {
        let s = font_color.get();
        if s.is_empty() {
            None
        } else {
            HexColor::new(s).ok()
        }
    });
    let on_font_change = Callback::new(move |c: Option<HexColor>| {
        font_color.set(c.map(|h| h.as_str().to_owned()).unwrap_or_default());
    });

    // ── Value / Formula field: colored ref overlay + validation ──────────────
    // The field reuses the shared, storage-agnostic `FormulaField`, so we run
    // the analyzer here against the workbook's sheet + name tables, anchored at
    // the current view cell. `events.content` keeps those tables fresh; a
    // keystroke re-derives refs + validity cheaply. A plain literal like "100"
    // analyzes as NotFormula → no refs, no error class.
    let analyzer_ctx = Memo::new(move |_| {
        let _ = state.events.content.get();
        model.with_value(|m| {
            (
                m.get_sheet_names(),
                m.get_defined_names(),
                CellAddress::from_view(m),
            )
        })
    });
    let formula_analysis = Memo::new(move |_| {
        let text = formula_text.get();
        analyzer_ctx
            .with(|(sheet_names, defined_names, ctx)| analyze_formula(&text, *ctx, sheet_names, defined_names))
    });
    let formula_refs = Signal::derive(move || formula_analysis.with(|a| a.refs().to_vec()));
    let formula_is_error = Signal::derive(move || formula_analysis.with(|a| a.has_any_error()));

    // Grid point-picking state. `formula_cursor` tracks the caret so a grid
    // selection can splice a reference at the right spot; `cf_formula_prev_span`
    // remembers the just-inserted ref so a continued drag grows/replaces it
    // instead of appending a new ref on every selection tick.
    let formula_cursor = RwSignal::new(0usize);
    let cf_formula_prev_span = RwSignal::new(None::<TextRef>);

    let on_formula_input = Callback::new(move |(v, cursor): (String, usize)| {
        formula_text.set(v);
        formula_cursor.set(cursor);
        // Manual typing disarms grid-capture (so a stray selection can't clobber
        // hand-edited text) and invalidates the remembered insert span.
        if state.range_capture.get_untracked() == Some(RangeCaptureTarget::CfFormula) {
            state.range_capture.set(None);
        }
        cf_formula_prev_span.set(None);
    });

    let cf_formula_armed =
        move || state.range_capture.get() == Some(RangeCaptureTarget::CfFormula);
    let toggle_formula_arm = move |_: web_sys::MouseEvent| {
        if state.range_capture.get_untracked() == Some(RangeCaptureTarget::CfFormula) {
            state.range_capture.set(None);
        } else {
            cf_formula_prev_span.set(None);
            state.range_capture.set(Some(RangeCaptureTarget::CfFormula));
        }
    };

    // While armed, mirror each grid selection into the formula as a spliced ref.
    Effect::new(move |_| {
        // Subscribe to the navigation bus — selection changes are the trigger.
        let _ = state.events.navigation.get();
        if state.range_capture.get_untracked() != Some(RangeCaptureTarget::CfFormula) {
            return;
        }
        if state.editing_cf_rule.get_untracked().is_none() {
            return;
        }
        // The grid selection as a relative A1 reference (e.g. "A1" / "A1:C10").
        let ref_str = model.with_value(selection_a1_relative);

        // First selection of a drag inserts at the caret; every later tick
        // replaces the ref we inserted last time, so a drag grows ONE reference
        // (`A1` → `A1:B3`) instead of appending a new one each tick. `get_untracked`
        // throughout: this Effect is driven by the navigation bus, and must not
        // re-subscribe to the formula text (it writes it) or re-fire itself.
        let target_span = cf_formula_prev_span
            .get_untracked()
            .unwrap_or_else(|| TextRef::at(formula_cursor.get_untracked()));
        let (new_text, new_span) = splice_ref(&formula_text.get_untracked(), target_span, &ref_str);

        formula_cursor.set(new_span.end);
        cf_formula_prev_span.set(Some(new_span));
        formula_text.set(new_text);
    });

    // When editing_cf_rule changes, populate the local signals.
    Effect::new(move |_| {
        if let Some(edit) = &state.editing_cf_rule.get() {
            range_text.set(edit.range.clone());
            match &edit.rule {
                CfRuleInput::CellIs {
                    operator,
                    formula,
                    formula2,
                    format,
                    stop_if_true: s,
                    ..
                } => {
                    selected_rule_type.set("cell_is".into());
                    selected_operator.set(operator_to_str(operator).into());
                    formula_text.set(formula.clone());
                    formula2_text.set(formula2.clone().unwrap_or_default());
                    formula2_visible.set(matches!(
                        operator,
                        ValueOperator::Between | ValueOperator::NotBetween
                    ));
                    operator_visible.set(true);
                    formula_visible.set(true);
                    text_visible.set(false);
                    format_visible.set(true);
                    fill_color.set(
                        format
                            .fill
                            .as_ref()
                            .and_then(|f| f.color.clone())
                            .unwrap_or_default(),
                    );
                    font_color.set(
                        format
                            .font
                            .as_ref()
                            .and_then(|f| f.color.clone())
                            .unwrap_or_default(),
                    );
                    bold.set(format.font.as_ref().and_then(|f| f.b).unwrap_or(false));
                    stop_if_true.set(*s);
                }
                CfRuleInput::Formula {
                    formula,
                    format,
                    stop_if_true: s,
                    ..
                } => {
                    selected_rule_type.set("formula".into());
                    formula_text.set(formula.clone());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(true);
                    text_visible.set(false);
                    format_visible.set(true);
                    fill_color.set(
                        format
                            .fill
                            .as_ref()
                            .and_then(|f| f.color.clone())
                            .unwrap_or_default(),
                    );
                    font_color.set(
                        format
                            .font
                            .as_ref()
                            .and_then(|f| f.color.clone())
                            .unwrap_or_default(),
                    );
                    bold.set(format.font.as_ref().and_then(|f| f.b).unwrap_or(false));
                    stop_if_true.set(*s);
                }
                CfRuleInput::Text {
                    value,
                    format,
                    stop_if_true: s,
                    ..
                } => {
                    selected_rule_type.set("text".into());
                    text_value.set(value.clone());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(false);
                    text_visible.set(true);
                    format_visible.set(true);
                    fill_color.set(
                        format
                            .fill
                            .as_ref()
                            .and_then(|f| f.color.clone())
                            .unwrap_or_default(),
                    );
                    font_color.set(
                        format
                            .font
                            .as_ref()
                            .and_then(|f| f.color.clone())
                            .unwrap_or_default(),
                    );
                    bold.set(format.font.as_ref().and_then(|f| f.b).unwrap_or(false));
                    stop_if_true.set(*s);
                }
                CfRuleInput::ColorScale { .. } => {
                    selected_rule_type.set("color_scale".into());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(false);
                    text_visible.set(false);
                    format_visible.set(false);
                }
                CfRuleInput::DuplicateValues {
                    stop_if_true: s, ..
                } => {
                    selected_rule_type.set("duplicate".into());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(false);
                    text_visible.set(false);
                    format_visible.set(true);
                    stop_if_true.set(*s);
                }
                CfRuleInput::Blanks {
                    stop_if_true: s, ..
                } => {
                    selected_rule_type.set("blanks".into());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(false);
                    text_visible.set(false);
                    format_visible.set(true);
                    stop_if_true.set(*s);
                }
                _ => {}
            }
        }
    });

    let build_rule = move || -> CfRuleInput {
        let format = Dxf {
            font: if bold.get() || !font_color.get().is_empty() {
                let mut f = DxfFont::default();
                if bold.get() {
                    f.b = Some(true);
                }
                if !font_color.get().is_empty() {
                    f.color = Some(font_color.get());
                }
                Some(f)
            } else {
                None
            },
            fill: if fill_color.get().is_empty() {
                None
            } else {
                Some(Fill {
                    color: Some(fill_color.get()),
                })
            },
            ..Default::default()
        };

        match selected_rule_type.get().as_str() {
            "cell_is" => CfRuleInput::CellIs {
                operator: operator_from_str(&selected_operator.get()),
                formula: formula_text.get(),
                formula2: if formula2_visible.get() {
                    Some(formula2_text.get())
                } else {
                    None
                },
                format,
                stop_if_true: stop_if_true.get(),
            },
            "formula" => CfRuleInput::Formula {
                formula: formula_text.get(),
                format,
                stop_if_true: stop_if_true.get(),
            },
            "text" => CfRuleInput::Text {
                operator: TextOperator::Contains,
                value: text_value.get(),
                format,
                stop_if_true: stop_if_true.get(),
            },
            "color_scale" => CfRuleInput::ColorScale {
                thresholds: vec![
                    ColorScaleThreshold {
                        cfvo: Cfvo::Min,
                        color: "#63be7b".into(),
                    },
                    ColorScaleThreshold {
                        cfvo: Cfvo::Max,
                        color: "#f8696b".into(),
                    },
                ],
            },
            "duplicate" => CfRuleInput::DuplicateValues {
                format,
                stop_if_true: stop_if_true.get(),
            },
            "blanks" => CfRuleInput::Blanks {
                format,
                stop_if_true: stop_if_true.get(),
            },
            _ => CfRuleInput::CellIs {
                operator: ValueOperator::GreaterThan,
                formula: String::new(),
                formula2: None,
                format: Dxf::default(),
                stop_if_true: false,
            },
        }
    };

    let save = move |_: web_sys::MouseEvent| {
        let Some(edit) = state.editing_cf_rule.get() else {
            return;
        };
        let rule = build_rule();
        let range = range_text.get();
        if range.is_empty() {
            return;
        }

        let result = if let Some(index) = edit.index {
            try_mutate(model, EvaluationMode::Immediate, |m| {
                let sheet = m.get_selected_sheet();
                m.update_conditional_formatting(sheet, index, &range, rule)
            })
        } else {
            try_mutate(model, EvaluationMode::Immediate, |m| {
                let sheet = m.get_selected_sheet();
                m.add_conditional_formatting(sheet, &range, rule)
            })
        };

        if result.is_ok() {
            state.editing_cf_rule.set(None);
            let sheet = model.with_value(|m| m.get_selected_sheet());
            state.emit_event(SpreadsheetEvent::Format(
                FormatEvent::ConditionalFormattingChanged { sheet },
            ));
        }
    };

    let cancel = move |_: web_sys::MouseEvent| {
        state.editing_cf_rule.set(None);
    };

    let editing = move || state.editing_cf_rule.get().is_some();

    view! {
        <div class="cfm-editor">
            <Show
                when=editing
                fallback=move || view! {
                    <div class="cfm-editor-empty">
                        <p>"Select a rule to edit, or click '+ New Rule'"</p>
                    </div>
                }
            >
                <div class="cfm-editor-form">
                    {/* Rule type selector */}
                    <div class="cfm-field">
                        <label class="cfm-label">"Rule Type"</label>
                        <select
                            class="cfm-select"
                            on:change=move |ev| {
                                let val = event_target_value(&ev);
                                let is_between = val == "cell_is"
                                    && matches!(
                                        operator_from_str(&selected_operator.get()),
                                        ValueOperator::Between | ValueOperator::NotBetween
                                    );
                                formula2_visible.set(is_between);
                                operator_visible.set(val == "cell_is");
                                formula_visible.set(val == "cell_is" || val == "formula");
                                text_visible.set(val == "text");
                                format_visible.set(val != "color_scale");
                                selected_rule_type.set(val);
                            }
                        >
                            {RULE_TYPES.iter().map(|(val, label)| {
                                let val = val.to_string();
                                view! {
                                    <option value=val.clone() selected=move || selected_rule_type.get() == val>
                                        {label.to_string()}
                                    </option>
                                }
                            }).collect::<Vec<_>>()}
                        </select>
                    </div>

                    {/* Range input — ⊞ arms grid capture (sheet-relative B2:D8). */}
                    <div class="cfm-field">
                        <label class="cfm-label">"Range"</label>
                        <RangePickerInput
                            value=range_text
                            target=RangeCaptureTarget::CfRange
                            format=RangeFormat::SheetRelative
                            placeholder="e.g. A1:C10"
                        />
                        <Show when=move || {
                            state.range_capture.get() == Some(RangeCaptureTarget::CfRange)
                        }>
                            <p class="rp-hint">"Selecting on grid… click ⊞ or press Esc when done."</p>
                        </Show>
                    </div>

                    {/* Operator selector (CellIs only) */}
                    <Show when=move || operator_visible.get()>
                        <div class="cfm-field">
                            <label class="cfm-label">"Operator"</label>
                            <select
                                class="cfm-select"
                                on:change=move |ev| {
                                    let val = event_target_value(&ev);
                                    formula2_visible.set(val == "between" || val == "not_between");
                                    selected_operator.set(val);
                                }
                            >
                                {VALUE_OPERATORS.iter().map(|(val, label)| {
                                    let val = val.to_string();
                                    view! {
                                        <option value=val.clone() selected=move || selected_operator.get() == val>
                                            {label.to_string()}
                                        </option>
                                    }
                                }).collect::<Vec<_>>()}
                            </select>
                        </div>
                    </Show>

                    {/* Formula input */}
                    <Show when=move || formula_visible.get()>
                        <div class="cfm-field">
                            <label class="cfm-label">"Value / Formula"</label>
                            <div class="rp-wrap">
                                <FormulaField
                                    value=formula_text
                                    refs=formula_refs
                                    is_error=formula_is_error
                                    on_input=on_formula_input
                                    placeholder="e.g. 100 or =$B$1"
                                />
                                <button
                                    class=move || if cf_formula_armed() {
                                        "rp-arm rp-arm-active"
                                    } else {
                                        "rp-arm"
                                    }
                                    type="button"
                                    title="Insert range reference from grid"
                                    on:click=toggle_formula_arm
                                >
                                    "⊞"
                                </button>
                            </div>
                            <Show when=move || cf_formula_armed()>
                                <p class="rp-hint">"Selecting on grid… click ⊞ or press Esc when done."</p>
                            </Show>
                        </div>
                    </Show>

                    {/* Text value input (Text rule only) */}
                    <Show when=move || text_visible.get()>
                        <div class="cfm-field">
                            <label class="cfm-label">"Contains"</label>
                            <input
                                class="cfm-input"
                                type="text"
                                prop:value=text_value
                                on:input=move |ev| text_value.set(event_target_value(&ev))
                                placeholder="e.g. urgent"
                            />
                        </div>
                    </Show>

                    {/* Second formula (Between/NotBetween only) */}
                    <Show when=move || formula2_visible.get()>
                        <div class="cfm-field">
                            <label class="cfm-label">"And"</label>
                            <input
                                class="cfm-input"
                                type="text"
                                prop:value=formula2_text
                                on:input=move |ev| formula2_text.set(event_target_value(&ev))
                                placeholder="e.g. 200"
                            />
                        </div>
                    </Show>

                    {/* Format section */}
                    <Show when=move || format_visible.get()>
                        <div class="cfm-section">
                            <span class="cfm-section-title">"Format"</span>
                            <div class="cfm-field-row">
                                <label class="cfm-label">"Fill"</label>
                                <ColorPicker
                                    color_type=ColorType::Generic
                                    current_color=fill_color_hex
                                    on_color_change=on_fill_change
                                >
                                    <div
                                        class="cp-bar"
                                        style=move || match fill_color_hex.get() {
                                            Some(c) => format!("background-color: {};", c.as_str()),
                                            None => "background-color: transparent; border: 1px solid var(--border-color);".to_string(),
                                        }
                                    />
                                </ColorPicker>
                            </div>
                            <div class="cfm-field-row">
                                <label class="cfm-label">"Font"</label>
                                <ColorPicker
                                    color_type=ColorType::Generic
                                    current_color=font_color_hex
                                    on_color_change=on_font_change
                                >
                                    <div
                                        class="cp-bar"
                                        style=move || match font_color_hex.get() {
                                            Some(c) => format!("background-color: {};", c.as_str()),
                                            None => "background-color: transparent; border: 1px solid var(--border-color);".to_string(),
                                        }
                                    />
                                </ColorPicker>
                            </div>
                            <div class="cfm-field-row">
                                <label class="cfm-checkbox-label">
                                    <input
                                        type="checkbox"
                                        prop:checked=bold
                                        on:change=move |ev| bold.set(event_target_checked(&ev))
                                    />
                                    "Bold"
                                </label>
                            </div>
                            <div class="cfm-field-row">
                                <label class="cfm-checkbox-label">
                                    <input
                                        type="checkbox"
                                        prop:checked=stop_if_true
                                        on:change=move |ev| stop_if_true.set(event_target_checked(&ev))
                                    />
                                    "Stop If True"
                                </label>
                            </div>
                        </div>
                    </Show>

                    {/* Action buttons */}
                    <div class="cfm-editor-actions">
                        <button class="cfm-btn-save" on:click=save>"Save"</button>
                        <button class="cfm-btn-cancel" on:click=cancel>"Cancel"</button>
                    </div>
                </div>
            </Show>
        </div>
    }
}
