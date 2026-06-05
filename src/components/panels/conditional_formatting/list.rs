//! Conditional-formatting rule list (left column of the CF dialog).
//!
//! Lists the active sheet's CF rules; clicking a row loads it into the editor,
//! the trash icon deletes it, and "+ New Rule" seeds an empty edit. The list
//! recomputes whenever a `FormatEvent` fires (CF add/update/delete emit one),
//! which is also what drives the canvas repaint — one signal, both effects.

use ironcalc_base::cf_types::{CfRule, CfRuleInput, ConditionalFormatting, ValueOperator};
use ironcalc_base::types::Dxf;
use leptos::prelude::*;

use crate::events::{FormatEvent, SpreadsheetEvent};
use crate::model::{EvaluationMode, try_mutate};
use crate::state::{CfRuleEditState, ModelStore, WorkbookState};

/// Human-readable label for a CfRule variant.
fn rule_label(rule: &CfRule) -> &'static str {
    match rule {
        CfRule::ColorScale { .. } => "Color Scale",
        CfRule::CellIs { .. } => "Cell Value",
        CfRule::Formula { .. } => "Formula",
        CfRule::Text { .. } => "Text",
        CfRule::TimePeriod { .. } => "Date",
        CfRule::DuplicateValues { .. } => "Duplicates",
        CfRule::UniqueValues { .. } => "Unique",
        CfRule::Blanks { .. } => "Blanks",
        CfRule::NotBlanks { .. } => "Not Blanks",
        CfRule::Errors { .. } => "Errors",
        CfRule::NoErrors { .. } => "No Errors",
        CfRule::AboveAverage { .. } => "Above Avg",
        CfRule::BelowAverage { .. } => "Below Avg",
        CfRule::Top10 { .. } => "Top 10",
        CfRule::Bottom10 { .. } => "Bottom 10",
        CfRule::DataBar { .. } => "Data Bar",
        CfRule::IconSet { .. } => "Icon Set",
        CfRule::IconRating { .. } => "Rating",
    }
}

/// Convert a stored `CfRule` back to a `CfRuleInput` for editing.
///
/// IronCalc stores a `dxf_id` in place of the inline `Dxf`, so the rule itself
/// can't reconstruct its own format. The caller resolves the real format via
/// `get_dxf_for_conditional_formatting` and passes it in here — preserving the
/// user's fill/font when re-opening a rule (the old branch dropped it with
/// `Dxf::default()`).
fn cf_rule_to_input(rule: &CfRule, format: Dxf) -> CfRuleInput {
    match rule {
        CfRule::ColorScale { thresholds } => CfRuleInput::ColorScale {
            thresholds: thresholds.clone(),
        },
        CfRule::CellIs {
            operator,
            formula,
            formula2,
            stop_if_true,
            ..
        } => CfRuleInput::CellIs {
            operator: operator.clone(),
            formula: formula.clone(),
            formula2: formula2.clone(),
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::Formula {
            formula,
            stop_if_true,
            ..
        } => CfRuleInput::Formula {
            formula: formula.clone(),
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::Text {
            operator,
            value,
            stop_if_true,
            ..
        } => CfRuleInput::Text {
            operator: operator.clone(),
            value: value.clone(),
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::DuplicateValues { stop_if_true, .. } => CfRuleInput::DuplicateValues {
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::Blanks { stop_if_true, .. } => CfRuleInput::Blanks {
            format,
            stop_if_true: *stop_if_true,
        },
        // Fall back to a default rule for unsupported types.
        _ => CfRuleInput::CellIs {
            operator: ValueOperator::GreaterThan,
            formula: String::new(),
            formula2: None,
            format,
            stop_if_true: false,
        },
    }
}

#[component]
pub fn CfRuleList() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Recompute on every format event — CF add/update/delete each emit a
    // `FormatEvent::ConditionalFormattingChanged`, which replaces this signal
    // and re-runs the derive. Empty while the dialog is closed (not mounted).
    let rules: Signal<Vec<ConditionalFormatting>> = Signal::derive(move || {
        let _ = state.events.format.get();
        if !state.cf_dialog_open.get() {
            return Vec::new();
        }
        model.with_value(|m| {
            let sheet = m.get_selected_sheet();
            m.get_conditional_formatting_list(sheet).unwrap_or_default()
        })
    });

    let delete_rule = move |index: u32| {
        let _ = try_mutate(model, EvaluationMode::Immediate, |m| {
            let sheet = m.get_selected_sheet();
            m.delete_conditional_formatting(sheet, index)
        });
        state.editing_cf_rule.set(None);
        let sheet = model.with_value(|m| m.get_selected_sheet());
        state.emit_event(SpreadsheetEvent::Format(
            FormatEvent::ConditionalFormattingChanged { sheet },
        ));
    };

    let edit_rule = move |idx: usize, cf: &ConditionalFormatting| {
        // Resolve the rule's real differential format before editing — the
        // stored `CfRule` only carries a `dxf_id`.
        let format = model.with_value(|m| {
            let sheet = m.get_selected_sheet();
            m.get_dxf_for_conditional_formatting(sheet, idx as u32)
                .ok()
                .flatten()
                .unwrap_or_default()
        });
        let edit = CfRuleEditState {
            index: Some(idx as u32),
            range: cf.range.clone(),
            rule: cf_rule_to_input(&cf.cf_rule, format),
        };
        state.editing_cf_rule.set(Some(edit));
    };

    let new_rule = move |_: web_sys::MouseEvent| {
        let default = CfRuleEditState {
            index: None,
            range: String::new(),
            rule: CfRuleInput::CellIs {
                operator: ValueOperator::GreaterThan,
                formula: String::new(),
                formula2: None,
                format: Dxf::default(),
                stop_if_true: false,
            },
        };
        state.editing_cf_rule.set(Some(default));
    };

    view! {
        <div class="cfm-list">
            <div class="cfm-list-header">
                <span class="cfm-list-title">"Rules"</span>
                <button class="cfm-btn-new" on:click=new_rule>"+ New Rule"</button>
            </div>
            <div class="cfm-list-body">
                <For
                    each=move || rules.get()
                    key=|cf: &ConditionalFormatting| cf.priority
                    children=move |cf: ConditionalFormatting| {
                        let range = cf.range.clone();
                        let label = rule_label(&cf.cf_rule).to_string();
                        let priority = cf.priority;
                        let e = edit_rule;
                        let d = delete_rule;
                        let rules_snapshot = rules.get();
                        let idx = rules_snapshot
                            .iter()
                            .position(|r| r.priority == priority)
                            .unwrap_or(0);
                        let idx_u32 = idx as u32;
                        view! {
                            <div class="cfm-list-item" on:click=move |_| e(idx, &cf)>
                                <span class="cfm-item-label">{label}</span>
                                <span class="cfm-item-range">{range}</span>
                                <button
                                    class="cfm-item-delete"
                                    on:click=move |ev| {
                                        ev.stop_propagation();
                                        d(idx_u32);
                                    }
                                    title="Delete rule"
                                >
                                    "✕"
                                </button>
                            </div>
                        }
                    }
                />
            </div>
        </div>
    }
}
