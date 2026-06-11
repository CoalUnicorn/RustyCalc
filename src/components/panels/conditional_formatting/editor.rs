//! Conditional-formatting rule editor (right column of the CF dialog).
//!
//! Mirrors the in-progress [`CfRuleEditState`] into a set of local signals,
//! lets the user shape one rule (type, operator, formula, format), then writes
//! it back through `add_/update_conditional_formatting`. Save/Delete emit a
//! `FormatEvent::ConditionalFormattingChanged` so the canvas repaints and the
//! rule list refreshes.

use ironcalc_base::cf_types::{
    CfRuleInput, Cfvo, ColorScaleThreshold, Icon, IconThreshold, PeriodType, TextOperator,
    ValueOperator,
};
use ironcalc_base::types::{Color, Dxf, DxfFont, Fill};
use leptos::prelude::*;

use crate::components::ui::color_picker::{ColorPicker, ColorType};
use crate::components::ui::formula_field::FormulaField;
use crate::components::ui::range_picker::{RangeFormat, RangePickerInput};
use crate::coord::{CellAddress, TextRef, selection_a1_relative};
use crate::events::{FormatEvent, SpreadsheetEvent};
use crate::input::formula::{analyze_formula, splice_ref};
use crate::model::frontend_model::DefinedNameManager;
use crate::model::style_types::HexColor;
use crate::model::{EvaluationMode, SheetRoster, try_mutate};
use crate::state::{ModelStore, RangeCaptureTarget, StatusMessage, WorkbookState};

/// Seed a UI color picker from a CF rule color, resolved against the
/// workbook theme (`""` for `Color::None`). Saving writes back `Color::Rgb`,
/// so editing a theme-colored rule pins it to its current hex — same
/// behavior as Excel's picker.
fn color_to_str(model: ModelStore, c: &Color) -> String {
    model.with_value(|m| m.resolve_color(c))
}

const RULE_TYPES: &[(&str, &str)] = &[
    ("cell_is", "Cell Value"),
    ("formula", "Formula"),
    ("text", "Text"),
    ("color_scale", "Color Scale"),
    ("duplicate", "Duplicate Values"),
    ("unique", "Unique Values"),
    ("blanks", "Blanks"),
    ("not_blanks", "Not Blanks"),
    ("errors", "Errors"),
    ("no_errors", "No Errors"),
    ("above_average", "Above Average"),
    ("below_average", "Below Average"),
    ("top10", "Top 10"),
    ("bottom10", "Bottom 10"),
    ("time_period", "Date Occurring"),
    ("data_bar", "Data Bar"),
    ("icon_set", "Icon Set"),
    ("icon_rating", "Rating"),
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

const TEXT_OPERATORS: &[(&str, &str)] = &[
    ("contains", "Contains"),
    ("does_not_contain", "Does Not Contain"),
    ("begins_with", "Begins With"),
    ("ends_with", "Ends With"),
    ("equals", "Equals"),
];

fn text_operator_from_str(s: &str) -> TextOperator {
    match s {
        "does_not_contain" => TextOperator::DoesNotContain,
        "begins_with" => TextOperator::BeginsWith,
        "ends_with" => TextOperator::EndsWith,
        "equals" => TextOperator::Equals,
        _ => TextOperator::Contains,
    }
}

fn text_operator_to_str(op: &TextOperator) -> &'static str {
    match op {
        TextOperator::Contains => "contains",
        TextOperator::DoesNotContain => "does_not_contain",
        TextOperator::BeginsWith => "begins_with",
        TextOperator::EndsWith => "ends_with",
        TextOperator::Equals => "equals",
    }
}

// `PeriodType::Between` / `NotBetween` are omitted: the engine's
// `apply_cf_time_period` does not evaluate them yet, so offering them here
// would create rules that never paint. The list guards them as not-editable.
const PERIOD_TYPES: &[(&str, &str)] = &[
    ("yesterday", "Yesterday"),
    ("today", "Today"),
    ("tomorrow", "Tomorrow"),
    ("last_7_days", "Last 7 Days"),
    ("next_7_days", "Next 7 Days"),
    ("last_week", "Last Week"),
    ("this_week", "This Week"),
    ("next_week", "Next Week"),
    ("last_month", "Last Month"),
    ("this_month", "This Month"),
    ("next_month", "Next Month"),
    ("last_year", "Last Year"),
    ("this_year", "This Year"),
    ("next_year", "Next Year"),
];

fn period_from_str(s: &str) -> PeriodType {
    match s {
        "yesterday" => PeriodType::Yesterday,
        "tomorrow" => PeriodType::Tomorrow,
        "last_7_days" => PeriodType::Last7Days,
        "next_7_days" => PeriodType::Next7Days,
        "last_week" => PeriodType::LastWeek,
        "this_week" => PeriodType::ThisWeek,
        "next_week" => PeriodType::NextWeek,
        "last_month" => PeriodType::LastMonth,
        "this_month" => PeriodType::ThisMonth,
        "next_month" => PeriodType::NextMonth,
        "last_year" => PeriodType::LastYear,
        "this_year" => PeriodType::ThisYear,
        "next_year" => PeriodType::NextYear,
        _ => PeriodType::Today,
    }
}

fn period_to_str(p: &PeriodType) -> Option<&'static str> {
    match p {
        PeriodType::Yesterday => Some("yesterday"),
        PeriodType::Today => Some("today"),
        PeriodType::Tomorrow => Some("tomorrow"),
        PeriodType::Last7Days => Some("last_7_days"),
        PeriodType::Next7Days => Some("next_7_days"),
        PeriodType::LastWeek => Some("last_week"),
        PeriodType::ThisWeek => Some("this_week"),
        PeriodType::NextWeek => Some("next_week"),
        PeriodType::LastMonth => Some("last_month"),
        PeriodType::ThisMonth => Some("this_month"),
        PeriodType::NextMonth => Some("next_month"),
        PeriodType::LastYear => Some("last_year"),
        PeriodType::ThisYear => Some("this_year"),
        PeriodType::NextYear => Some("next_year"),
        PeriodType::Between | PeriodType::NotBetween => None,
    }
}

const ICONS: &[(&str, &str)] = &[
    ("arrow_up", "Arrow Up"),
    ("arrow_right", "Arrow Right"),
    ("arrow_down", "Arrow Down"),
    ("arrow_angle_up", "Arrow Angle Up"),
    ("arrow_angle_down", "Arrow Angle Down"),
    ("circle", "Circle"),
    ("triangle_up", "Triangle Up"),
    ("triangle_down", "Triangle Down"),
    ("triangle_up_filled", "Triangle Up Filled"),
    ("triangle_down_filled", "Triangle Down Filled"),
    ("flat_rectangle", "Flat Rectangle"),
    ("rhombus", "Rhombus"),
    ("flag", "Flag"),
    ("check", "Check"),
    ("cross", "Cross"),
    ("exclamation", "Exclamation"),
    ("star", "Star"),
    ("heart", "Heart"),
    ("thumbs_up", "Thumbs Up"),
    ("thumbs_down", "Thumbs Down"),
];

fn icon_from_str(s: &str) -> Icon {
    match s {
        "arrow_right" => Icon::ArrowRight,
        "arrow_down" => Icon::ArrowDown,
        "arrow_angle_up" => Icon::ArrowAngleUp,
        "arrow_angle_down" => Icon::ArrowAngleDown,
        "circle" => Icon::Circle,
        "triangle_up" => Icon::TriangleUp,
        "triangle_down" => Icon::TriangleDown,
        "triangle_up_filled" => Icon::TriangleUpFilled,
        "triangle_down_filled" => Icon::TriangleDownFilled,
        "flat_rectangle" => Icon::FlatRectangle,
        "rhombus" => Icon::Rhombus,
        "flag" => Icon::Flag,
        "check" => Icon::Check,
        "cross" => Icon::Cross,
        "exclamation" => Icon::Exclamation,
        "star" => Icon::Star,
        "heart" => Icon::Heart,
        "thumbs_up" => Icon::ThumbsUp,
        "thumbs_down" => Icon::ThumbsDown,
        _ => Icon::ArrowUp,
    }
}

fn icon_to_str(icon: &Icon) -> &'static str {
    match icon {
        Icon::ArrowUp => "arrow_up",
        Icon::ArrowRight => "arrow_right",
        Icon::ArrowDown => "arrow_down",
        Icon::ArrowAngleUp => "arrow_angle_up",
        Icon::ArrowAngleDown => "arrow_angle_down",
        Icon::Circle => "circle",
        Icon::TriangleUp => "triangle_up",
        Icon::TriangleDown => "triangle_down",
        Icon::TriangleUpFilled => "triangle_up_filled",
        Icon::TriangleDownFilled => "triangle_down_filled",
        Icon::FlatRectangle => "flat_rectangle",
        Icon::Rhombus => "rhombus",
        Icon::Flag => "flag",
        Icon::Check => "check",
        Icon::Cross => "cross",
        Icon::Exclamation => "exclamation",
        Icon::Star => "star",
        Icon::Heart => "heart",
        Icon::ThumbsUp => "thumbs_up",
        Icon::ThumbsDown => "thumbs_down",
    }
}

fn cfvo_from_ui(kind: &str, value: &str) -> Option<Cfvo> {
    match kind {
        "min" => Some(Cfvo::Min),
        "max" => Some(Cfvo::Max),
        "number" => Some(Cfvo::Number(value.trim().parse().unwrap_or(0.0))),
        "percent" => Some(Cfvo::Percent(value.trim().parse().unwrap_or(0.0))),
        "percentile" => Some(Cfvo::Percentile(value.trim().parse().unwrap_or(50.0))),
        "formula" => Some(Cfvo::Formula(value.to_string())),
        _ => None, // "auto"
    }
}

fn cfvo_to_ui(cfvo: &Option<Cfvo>) -> (&'static str, String) {
    match cfvo {
        None => ("auto", String::new()),
        Some(Cfvo::Min) => ("min", String::new()),
        Some(Cfvo::Max) => ("max", String::new()),
        Some(Cfvo::Number(n)) => ("number", n.to_string()),
        Some(Cfvo::Percent(p)) => ("percent", p.to_string()),
        Some(Cfvo::Percentile(p)) => ("percentile", p.to_string()),
        Some(Cfvo::Formula(f)) => ("formula", f.clone()),
    }
}

/// Bridge a hex-string color signal to the `ColorPicker`'s `HexColor` API.
/// An empty string means "no color"; saving reads it back as `Color::None`.
fn hex_bridge(sig: RwSignal<String>) -> (Signal<Option<HexColor>>, Callback<Option<HexColor>>) {
    let hex = Signal::derive(move || {
        let s = sig.get();
        if s.is_empty() { None } else { HexColor::new(s).ok() }
    });
    let on_change = Callback::new(move |c: Option<HexColor>| {
        sig.set(c.map(|h| h.as_str().to_owned()).unwrap_or_default());
    });
    (hex, on_change)
}

/// One icon-set threshold row, lowest value first (the engine picks the last
/// row whose threshold the cell value exceeds). `strict` mirrors the engine's
/// inclusive flag: true ⇒ the icon applies at `>=`, false at `>`.
#[derive(Clone, Copy, PartialEq)]
struct IconRow {
    id: usize,
    icon: RwSignal<String>,
    kind: RwSignal<String>,
    value: RwSignal<String>,
    color: RwSignal<String>,
    strict: RwSignal<bool>,
}

/// One Cfvo threshold slot: kind select + value field (hidden for the fixed
/// kinds). `endpoint` is the rule-end this slot can pin ("min" offers
/// "Lowest Value", "max" offers "Highest Value"); "auto" maps to `None` and
/// is omitted where the schema requires a concrete Cfvo (icon thresholds).
#[component]
fn CfvoField(
    label: &'static str,
    endpoint: &'static str,
    kind: RwSignal<String>,
    value: RwSignal<String>,
    #[prop(default = true)] allow_auto: bool,
) -> impl IntoView {
    let endpoint_label = if endpoint == "min" {
        "Lowest Value"
    } else {
        "Highest Value"
    };
    let mut options: Vec<(&'static str, &'static str)> = vec![
        (endpoint, endpoint_label),
        ("number", "Number"),
        ("percent", "Percent"),
        ("percentile", "Percentile"),
        ("formula", "Formula"),
    ];
    if allow_auto {
        options.insert(0, ("auto", "Automatic"));
    }
    let value_visible = move || !matches!(kind.get().as_str(), "auto" | "min" | "max");
    view! {
        <div class="cfm-field">
            <label class="cfm-label">{label}</label>
            <select
                class="cfm-select"
                on:change=move |ev| kind.set(event_target_value(&ev))
            >
                {options.into_iter().map(|(val, lab)| {
                    let val = val.to_string();
                    view! {
                        <option value=val.clone() selected=move || kind.get() == val>
                            {lab.to_string()}
                        </option>
                    }
                }).collect::<Vec<_>>()}
            </select>
            <Show when=value_visible>
                <input
                    class="cfm-input"
                    type="text"
                    prop:value=value
                    on:input=move |ev| value.set(event_target_value(&ev))
                    placeholder="e.g. 50"
                />
            </Show>
        </div>
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
    let text_operator = RwSignal::new("contains".to_string());
    let rank_text = RwSignal::new("10".to_string());
    let percent = RwSignal::new(false);
    let period = RwSignal::new("today".to_string());
    // Excel's default data-bar blue / negative red.
    let bar_pos_color = RwSignal::new("#638ec6".to_string());
    let bar_neg_color = RwSignal::new("#ff0000".to_string());
    let bar_gradient = RwSignal::new(true);
    let bar_show_value = RwSignal::new(true);
    let bar_min_kind = RwSignal::new("auto".to_string());
    let bar_min_value = RwSignal::new(String::new());
    let bar_max_kind = RwSignal::new("auto".to_string());
    let bar_max_value = RwSignal::new(String::new());
    // Icon-set rows need stable keys across add/remove, so each allocation
    // takes a fresh id from this counter.
    let icon_row_seq = StoredValue::new(0usize);
    let alloc_icon_row =
        move |icon: &str, kind: &str, value: &str, color: &str, strict: bool| -> IconRow {
            let id = icon_row_seq.get_value();
            icon_row_seq.set_value(id + 1);
            IconRow {
                id,
                icon: RwSignal::new(icon.to_string()),
                kind: RwSignal::new(kind.to_string()),
                value: RwSignal::new(value.to_string()),
                color: RwSignal::new(color.to_string()),
                strict: RwSignal::new(strict),
            }
        };
    // Excel's default 3-arrow set, lowest bucket first.
    let icon_rows = RwSignal::new(vec![
        alloc_icon_row("arrow_down", "percent", "0", "#e53935", true),
        alloc_icon_row("arrow_right", "percent", "33", "#fb8c00", true),
        alloc_icon_row("arrow_up", "percent", "67", "#43a047", true),
    ]);
    let icon_show_value = RwSignal::new(true);
    let rating_icon = RwSignal::new("star".to_string());
    let rating_color = RwSignal::new("#fbc02d".to_string());
    let rating_max = RwSignal::new("5".to_string());
    // Original thresholds of the rule being edited; reused on save while the
    // count is unchanged so imported custom stops survive a round-trip.
    let rating_thresholds = RwSignal::new(Vec::<(Cfvo, bool)>::new());
    let stop_if_true = RwSignal::new(false);
    let fill_color = RwSignal::new("#a5d6a7".to_string());
    let font_color = RwSignal::new("#000000".to_string());
    let bold = RwSignal::new(false);
    let operator_visible = RwSignal::new(true);
    let formula2_visible = RwSignal::new(false);
    let formula_visible = RwSignal::new(true);
    let text_visible = RwSignal::new(false);
    let format_visible = RwSignal::new(true);
    // Sections introduced after the toggle pattern above derive straight from
    // the rule type, so existing arms can't leave them stale.
    let rank_visible = Signal::derive(move || {
        matches!(selected_rule_type.get().as_str(), "top10" | "bottom10")
    });
    let period_visible = Signal::derive(move || selected_rule_type.get() == "time_period");
    let bar_visible = Signal::derive(move || selected_rule_type.get() == "data_bar");
    let icon_set_visible = Signal::derive(move || selected_rule_type.get() == "icon_set");
    let rating_visible = Signal::derive(move || selected_rule_type.get() == "icon_rating");

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
    let (bar_pos_hex, on_bar_pos) = hex_bridge(bar_pos_color);
    let (bar_neg_hex, on_bar_neg) = hex_bridge(bar_neg_color);
    let (rating_hex, on_rating_color) = hex_bridge(rating_color);

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
        analyzer_ctx.with(|(sheet_names, defined_names, ctx)| {
            analyze_formula(&text, *ctx, sheet_names, defined_names)
        })
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

    let cf_formula_armed = move || state.range_capture.get() == Some(RangeCaptureTarget::CfFormula);
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

    let restore_format = move |format: &Dxf| {
        fill_color.set(
            format
                .fill
                .as_ref()
                .map(|f| color_to_str(model, &f.color))
                .unwrap_or_default(),
        );
        font_color.set(
            format
                .font
                .as_ref()
                .map(|f| color_to_str(model, &f.color))
                .unwrap_or_default(),
        );
        bold.set(format.font.as_ref().and_then(|f| f.b).unwrap_or(false));
    };

    // Zero-parameter rules share one editor shape: Format + Stop If True only.
    let seed_format_only = move |kind: &str, format: &Dxf, s: bool| {
        selected_rule_type.set(kind.into());
        operator_visible.set(false);
        formula2_visible.set(false);
        formula_visible.set(false);
        text_visible.set(false);
        format_visible.set(true);
        restore_format(format);
        stop_if_true.set(s);
    };

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
                    restore_format(format);
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
                    restore_format(format);
                    stop_if_true.set(*s);
                }
                CfRuleInput::Text {
                    operator,
                    value,
                    format,
                    stop_if_true: s,
                    ..
                } => {
                    selected_rule_type.set("text".into());
                    text_operator.set(text_operator_to_str(operator).into());
                    text_value.set(value.clone());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(false);
                    text_visible.set(true);
                    format_visible.set(true);
                    restore_format(format);
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
                    format,
                    stop_if_true: s,
                    ..
                } => {
                    selected_rule_type.set("duplicate".into());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(false);
                    text_visible.set(false);
                    format_visible.set(true);
                    restore_format(format);
                    stop_if_true.set(*s);
                }
                CfRuleInput::Blanks {
                    format,
                    stop_if_true: s,
                    ..
                } => {
                    selected_rule_type.set("blanks".into());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(false);
                    text_visible.set(false);
                    format_visible.set(true);
                    restore_format(format);
                    stop_if_true.set(*s);
                }
                CfRuleInput::UniqueValues {
                    format,
                    stop_if_true: s,
                    ..
                } => seed_format_only("unique", format, *s),
                CfRuleInput::NotBlanks {
                    format,
                    stop_if_true: s,
                    ..
                } => seed_format_only("not_blanks", format, *s),
                CfRuleInput::Errors {
                    format,
                    stop_if_true: s,
                    ..
                } => seed_format_only("errors", format, *s),
                CfRuleInput::NoErrors {
                    format,
                    stop_if_true: s,
                    ..
                } => seed_format_only("no_errors", format, *s),
                CfRuleInput::AboveAverage {
                    format,
                    stop_if_true: s,
                    ..
                } => seed_format_only("above_average", format, *s),
                CfRuleInput::BelowAverage {
                    format,
                    stop_if_true: s,
                    ..
                } => seed_format_only("below_average", format, *s),
                CfRuleInput::Top10 {
                    rank,
                    percent: p,
                    format,
                    stop_if_true: s,
                    ..
                } => {
                    seed_format_only("top10", format, *s);
                    rank_text.set(rank.to_string());
                    percent.set(*p);
                }
                CfRuleInput::Bottom10 {
                    rank,
                    percent: p,
                    format,
                    stop_if_true: s,
                    ..
                } => {
                    seed_format_only("bottom10", format, *s);
                    rank_text.set(rank.to_string());
                    percent.set(*p);
                }
                CfRuleInput::TimePeriod {
                    time_period,
                    format,
                    stop_if_true: s,
                    ..
                } => {
                    seed_format_only("time_period", format, *s);
                    if let Some(p) = period_to_str(time_period) {
                        period.set(p.into());
                    }
                }
                CfRuleInput::DataBar {
                    min,
                    max,
                    positive_color,
                    negative_color,
                    is_gradient,
                    show_value,
                } => {
                    selected_rule_type.set("data_bar".into());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(false);
                    text_visible.set(false);
                    format_visible.set(false);
                    bar_pos_color.set(color_to_str(model, positive_color));
                    bar_neg_color.set(color_to_str(model, negative_color));
                    bar_gradient.set(*is_gradient);
                    bar_show_value.set(*show_value);
                    let (kind, value) = cfvo_to_ui(min);
                    bar_min_kind.set(kind.into());
                    bar_min_value.set(value);
                    let (kind, value) = cfvo_to_ui(max);
                    bar_max_kind.set(kind.into());
                    bar_max_value.set(value);
                }
                CfRuleInput::IconSet {
                    thresholds,
                    show_value,
                } => {
                    selected_rule_type.set("icon_set".into());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(false);
                    text_visible.set(false);
                    format_visible.set(false);
                    icon_show_value.set(*show_value);
                    icon_rows.set(
                        thresholds
                            .iter()
                            .map(|t| {
                                let (kind, value) = cfvo_to_ui(&Some(t.cfvo.clone()));
                                alloc_icon_row(
                                    icon_to_str(&t.icon),
                                    kind,
                                    &value,
                                    &color_to_str(model, &t.color),
                                    t.is_strict,
                                )
                            })
                            .collect(),
                    );
                }
                CfRuleInput::IconRating {
                    icon,
                    color,
                    thresholds,
                    show_value,
                } => {
                    selected_rule_type.set("icon_rating".into());
                    operator_visible.set(false);
                    formula2_visible.set(false);
                    formula_visible.set(false);
                    text_visible.set(false);
                    format_visible.set(false);
                    rating_icon.set(icon_to_str(icon).into());
                    rating_color.set(color_to_str(model, color));
                    rating_max.set(thresholds.len().to_string());
                    rating_thresholds.set(thresholds.clone());
                    icon_show_value.set(*show_value);
                }
            }
        }
    });

    // `None` for a rule-type key without a build arm — a state that only a
    // RULE_TYPES/build mismatch can produce. Surfacing it beats the old
    // fallback, which silently saved a blank CellIs over the user's rule.
    let build_rule = move || -> Option<CfRuleInput> {
        let format = Dxf {
            font: if bold.get() || !font_color.get().is_empty() {
                let mut f = DxfFont::default();
                if bold.get() {
                    f.b = Some(true);
                }
                if !font_color.get().is_empty() {
                    f.color = Color::from_rgb(&font_color.get()).unwrap_or(Color::None);
                }
                Some(f)
            } else {
                None
            },
            fill: if fill_color.get().is_empty() {
                None
            } else {
                Some(Fill {
                    color: Color::from_rgb(&fill_color.get()).unwrap_or(Color::None),
                })
            },
            ..Default::default()
        };

        Some(match selected_rule_type.get().as_str() {
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
                operator: text_operator_from_str(&text_operator.get()),
                value: text_value.get(),
                format,
                stop_if_true: stop_if_true.get(),
            },
            "color_scale" => CfRuleInput::ColorScale {
                thresholds: vec![
                    ColorScaleThreshold {
                        cfvo: Cfvo::Min,
                        color: Color::Rgb("#63be7b".into()),
                    },
                    ColorScaleThreshold {
                        cfvo: Cfvo::Max,
                        color: Color::Rgb("#f8696b".into()),
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
            "unique" => CfRuleInput::UniqueValues {
                format,
                stop_if_true: stop_if_true.get(),
            },
            "not_blanks" => CfRuleInput::NotBlanks {
                format,
                stop_if_true: stop_if_true.get(),
            },
            "errors" => CfRuleInput::Errors {
                format,
                stop_if_true: stop_if_true.get(),
            },
            "no_errors" => CfRuleInput::NoErrors {
                format,
                stop_if_true: stop_if_true.get(),
            },
            "above_average" => CfRuleInput::AboveAverage {
                format,
                stop_if_true: stop_if_true.get(),
            },
            "below_average" => CfRuleInput::BelowAverage {
                format,
                stop_if_true: stop_if_true.get(),
            },
            "top10" => CfRuleInput::Top10 {
                rank: rank_text.get().trim().parse().unwrap_or(10),
                percent: percent.get(),
                format,
                stop_if_true: stop_if_true.get(),
            },
            "bottom10" => CfRuleInput::Bottom10 {
                rank: rank_text.get().trim().parse().unwrap_or(10),
                percent: percent.get(),
                format,
                stop_if_true: stop_if_true.get(),
            },
            "time_period" => CfRuleInput::TimePeriod {
                time_period: period_from_str(&period.get()),
                date1: None,
                date2: None,
                format,
                stop_if_true: stop_if_true.get(),
            },
            "data_bar" => CfRuleInput::DataBar {
                min: cfvo_from_ui(&bar_min_kind.get(), &bar_min_value.get()),
                max: cfvo_from_ui(&bar_max_kind.get(), &bar_max_value.get()),
                // Empty picker → Color::None → renderer's default bar colors.
                positive_color: Color::from_rgb(&bar_pos_color.get()).unwrap_or(Color::None),
                negative_color: Color::from_rgb(&bar_neg_color.get()).unwrap_or(Color::None),
                is_gradient: bar_gradient.get(),
                show_value: bar_show_value.get(),
            },
            "icon_set" => CfRuleInput::IconSet {
                thresholds: icon_rows
                    .get()
                    .iter()
                    .map(|r| IconThreshold {
                        icon: icon_from_str(&r.icon.get()),
                        cfvo: cfvo_from_ui(&r.kind.get(), &r.value.get())
                            .unwrap_or(Cfvo::Percent(0.0)),
                        color: Color::from_rgb(&r.color.get()).unwrap_or(Color::None),
                        is_strict: r.strict.get(),
                    })
                    .collect(),
                show_value: icon_show_value.get(),
            },
            "icon_rating" => {
                let max: usize = rating_max.get().trim().parse().unwrap_or(5).clamp(1, 10);
                let stored = rating_thresholds.get();
                // Keep the loaded thresholds while the count is unchanged;
                // regenerate evenly spaced stops only when `max` moved.
                let thresholds = if stored.len() == max {
                    stored
                } else {
                    (0..max)
                        .map(|i| (Cfvo::Percent(100.0 * i as f64 / max as f64), true))
                        .collect()
                };
                CfRuleInput::IconRating {
                    icon: icon_from_str(&rating_icon.get()),
                    color: Color::from_rgb(&rating_color.get()).unwrap_or(Color::None),
                    thresholds,
                    show_value: icon_show_value.get(),
                }
            }
            _ => return None,
        })
    };

    let save = move |_: web_sys::MouseEvent| {
        let Some(edit) = state.editing_cf_rule.get() else {
            return;
        };
        let Some(rule) = build_rule() else {
            state.status.set(Some(StatusMessage::Error(format!(
                "No rule builder for type \"{}\"",
                selected_rule_type.get()
            ))));
            return;
        };
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

        match result {
            Ok(()) => {
                state.editing_cf_rule.set(None);
                let sheet = model.with_value(|m| m.get_selected_sheet());
                state.emit_event(SpreadsheetEvent::Format(
                    FormatEvent::ConditionalFormattingChanged { sheet },
                ));
            }
            // Editor stays open with its data intact so the user can correct
            // the rule instead of losing it.
            Err(e) => state.status.set(Some(StatusMessage::Error(e))),
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
                                format_visible.set(!matches!(
                                    val.as_str(),
                                    "color_scale" | "data_bar" | "icon_set" | "icon_rating"
                                ));
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

                    {/* Text operator + value (Text rule only) */}
                    <Show when=move || text_visible.get()>
                        <div class="cfm-field">
                            <label class="cfm-label">"Operator"</label>
                            <select
                                class="cfm-select"
                                on:change=move |ev| text_operator.set(event_target_value(&ev))
                            >
                                {TEXT_OPERATORS.iter().map(|(val, label)| {
                                    let val = val.to_string();
                                    view! {
                                        <option value=val.clone() selected=move || text_operator.get() == val>
                                            {label.to_string()}
                                        </option>
                                    }
                                }).collect::<Vec<_>>()}
                            </select>
                        </div>
                        <div class="cfm-field">
                            <label class="cfm-label">"Value"</label>
                            <input
                                class="cfm-input"
                                type="text"
                                prop:value=text_value
                                on:input=move |ev| text_value.set(event_target_value(&ev))
                                placeholder="e.g. urgent"
                            />
                        </div>
                    </Show>

                    {/* Rank + percent (Top10 / Bottom10 only) */}
                    <Show when=move || rank_visible.get()>
                        <div class="cfm-field">
                            <label class="cfm-label">"Rank"</label>
                            <input
                                class="cfm-input"
                                type="number"
                                min="1"
                                prop:value=rank_text
                                on:input=move |ev| rank_text.set(event_target_value(&ev))
                            />
                        </div>
                        <div class="cfm-field-row">
                            <label class="cfm-checkbox-label">
                                <input
                                    type="checkbox"
                                    prop:checked=percent
                                    on:change=move |ev| percent.set(event_target_checked(&ev))
                                />
                                "% of range"
                            </label>
                        </div>
                    </Show>

                    {/* Period selector (Date Occurring only) */}
                    <Show when=move || period_visible.get()>
                        <div class="cfm-field">
                            <label class="cfm-label">"Period"</label>
                            <select
                                class="cfm-select"
                                on:change=move |ev| period.set(event_target_value(&ev))
                            >
                                {PERIOD_TYPES.iter().map(|(val, label)| {
                                    let val = val.to_string();
                                    view! {
                                        <option value=val.clone() selected=move || period.get() == val>
                                            {label.to_string()}
                                        </option>
                                    }
                                }).collect::<Vec<_>>()}
                            </select>
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

                    {/* Data bar settings (DataBar only) */}
                    <Show when=move || bar_visible.get()>
                        <div class="cfm-section">
                            <span class="cfm-section-title">"Data Bar"</span>
                            <div class="cfm-field-row">
                                <label class="cfm-label">"Positive"</label>
                                <ColorPicker
                                    color_type=ColorType::Generic
                                    current_color=bar_pos_hex
                                    on_color_change=on_bar_pos
                                >
                                    <div
                                        class="cp-bar"
                                        style=move || match bar_pos_hex.get() {
                                            Some(c) => format!("background-color: {};", c.as_str()),
                                            None => "background-color: transparent; border: 1px solid var(--border-color);".to_string(),
                                        }
                                    />
                                </ColorPicker>
                            </div>
                            <div class="cfm-field-row">
                                <label class="cfm-label">"Negative"</label>
                                <ColorPicker
                                    color_type=ColorType::Generic
                                    current_color=bar_neg_hex
                                    on_color_change=on_bar_neg
                                >
                                    <div
                                        class="cp-bar"
                                        style=move || match bar_neg_hex.get() {
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
                                        prop:checked=bar_gradient
                                        on:change=move |ev| bar_gradient.set(event_target_checked(&ev))
                                    />
                                    "Gradient fill"
                                </label>
                            </div>
                            <div class="cfm-field-row">
                                <label class="cfm-checkbox-label">
                                    <input
                                        type="checkbox"
                                        prop:checked=bar_show_value
                                        on:change=move |ev| bar_show_value.set(event_target_checked(&ev))
                                    />
                                    "Show value"
                                </label>
                            </div>
                            <CfvoField label="Min" endpoint="min" kind=bar_min_kind value=bar_min_value />
                            <CfvoField label="Max" endpoint="max" kind=bar_max_kind value=bar_max_value />
                        </div>
                    </Show>

                    {/* Icon set thresholds (IconSet only), lowest bucket first */}
                    <Show when=move || icon_set_visible.get()>
                        <div class="cfm-section">
                            <span class="cfm-section-title">"Icon Set (low → high)"</span>
                            <For
                                each=move || icon_rows.get()
                                key=|row: &IconRow| row.id
                                children=move |row: IconRow| {
                                    let (row_hex, on_row_color) = hex_bridge(row.color);
                                    let id = row.id;
                                    view! {
                                        <div class="cfm-field-row">
                                            <select
                                                class="cfm-select"
                                                on:change=move |ev| row.icon.set(event_target_value(&ev))
                                            >
                                                {ICONS.iter().map(|(val, label)| {
                                                    let val = val.to_string();
                                                    view! {
                                                        <option value=val.clone() selected=move || row.icon.get() == val>
                                                            {label.to_string()}
                                                        </option>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </select>
                                            <select
                                                class="cfm-select"
                                                on:change=move |ev| row.strict.set(event_target_value(&ev) == "gte")
                                            >
                                                <option value="gte" selected=move || row.strict.get()>"≥"</option>
                                                <option value="gt" selected=move || !row.strict.get()>">"</option>
                                            </select>
                                            <ColorPicker
                                                color_type=ColorType::Generic
                                                current_color=row_hex
                                                on_color_change=on_row_color
                                            >
                                                <div
                                                    class="cp-bar"
                                                    style=move || match row_hex.get() {
                                                        Some(c) => format!("background-color: {};", c.as_str()),
                                                        None => "background-color: transparent; border: 1px solid var(--border-color);".to_string(),
                                                    }
                                                />
                                            </ColorPicker>
                                            <button
                                                class="cfm-btn-cancel"
                                                type="button"
                                                title="Remove threshold"
                                                on:click=move |_| icon_rows.update(|rows| {
                                                    if rows.len() > 1 {
                                                        rows.retain(|r| r.id != id);
                                                    }
                                                })
                                            >
                                                "✕"
                                            </button>
                                        </div>
                                        <CfvoField
                                            label="Threshold"
                                            endpoint="min"
                                            kind=row.kind
                                            value=row.value
                                            allow_auto=false
                                        />
                                    }
                                }
                            />
                            <button
                                class="cfm-btn-new"
                                type="button"
                                on:click=move |_| icon_rows.update(|rows| {
                                    rows.push(alloc_icon_row("arrow_up", "percent", "50", "", true));
                                })
                            >
                                "+ Add Threshold"
                            </button>
                            <div class="cfm-field-row">
                                <label class="cfm-checkbox-label">
                                    <input
                                        type="checkbox"
                                        prop:checked=icon_show_value
                                        on:change=move |ev| icon_show_value.set(event_target_checked(&ev))
                                    />
                                    "Show value"
                                </label>
                            </div>
                        </div>
                    </Show>

                    {/* Rating settings (IconRating only) */}
                    <Show when=move || rating_visible.get()>
                        <div class="cfm-section">
                            <span class="cfm-section-title">"Rating"</span>
                            <div class="cfm-field">
                                <label class="cfm-label">"Icon"</label>
                                <select
                                    class="cfm-select"
                                    on:change=move |ev| rating_icon.set(event_target_value(&ev))
                                >
                                    {ICONS.iter().map(|(val, label)| {
                                        let val = val.to_string();
                                        view! {
                                            <option value=val.clone() selected=move || rating_icon.get() == val>
                                                {label.to_string()}
                                            </option>
                                        }
                                    }).collect::<Vec<_>>()}
                                </select>
                            </div>
                            <div class="cfm-field-row">
                                <label class="cfm-label">"Color"</label>
                                <ColorPicker
                                    color_type=ColorType::Generic
                                    current_color=rating_hex
                                    on_color_change=on_rating_color
                                >
                                    <div
                                        class="cp-bar"
                                        style=move || match rating_hex.get() {
                                            Some(c) => format!("background-color: {};", c.as_str()),
                                            None => "background-color: transparent; border: 1px solid var(--border-color);".to_string(),
                                        }
                                    />
                                </ColorPicker>
                            </div>
                            <div class="cfm-field">
                                <label class="cfm-label">"Max"</label>
                                <input
                                    class="cfm-input"
                                    type="number"
                                    min="1"
                                    max="10"
                                    prop:value=rating_max
                                    on:input=move |ev| rating_max.set(event_target_value(&ev))
                                />
                            </div>
                            <div class="cfm-field-row">
                                <label class="cfm-checkbox-label">
                                    <input
                                        type="checkbox"
                                        prop:checked=icon_show_value
                                        on:change=move |ev| icon_show_value.set(event_target_checked(&ev))
                                    />
                                    "Show value"
                                </label>
                            </div>
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
