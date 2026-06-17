//! Conditional-formatting rule list (left column of the CF dialog).
//!
//! Lists the active sheet's CF rules; clicking a row loads it into the editor,
//! the trash icon deletes it, and "+ New Rule" seeds an empty edit. The list
//! recomputes whenever a `FormatEvent` fires (CF add/update/delete emit one),
//! which is also what drives the canvas repaint — one signal, both effects.

use ironcalc_base::cf_types::{
    CfRule, CfRuleInput, ConditionalFormatting, PeriodType, ValueOperator,
};
use ironcalc_base::types::Dxf;
use leptos::prelude::*;

use crate::events::{FormatEvent, SpreadsheetEvent};
use crate::model::{EvaluationMode, try_mutate};
use crate::state::{ActiveDrawer, CfRuleEditState, ModelStore, StatusMessage, WorkbookState};

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

/// Convert a stored `CfRule` back to a `CfRuleInput` for editing, or `None`
/// for the one shape the editor can't represent (Between/NotBetween date
/// rules) — editing those would silently drop their dates.
///
/// IronCalc stores a `dxf_id` in place of the inline `Dxf`, so the rule itself
/// can't reconstruct its own format. The caller resolves the real format via
/// `get_dxf_for_conditional_formatting` and passes it in here — preserving the
/// user's fill/font when re-opening a rule (the old branch dropped it with
/// `Dxf::default()`).
fn cf_rule_to_input(rule: &CfRule, format: Dxf) -> Option<CfRuleInput> {
    Some(match rule {
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
        CfRule::UniqueValues { stop_if_true, .. } => CfRuleInput::UniqueValues {
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::NotBlanks { stop_if_true, .. } => CfRuleInput::NotBlanks {
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::Errors { stop_if_true, .. } => CfRuleInput::Errors {
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::NoErrors { stop_if_true, .. } => CfRuleInput::NoErrors {
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::AboveAverage { stop_if_true, .. } => CfRuleInput::AboveAverage {
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::BelowAverage { stop_if_true, .. } => CfRuleInput::BelowAverage {
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::Top10 {
            rank,
            percent,
            stop_if_true,
            ..
        } => CfRuleInput::Top10 {
            rank: *rank,
            percent: *percent,
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::Bottom10 {
            rank,
            percent,
            stop_if_true,
            ..
        } => CfRuleInput::Bottom10 {
            rank: *rank,
            percent: *percent,
            format,
            stop_if_true: *stop_if_true,
        },
        // Between/NotBetween date rules carry date1/date2 the editor has no
        // fields for (and the engine does not evaluate them yet) — guard them
        // like the other not-yet-editable types instead of dropping the dates.
        CfRule::TimePeriod {
            time_period: PeriodType::Between | PeriodType::NotBetween,
            ..
        } => return None,
        CfRule::TimePeriod {
            time_period,
            date1,
            date2,
            stop_if_true,
            ..
        } => CfRuleInput::TimePeriod {
            time_period: time_period.clone(),
            date1: date1.clone(),
            date2: date2.clone(),
            format,
            stop_if_true: *stop_if_true,
        },
        CfRule::DataBar {
            min,
            max,
            positive_color,
            negative_color,
            is_gradient,
            show_value,
        } => CfRuleInput::DataBar {
            min: min.clone(),
            max: max.clone(),
            positive_color: positive_color.clone(),
            negative_color: negative_color.clone(),
            is_gradient: *is_gradient,
            show_value: *show_value,
        },
        CfRule::IconSet {
            thresholds,
            show_value,
        } => CfRuleInput::IconSet {
            thresholds: thresholds.clone(),
            show_value: *show_value,
        },
        CfRule::IconRating {
            icon,
            color,
            thresholds,
            show_value,
        } => CfRuleInput::IconRating {
            icon: icon.clone(),
            color: color.clone(),
            thresholds: thresholds.clone(),
            show_value: *show_value,
        },
    })
}

#[component]
pub fn CfRuleList() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // The sheet the list is bound to. A Memo so it re-notifies only when the
    // active sheet actually changes — the navigation bus fires on every
    // selection/scroll, but `get_selected_sheet` dedups via PartialEq, so the
    // list won't rebuild on unrelated nav events. #18: the CF drawer is
    // non-modal, so the list and its delete/edit actions must track the active
    // sheet rather than read it lazily at click time.
    let active_sheet = Memo::new(move |_| {
        let _ = state.events.navigation.get();
        model.with_value(|m| m.get_selected_sheet())
    });

    // Recompute on every format event — CF add/update/delete each emit a
    // `FormatEvent::ConditionalFormattingChanged` — and whenever the active
    // sheet changes. Empty while the dialog is closed (not mounted).
    let rules: Signal<Vec<ConditionalFormatting>> = Signal::derive(move || {
        let _ = state.events.format.get();
        let sheet = active_sheet.get();
        if state.active_drawer.get() != Some(ActiveDrawer::ConditionalFormatting) {
            return Vec::new();
        }
        model.with_value(|m| m.get_conditional_formatting_list(sheet).unwrap_or_default())
    });

    let delete_rule = move |index: u32| {
        // Operate on the sheet the list is showing, not the live selection (#18).
        let sheet = active_sheet.get_untracked();
        let result = try_mutate(model, EvaluationMode::Immediate, |m| {
            m.delete_conditional_formatting(sheet, index)
        });
        match result {
            Ok(()) => {
                state.editing_cf_rule.set(None);
                state.emit_event(SpreadsheetEvent::Format(
                    FormatEvent::ConditionalFormattingChanged { sheet },
                ));
            }
            Err(e) => state.status.set(Some(StatusMessage::Error(e))),
        }
    };

    let edit_rule = move |idx: usize, cf: &ConditionalFormatting| {
        let sheet = active_sheet.get_untracked();
        // Resolve the rule's real differential format before editing — the
        // stored `CfRule` only carries a `dxf_id`.
        let format = model.with_value(|m| {
            m.get_dxf_for_conditional_formatting(sheet, idx as u32)
                .ok()
                .flatten()
                .unwrap_or_default()
        });
        let Some(rule) = cf_rule_to_input(&cf.cf_rule, format) else {
            state.status.set(Some(StatusMessage::Error(format!(
                "\"{}\" rules are not yet editable",
                rule_label(&cf.cf_rule)
            ))));
            return;
        };
        let edit = CfRuleEditState {
            sheet,
            index: Some(idx as u32),
            range: cf.range.clone(),
            rule,
        };
        state.editing_cf_rule.set(Some(edit));
    };

    let new_rule = move |_: web_sys::MouseEvent| {
        let default = CfRuleEditState {
            sheet: active_sheet.get_untracked(),
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
                        // The index is resolved at click time, not render time:
                        // the keyed <For> reuses DOM nodes after deletions, so a
                        // captured index (and `cf` snapshot) would go stale and
                        // target the wrong rule.
                        let find_rule = move || {
                            rules
                                .get_untracked()
                                .into_iter()
                                .enumerate()
                                .find(|(_, r)| r.priority == priority)
                        };
                        view! {
                            <div
                                class="cfm-list-item"
                                on:click=move |_| {
                                    if let Some((idx, rule)) = find_rule() {
                                        e(idx, &rule);
                                    }
                                }
                            >
                                <span class="cfm-item-label">{label}</span>
                                <span class="cfm-item-range">{range}</span>
                                <button
                                    class="cfm-item-delete"
                                    on:click=move |ev| {
                                        ev.stop_propagation();
                                        if let Some((idx, _)) = find_rule() {
                                            d(idx as u32);
                                        }
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
