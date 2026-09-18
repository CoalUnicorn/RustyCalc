//! Performance inspector: the floating window, its capture controls, and the
//! view switch.
//!
//! This module is the composition root. It reads the store and the two pure
//! adapters, formats every string a view shows, and owns the selection. It
//! never touches the canvas: every capture action publishes intent through
//! [`CaptureCmd`], which one worksheet coordinator drains.
//!
//! The views under this module take resolved data and callbacks. None of them
//! reads `AppState`, and none of them formats an address.

mod attempts;
mod details;
mod digest;
mod json;
mod tools;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use attempts::{AttemptRow, InspectorAttempts};
use details::{AdvancedSection, DetailsView, EvidenceRow, InspectorDetails};
use digest::{DigestPanel, InspectorDigest};
use json::{InspectorJson, JsonView};
use tools::InspectorTools;

use crate::app_state::{AppState, CaptureCmd};
use crate::components::ui::floating_window::FloatingWindow;
use crate::perf::{
    AttemptKey, AttemptRecord, CaptureRecord, CaptureState, DigestFilter, EvidenceNote,
    EvidencePrecision, SheetRef, UnavailableReason, attempt_evidence, attempt_json, digest,
    format_range,
};
use iron_canvas_core::chrome::PaneRegion;
use iron_canvas_core::geometry::prim::Axis;
use iron_canvas_core::renderer::diag::{
    DiagBlit, DiagCacheActionTag, DiagCacheResolution, DiagFingerprintActionTag,
};
use iron_canvas_core::{FrameOutcome, PixelRect, RCRange, RenderStrategy, WorkFlags};

/// Which inspector view is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum InspectorView {
    #[default]
    Digest,
    Attempts,
    Details,
    Json,
    Tools,
}

impl InspectorView {
    const ALL: [InspectorView; 5] = [
        InspectorView::Digest,
        InspectorView::Attempts,
        InspectorView::Details,
        InspectorView::Json,
        InspectorView::Tools,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Digest => "Digest",
            Self::Attempts => "Attempts",
            Self::Details => "Details",
            Self::Json => "JSON",
            Self::Tools => "Tools",
        }
    }
}

/// The dev-tools performance inspector.
///
/// A non-modal floating window beside the worksheet. It presents the capture
/// store and the digest, and it publishes capture intent; nothing here paints,
/// measures, or borrows the canvas.
#[component]
pub fn PerfPanel() -> impl IntoView {
    let app = expect_context::<AppState>();
    let store = app.perf_store;
    let perf = app.perf;

    let view = RwSignal::new(InspectorView::default());
    // `None` follows the newest attempt. A row click pins a key, and the pin
    // survives new attempts arriving.
    let selection = RwSignal::new(None::<AttemptKey>);
    let follow_latest = RwSignal::new(false);
    let filter_include_forced = RwSignal::new(false);
    let filter_effective = RwSignal::new(None::<RenderStrategy>);

    let status = Memo::new(move |_| store.status());
    let state = Memo::new(move |_| store.state());
    let error = Memo::new(move |_| store.error());

    // The capture slot and roster, re-read whenever the archive changes.
    let captures = Memo::new(move |_| {
        let _ = store.revision();
        (store.capture_summaries(), store.selected())
    });

    // The attempt every per-attempt view shows. A pinned key that no longer
    // exists falls back to the newest attempt rather than blanking the views.
    let effective_key = Memo::new(move |_| {
        let _ = store.revision();
        let latest = || {
            store
                .with_selected(|capture| capture.attempts.last().map(|attempt| attempt.key))
                .flatten()
        };
        if follow_latest.get() {
            return latest();
        }
        match selection.get() {
            Some(key)
                if store
                    .with_selected(|capture| {
                        capture.attempts.iter().any(|attempt| attempt.key == key)
                    })
                    .unwrap_or(false) =>
            {
                Some(key)
            }
            _ => latest(),
        }
    });

    let digest_panel = Memo::new(move |_| {
        let _ = store.revision();
        let filter = DigestFilter {
            include_forced_baseline: filter_include_forced.get(),
            outcome: None,
            effective: filter_effective.get(),
        };
        DigestPanel {
            capture_name: store.with_selected(|capture| capture.name.clone()),
            digest: store.with_selected(|capture| digest(capture, &filter, crate::perf::now())),
            include_forced: filter.include_forced_baseline,
            effective: filter.effective,
            live_mutation: perf.mutation.get(),
            live_render: perf.render_call.get(),
        }
    });

    let attempt_rows = Memo::new(move |_| {
        let _ = store.revision();
        store
            .with_selected(|capture| {
                capture
                    .attempts
                    .iter()
                    .map(|record| attempt_row(capture, record))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });

    let details_view = Memo::new(move |_| match effective_key.get() {
        None => DetailsView::default(),
        Some(key) => store
            .with_selected(|capture| match find_attempt(capture, key) {
                Some(record) => details_view(capture, record, perf.frame_trace.get()),
                None => DetailsView::default(),
            })
            .unwrap_or_default(),
    });

    let json_view = Memo::new(move |_| {
        if view.get() != InspectorView::Json {
            // Lazy: the JSON is built only while the view is open.
            return JsonView::default();
        }
        let Some(key) = effective_key.get() else {
            return JsonView::default();
        };
        let text = store
            .with_selected(|capture| find_attempt(capture, key).map(attempt_json))
            .flatten()
            .and_then(Result::ok)
            .unwrap_or_default();
        JsonView {
            text,
            has_attempt: true,
        }
    });

    let send_capture = Callback::new(move |cmd: CaptureCmd| app.capture_cmd.set(Some(cmd)));
    let send_record = Callback::new(move |_: ()| {
        let cmd = if app.recording_active.get_untracked() {
            crate::app_state::RecordingCmd::Stop
        } else {
            crate::app_state::RecordingCmd::Start
        };
        app.recording_cmd.set(Some(cmd));
    });
    let send_svg = Callback::new(move |_: ()| {
        app.export_cmd.set(Some(crate::app_state::ExportCmd::Svg));
    });
    let send_pdf = Callback::new(move |_: ()| {
        app.export_cmd.set(Some(crate::app_state::ExportCmd::Pdf));
    });

    let on_close = Callback::new(move |_: ()| {
        // Closing the window pauses an active capture and keeps every record.
        if matches!(store.state_untracked(), CaptureState::Capturing(_)) {
            store.pause();
        }
        focus_launcher();
    });

    view! {
        <FloatingWindow
            open=app.inspector_open.read()
            set_open=app.inspector_open.write()
            title="Performance"
            on_close=on_close
        >
            <div class="pp-inspector">
                <div class="pp-head">
                    <span class="pp-head-label">"Capture"</span>
                    <select
                        class="pp-capture-select"
                        title="Select a retained capture"
                        prop:value=move || {
                            captures.get().1.map(|id| id.to_string()).unwrap_or_default()
                        }
                        on:change=move |ev: web_sys::Event| {
                            let Some(target) = ev.target() else { return };
                            let Ok(select) = target.dyn_into::<web_sys::HtmlSelectElement>()
                            else {
                                return;
                            };
                            if let Ok(id) = select.value().parse::<u64>() {
                                store.select(id);
                            }
                        }
                    >
                        {move || {
                            let (summaries, _) = captures.get();
                            if summaries.is_empty() {
                                return view! {
                                    <option value="">"no capture"</option>
                                }
                                .into_any();
                            }
                            summaries
                                .into_iter()
                                .map(|summary| {
                                    let id = summary.id;
                                    view! {
                                        <option value=id.to_string()>
                                            {format!(
                                                "{} \u{b7} {} attempt(s){}",
                                                summary.name,
                                                summary.attempts,
                                                if summary.active { " \u{b7} live" } else { "" },
                                            )}
                                        </option>
                                    }
                                })
                                .collect_view()
                                .into_any()
                        }}
                    </select>
                    <button
                        class="pp-action-btn"
                        type="button"
                        disabled=move || state.get() != CaptureState::Idle
                        title="Start a paint-attempt capture"
                        on:click=move |_| send_capture.run(CaptureCmd::Start)
                    >
                        "\u{25cf} New capture"
                    </button>
                    <button
                        class="pp-action-btn"
                        type="button"
                        disabled=move || !matches!(state.get(), CaptureState::Capturing(_))
                        title="Stop accepting records; every record is kept"
                        on:click=move |_| send_capture.run(CaptureCmd::Pause)
                    >
                        "\u{23f8} Pause"
                    </button>
                    <button
                        class="pp-action-btn"
                        type="button"
                        disabled=move || !matches!(state.get(), CaptureState::Paused(_))
                        title="Accept records again"
                        on:click=move |_| send_capture.run(CaptureCmd::Resume)
                    >
                        "\u{25b6} Resume"
                    </button>
                    <button
                        class="pp-action-btn"
                        type="button"
                        disabled=move || state.get() == CaptureState::Idle
                        title="Close the capture and keep it in the archive"
                        on:click=move |_| send_capture.run(CaptureCmd::Finish)
                    >
                        "\u{23f9} Finish"
                    </button>
                    <button
                        class="pp-action-btn"
                        type="button"
                        disabled=move || status.get().selected.is_none()
                        title="Download the selected capture as JSON"
                        on:click=move |_| send_capture.run(CaptureCmd::ExportCaptureJson)
                    >
                        "\u{2913} Export JSON"
                    </button>
                </div>

                <div class="pp-status-line">
                    {move || {
                        let status = status.get();
                        let mut text = format!(
                            "{} \u{b7} {} attempt(s) \u{b7} {} batch(es) \u{b7} {} mutation(s) \u{b7} {} capture(s) \u{b7} {} KiB",
                            status.state.label(),
                            status.attempts,
                            status.batches,
                            status.mutations,
                            status.retained_captures,
                            status.retained_bytes / 1024,
                        );
                        if let Some(reason) = status.stop_reason {
                            text.push_str(&format!(" \u{b7} {}", reason.label()));
                        }
                        if let Some(error) = error.get() {
                            text.push_str(&format!(" \u{b7} {error}"));
                        }
                        text
                    }}
                </div>

                <div class="pp-tabs" role="tablist">
                    {InspectorView::ALL
                        .iter()
                        .map(|candidate| {
                            let candidate = *candidate;
                            view! {
                                <button
                                    class="pp-tab"
                                    class:active=move || view.get() == candidate
                                    type="button"
                                    role="tab"
                                    aria-selected=move || {
                                        if view.get() == candidate { "true" } else { "false" }
                                    }
                                    on:click=move |_| view.set(candidate)
                                >
                                    {candidate.label()}
                                </button>
                            }
                        })
                        .collect_view()}
                </div>

                <div class="pp-view">
                    {move || match view.get() {
                        InspectorView::Digest => {
                            view! {
                                <InspectorDigest
                                    panel=digest_panel
                                    on_include_forced=Callback::new(move |include: bool| {
                                        filter_include_forced.set(include)
                                    })
                                    on_effective=Callback::new(move |strategy: Option<
                                        RenderStrategy,
                                    >| {
                                        filter_effective.set(strategy)
                                    })
                                />
                            }
                                .into_any()
                        }
                        InspectorView::Attempts => {
                            view! {
                                <InspectorAttempts
                                    rows=attempt_rows
                                    selected=selection.read_only()
                                    follow_latest=follow_latest.read_only()
                                    on_select=Callback::new(move |key: AttemptKey| {
                                        selection.set(Some(key));
                                        follow_latest.set(false);
                                    })
                                    on_follow_latest=Callback::new(move |follow: bool| {
                                        follow_latest.set(follow)
                                    })
                                />
                            }
                                .into_any()
                        }
                        InspectorView::Details => {
                            view! { <InspectorDetails view=details_view /> }.into_any()
                        }
                        InspectorView::Json => {
                            let key = effective_key.get();
                            view! {
                                <InspectorJson
                                    view=json_view
                                    on_copy=Callback::new(move |_: ()| {
                                        if let Some(key) = key {
                                            app.capture_cmd
                                                .set(Some(CaptureCmd::CopyAttemptJson(key)));
                                        }
                                    })
                                    on_export=Callback::new(move |_: ()| {
                                        app.capture_cmd.set(Some(CaptureCmd::ExportCaptureJson))
                                    })
                                />
                            }
                                .into_any()
                        }
                        InspectorView::Tools => {
                            view! {
                                <InspectorTools
                                    recording_active=app.recording_active.read()
                                    playback_loaded=app.playback_loaded.read()
                                    on_record=send_record
                                    on_export_svg=send_svg
                                    on_export_pdf=send_pdf
                                />
                            }
                                .into_any()
                        }
                    }}
                </div>
            </div>
        </FloatingWindow>
    }
}

/// The attempt the views show, by key.
fn find_attempt(capture: &CaptureRecord, key: AttemptKey) -> Option<&AttemptRecord> {
    capture.attempts.iter().find(|attempt| attempt.key == key)
}

/// One attempt row, with every string resolved here rather than in the view.
fn attempt_row(capture: &CaptureRecord, record: &AttemptRecord) -> AttemptRow {
    let diagnostics = &record.diagnostics;
    AttemptRow {
        key: record.key,
        seq: record.key.attempt_seq,
        scope: attempt_scope(capture, record),
        strategy: diagnostics.effective.or(diagnostics.selected).map_or_else(
            || "\u{2014}".to_owned(),
            |strategy| strategy_label(strategy).to_owned(),
        ),
        verdict: diagnostics
            .repaint
            .verdict
            .map_or_else(|| "\u{2014}".to_owned(), |verdict| verdict.to_string()),
        outcome: outcome_label(diagnostics.outcome).to_owned(),
        render_ms: record.render_call_ms,
        retry_of: record.retry_of.map(|key| key.attempt_seq),
        batches: record.batch_ids.len(),
    }
}

/// The host scope one attempt responded to, as A1 text when addressable.
///
/// A batch covers an input or notification scope; the label names the first
/// addressable range and how many more the attempt claimed.
fn attempt_scope(capture: &CaptureRecord, record: &AttemptRecord) -> String {
    let mut addresses: Vec<String> = Vec::new();
    let mut kinds: Vec<&'static str> = Vec::new();
    let mut unresolved = 0usize;
    for batch_id in &record.batch_ids {
        let summaries = capture.batch_summaries(*batch_id);
        if summaries.is_empty() {
            unresolved += 1;
        }
        for summary in summaries {
            match (summary.sheet.as_ref(), summary.scope.as_ref()) {
                (Some(sheet), Some(scope)) if scope.range().is_some() => {
                    let range = scope.range().expect("checked above");
                    addresses.push(format_range(sheet, range));
                }
                _ => kinds.push(summary.kind.label()),
            }
        }
    }
    let mut text = match addresses.first() {
        Some(first) if addresses.len() > 1 => format!("{first} +{}", addresses.len() - 1),
        Some(first) => first.clone(),
        None => kinds.first().copied().unwrap_or("\u{2014}").to_owned(),
    };
    if unresolved > 0 {
        text.push_str(&format!(" \u{b7} {unresolved} unresolved"));
    }
    text
}

/// Address evidence and the raw sections for one attempt.
fn details_view(
    capture: &CaptureRecord,
    record: &AttemptRecord,
    frame_trace: Option<String>,
) -> DetailsView {
    DetailsView {
        attempt: Some(format!("#{}", record.key.attempt_seq)),
        evidence: attempt_evidence(capture, record)
            .into_iter()
            .map(|row| EvidenceRow {
                label: row.label,
                precision: precision_label(&row.precision),
                note: row.note.map(note_label),
                addresses: row
                    .ranges
                    .iter()
                    .map(|range| format_range(&range.sheet, range.range))
                    .collect(),
                clip: row.clip.map(pixel_label),
            })
            .collect(),
        advanced: advanced_sections(record),
        frame_trace,
    }
}

/// The raw diagnostic sections, both grid-derived and attempt-derived.
fn advanced_sections(record: &AttemptRecord) -> Vec<AdvancedSection> {
    let diagnostics = &record.diagnostics;
    let sheet_name = record.sheet_name.clone();
    let address =
        |sheet: u32, range: RCRange| format_range(&SheetRef::new(sheet, sheet_name.clone()), range);

    let mut sections = Vec::new();

    let mut cache = vec![
        (
            "resolution".to_owned(),
            resolution_label(diagnostics.cache.resolution).to_owned(),
        ),
        (
            "planned action".to_owned(),
            option_debug(diagnostics.cache.planned_action, action_label),
        ),
        (
            "fingerprint action".to_owned(),
            option_debug(diagnostics.cache.fingerprint_action, fingerprint_label),
        ),
    ];
    cache.push((
        "committed seq".to_owned(),
        diagnostics
            .committed_seq
            .map_or_else(|| "\u{2014}".to_owned(), |seq| format!("#{seq}")),
    ));
    sections.push(AdvancedSection {
        title: "Cache",
        rows: cache,
    });

    if let Some(geometry) = diagnostics.geometry.as_ref() {
        let mut rows = vec![
            (
                "sheet".to_owned(),
                address_sheet(&sheet_name, geometry.sheet),
            ),
            ("top row".to_owned(), geometry.top_row.to_string()),
            ("left column".to_owned(), geometry.left_column.to_string()),
            (
                "backing size".to_owned(),
                format!(
                    "{} \u{d7} {}",
                    geometry.backing_size.0, geometry.backing_size.1
                ),
            ),
            ("dpr".to_owned(), format!("{:.2}", geometry.dpr)),
        ];
        for segment in &geometry.segments {
            rows.push((
                format!("segment {}", region_label(segment.region)),
                format!(
                    "{} \u{b7} {} cell(s)",
                    address(geometry.sheet, segment.range),
                    segment.cells
                ),
            ));
        }
        sections.push(AdvancedSection {
            title: "Geometry",
            rows,
        });
    }

    if let Some(blit) = diagnostics.blit.as_ref() {
        let sheet = diagnostics.geometry.as_ref().map(|geometry| geometry.sheet);
        sections.push(AdvancedSection {
            title: "Blit",
            rows: blit_rows(blit, sheet, &address),
        });
    }

    sections.push(AdvancedSection {
        title: "Work flags",
        rows: work_rows(diagnostics.work),
    });

    let probe = diagnostics.probe.map(|range| {
        let sheet = diagnostics.geometry.as_ref().map(|geometry| geometry.sheet);
        match sheet {
            Some(sheet) => address(sheet, range),
            None => format!("r{}-{} c{}-{}", range.r1, range.r2, range.c1, range.c2),
        }
    });
    sections.push(AdvancedSection {
        title: "Probe",
        rows: vec![
            (
                "expected change".to_owned(),
                probe.unwrap_or_else(|| "\u{2014}".to_owned()),
            ),
            (
                "containing segments".to_owned(),
                if diagnostics.probe_segments.is_empty() {
                    "\u{2014}".to_owned()
                } else {
                    diagnostics
                        .probe_segments
                        .iter()
                        .map(|region| region_label(*region).to_owned())
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            ),
        ],
    });

    sections
}

fn blit_rows(
    blit: &DiagBlit,
    sheet: Option<u32>,
    address: &impl Fn(u32, RCRange) -> String,
) -> Vec<(String, String)> {
    let mut rows = vec![
        (
            "axis".to_owned(),
            match blit.axis {
                Axis::Row => "row".to_owned(),
                Axis::Column => "column".to_owned(),
            },
        ),
        ("delta".to_owned(), blit.delta.to_string()),
        ("result".to_owned(), format!("{:?}", blit.result)),
        (
            "clip".to_owned(),
            blit.clip.map_or_else(|| "\u{2014}".to_owned(), pixel_label),
        ),
        ("strip".to_owned(), pixel_label(blit.strip)),
    ];
    for revealed in &blit.revealed {
        let range = match sheet {
            Some(sheet) => address(sheet, revealed.range),
            None => format!(
                "r{}-{} c{}-{}",
                revealed.range.r1, revealed.range.r2, revealed.range.c1, revealed.range.c2
            ),
        };
        rows.push((format!("revealed {}", region_label(revealed.region)), range));
    }
    rows
}

fn work_rows(work: WorkFlags) -> Vec<(String, String)> {
    const FLAGS: [(WorkFlags, &str); 4] = [
        (WorkFlags::VIEW, "view"),
        (WorkFlags::CONTENT, "content"),
        (WorkFlags::GEOMETRY, "geometry"),
        (WorkFlags::OVERLAY, "overlay"),
    ];
    FLAGS
        .iter()
        .map(|(flag, label)| {
            (
                (*label).to_owned(),
                if work.contains(*flag) { "set" } else { "clear" }.to_owned(),
            )
        })
        .collect()
}

fn precision_label(precision: &EvidencePrecision) -> String {
    match precision {
        EvidencePrecision::Exact => "exact".to_owned(),
        EvidencePrecision::Derived => "derived".to_owned(),
        EvidencePrecision::Unavailable(reason) => {
            format!("unavailable: {}", reason_label(*reason))
        }
    }
}

fn note_label(note: EvidenceNote) -> String {
    match note {
        EvidenceNote::NoVisibleFingerprintChanges => {
            "the comparison found no visible change".to_owned()
        }
        EvidenceNote::NotCompared => "the fingerprint comparison did not run".to_owned(),
        EvidenceNote::Unavailable(reason) => {
            format!("unavailable: {}", reason_label(reason))
        }
        EvidenceNote::HeldExcludesCoverage => {
            "a held attempt paints no coverage; revealed ranges are not painted work".to_owned()
        }
        EvidenceNote::ProbeIsNotAChangeList => {
            "the host probe is an expected-change hint, not a change list".to_owned()
        }
    }
}

fn reason_label(reason: UnavailableReason) -> &'static str {
    match reason {
        UnavailableReason::NotCompared => "not compared",
        UnavailableReason::NoPaintedHistory => "no painted history",
        UnavailableReason::LayoutMismatch => "layout mismatch",
        UnavailableReason::NoGridWork => "no grid work",
        UnavailableReason::HeldAttempt => "held attempt",
        UnavailableReason::SectionMissing => "section missing",
    }
}

fn outcome_label(outcome: FrameOutcome) -> &'static str {
    match outcome {
        FrameOutcome::Painted => "painted",
        FrameOutcome::HeldOnBridgeFailure => "held (bridge failure)",
        FrameOutcome::HeldOnInputFailure(_) => "held (input failure)",
    }
}

fn strategy_label(strategy: RenderStrategy) -> &'static str {
    match strategy {
        RenderStrategy::OverlayOnly => "overlay only",
        RenderStrategy::ScrollBlit => "scroll blit",
        RenderStrategy::ChangedCells => "changed cells",
        RenderStrategy::FullRebuild => "full rebuild",
        RenderStrategy::DamagedRows => "damaged rows",
    }
}

fn region_label(region: PaneRegion) -> &'static str {
    match region {
        PaneRegion::TopLeft => "top left",
        PaneRegion::TopRight => "top right",
        PaneRegion::BottomLeft => "bottom left",
        PaneRegion::BottomRight => "bottom right",
    }
}

fn resolution_label(resolution: DiagCacheResolution) -> &'static str {
    match resolution {
        DiagCacheResolution::Committed => "committed",
        DiagCacheResolution::HeldForRetry => "held for retry",
    }
}

fn action_label(action: DiagCacheActionTag) -> &'static str {
    match action {
        DiagCacheActionTag::None => "none",
        DiagCacheActionTag::Replace => "replace",
        DiagCacheActionTag::Splice => "splice",
        DiagCacheActionTag::Shift => "shift",
        DiagCacheActionTag::Reset => "reset",
    }
}

fn fingerprint_label(action: DiagFingerprintActionTag) -> &'static str {
    match action {
        DiagFingerprintActionTag::Install => "install",
        DiagFingerprintActionTag::MarkStale => "mark stale",
        DiagFingerprintActionTag::Reset => "reset",
    }
}

fn option_debug<T: Copy>(value: Option<T>, label: impl Fn(T) -> &'static str) -> String {
    value.map_or_else(|| "\u{2014}".to_owned(), |value| label(value).to_owned())
}

fn address_sheet(sheet_name: &Option<String>, sheet: u32) -> String {
    match sheet_name {
        Some(name) => name.clone(),
        None => format!("Sheet#{sheet}"),
    }
}

fn pixel_label(rect: PixelRect) -> String {
    format!(
        "{},{} {} \u{d7} {}",
        rect.left(),
        rect.top(),
        rect.width,
        rect.height
    )
}

/// Return focus to the toolbar launcher after a close.
///
/// The launcher is a separate component with no shared state, so the window
/// finds it by the id it publishes.
fn focus_launcher() {
    let Some(element) = window()
        .document()
        .and_then(|document| document.get_element_by_id("dev-tools-launcher"))
    else {
        return;
    };
    if let Ok(element) = element.dyn_into::<web_sys::HtmlElement>() {
        let _ = element.focus();
    }
}
