//! Labelled address evidence for one paint attempt.
//!
//! The adapter turns one immutable [`AttemptRecord`] into a list of evidence
//! rows. Every row names its [`EvidenceSource`] and its
//! [`EvidencePrecision`], so a reader can separate what the host reported,
//! what the engine recorded, and what this adapter derived. A list that
//! cannot be built is `Unavailable` with a reason — never an empty list.
//!
//! Nothing here reads a signal, the canvas, a clock, or the model. Sheet
//! identity comes from the record itself and from the host batch summaries,
//! never from the capture-level roster, so a later rename or delete cannot
//! rewrite history.

use super::capture::{AttemptRecord, CaptureRecord, HostBatchSummary, SheetRef};
use iron_canvas_core::renderer::diag::{
    DiagCacheResolution, DiagFetchPurpose, DiagGeometry, DiagRepaintReason, FrameDiagnostics,
};
use iron_canvas_core::{GridVerdict, PixelRect, RCRange, RenderStrategy, RowSpan, col_name};

/// One address range on one sheet, with the sheet identity captured with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressRange {
    pub sheet: SheetRef,
    pub range: RCRange,
}

/// Where one evidence list came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceSource {
    ReportedChanges,
    FingerprintChanges,
    FetchRequests,
    RepaintSourceRanges,
    RepaintClip,
    RepaintedCoverage,
    RevealedRanges,
}

/// How much the evidence list can be trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidencePrecision {
    /// Recorded directly by the engine or the host.
    Exact,
    /// Computed by this adapter from executed facts.
    Derived,
    /// The data does not exist for this attempt.
    Unavailable(UnavailableReason),
}

/// Why an evidence list is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    /// The fingerprint comparison did not run on this path.
    NotCompared,
    /// The attempt had no painted history to compare against.
    NoPaintedHistory,
    /// The committed and candidate layouts differ.
    LayoutMismatch,
    /// The attempt painted no grid layer.
    NoGridWork,
    /// The attempt held its transaction; coverage is undecided.
    HeldAttempt,
    /// The section that would hold the data is absent from the snapshot.
    SectionMissing,
}

/// A caveat attached to an evidence row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceNote {
    /// The comparison ran and found no visible change.
    NoVisibleFingerprintChanges,
    /// The fingerprint comparison did not run.
    NotCompared,
    /// The answer is missing for `reason`.
    Unavailable(UnavailableReason),
    /// A held attempt paints no coverage, so its revealed ranges must not be
    /// read as painted work.
    HeldExcludesCoverage,
    /// The host probe is an expected-change hint, not a change list.
    ProbeIsNotAChangeList,
}

/// One row of address evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressEvidence {
    pub label: &'static str,
    pub source: EvidenceSource,
    pub precision: EvidencePrecision,
    pub ranges: Vec<AddressRange>,
    /// A pixel rectangle in CSS pixels. Clips never mix with addresses.
    pub clip: Option<PixelRect>,
    pub note: Option<EvidenceNote>,
}

/// Derived repaint coverage, or the reason it cannot be derived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Coverage {
    None { reason: UnavailableReason },
    Ranges(Vec<AddressRange>),
}

/// Build the evidence rows for one attempt, in display order.
///
/// `capture` supplies the host batch summaries the attempt's `batch_ids`
/// name. The rows are: reported changes, fingerprint changes, fetched ranges,
/// repaint source ranges, repaint clip, repainted coverage, revealed ranges.
pub fn attempt_evidence(capture: &CaptureRecord, record: &AttemptRecord) -> Vec<AddressEvidence> {
    let sheet_id = record
        .diagnostics
        .geometry
        .as_ref()
        .map(|geometry| geometry.sheet);
    vec![
        reported_changes(capture, record),
        fingerprint_changes(record, sheet_id),
        fetched_ranges(record, sheet_id),
        source_ranges(record, sheet_id),
        repaint_clip(&record.diagnostics),
        coverage_row(record),
        revealed_ranges(record, sheet_id),
    ]
}

/// Resolve the address coverage the attempt actually repainted.
///
/// The decision uses executed facts only: the cache resolution, the painted
/// layers, the effective strategy (falling back to the selected one), and the
/// verdict. Work flags describe queued intent, so they are never a gate — a
/// view-only attempt that executed a scroll blit still reports its revealed
/// coverage.
pub fn repainted_coverage(diag: &FrameDiagnostics) -> Coverage {
    if diag.cache.resolution == DiagCacheResolution::HeldForRetry {
        return Coverage::None {
            reason: UnavailableReason::HeldAttempt,
        };
    }
    if !diag.painted_layers.grid {
        return Coverage::None {
            reason: UnavailableReason::NoGridWork,
        };
    }
    let Some(geometry) = diag.geometry.as_ref() else {
        return Coverage::None {
            reason: UnavailableReason::SectionMissing,
        };
    };
    let sheet = SheetRef::new(geometry.sheet, None);
    let effective = diag.effective.or(diag.selected);

    match diag.repaint.verdict {
        Some(GridVerdict::Cell) | Some(GridVerdict::Range) => Coverage::Ranges(
            diag.repaint
                .source_ranges
                .iter()
                .map(|source| AddressRange {
                    sheet: sheet.clone(),
                    range: source.range,
                })
                .collect(),
        ),
        Some(GridVerdict::Full) => Coverage::Ranges(
            geometry
                .segments
                .iter()
                .map(|segment| AddressRange {
                    sheet: sheet.clone(),
                    range: segment.range,
                })
                .collect(),
        ),
        Some(GridVerdict::Rows { .. }) => {
            Coverage::Ranges(row_coverage(&diag.repaint.changed_rows, geometry))
        }
        // A scroll strip repainted by damage uses its damage-strip fetches. A
        // successful blit uses the revealed strips. A fallback never reaches
        // here: its verdict is `Full`, not `Strip`.
        Some(GridVerdict::Strip) => match effective {
            Some(RenderStrategy::DamagedRows) => Coverage::Ranges(
                diag.fetch
                    .requests
                    .iter()
                    .filter(|request| request.purpose == DiagFetchPurpose::DamageStrip)
                    .map(|request| AddressRange {
                        sheet: sheet.clone(),
                        range: request.range,
                    })
                    .collect(),
            ),
            Some(RenderStrategy::ScrollBlit) => match diag.blit.as_ref() {
                Some(blit) => Coverage::Ranges(
                    blit.revealed
                        .iter()
                        .map(|revealed| AddressRange {
                            sheet: sheet.clone(),
                            range: revealed.range,
                        })
                        .collect(),
                ),
                None => Coverage::None {
                    reason: UnavailableReason::SectionMissing,
                },
            },
            _ => Coverage::None {
                reason: UnavailableReason::NoGridWork,
            },
        },
        Some(GridVerdict::Skip) => Coverage::None {
            reason: UnavailableReason::NoGridWork,
        },
        Some(GridVerdict::Held) => Coverage::None {
            reason: UnavailableReason::HeldAttempt,
        },
        None => Coverage::None {
            reason: UnavailableReason::NoGridWork,
        },
    }
}

/// Render `Sheet1!A4:R4`, or `Sheet#2!A4:R4` when the name is unavailable.
///
/// A single cell collapses to one coordinate, `Sheet1!A4`.
pub fn format_range(sheet: &SheetRef, range: RCRange) -> String {
    let name = match &sheet.name {
        Some(name) => name.clone(),
        None => format!("Sheet#{}", sheet.id),
    };
    let start = format!("{}{}", col_name(range.c1), range.r1);
    if range.r1 == range.r2 && range.c1 == range.c2 {
        return format!("{name}!{start}");
    }
    format!("{name}!{start}:{}{}", col_name(range.c2), range.r2)
}

/// One `AddressRange` per changed-row span, intersected with each visible
/// segment so frozen columns stay separate.
fn row_coverage(rows: &[RowSpan], geometry: &DiagGeometry) -> Vec<AddressRange> {
    let sheet = SheetRef::new(geometry.sheet, None);
    let mut out = Vec::new();
    for segment in &geometry.segments {
        for span in rows {
            let r1 = span.start().max(segment.range.r1);
            let r2 = span.end().min(segment.range.r2);
            if r1 <= r2 {
                out.push(AddressRange {
                    sheet: sheet.clone(),
                    range: RCRange {
                        r1,
                        c1: segment.range.c1,
                        r2,
                        c2: segment.range.c2,
                    },
                });
            }
        }
    }
    out
}

/// Reported host changes, resolved through `capture.batches`.
///
/// A referenced batch that is absent makes the list incomplete, so the row is
/// `Unavailable(SectionMissing)` — never an empty list. The label names the
/// scope, because a host batch is an input or notification scope, not a
/// change list.
fn reported_changes(capture: &CaptureRecord, record: &AttemptRecord) -> AddressEvidence {
    const LABEL: &str = "reported changes (host input scope)";
    let mut ranges = Vec::new();
    let mut missing = false;
    for batch_id in &record.batch_ids {
        let summaries = capture.batch_summaries(*batch_id);
        if summaries.is_empty() {
            missing = true;
        }
        for summary in summaries {
            if let Some(range) = summary_range(summary) {
                ranges.push(range);
            }
        }
    }

    let note = record
        .diagnostics
        .probe
        .is_some()
        .then_some(EvidenceNote::ProbeIsNotAChangeList);
    if missing {
        return AddressEvidence {
            label: LABEL,
            source: EvidenceSource::ReportedChanges,
            precision: EvidencePrecision::Unavailable(UnavailableReason::SectionMissing),
            ranges: Vec::new(),
            clip: None,
            note,
        };
    }
    AddressEvidence {
        label: LABEL,
        source: EvidenceSource::ReportedChanges,
        precision: EvidencePrecision::Exact,
        ranges,
        clip: None,
        note,
    }
}

/// The addressable range of one host summary, if it has one.
fn summary_range(summary: &HostBatchSummary) -> Option<AddressRange> {
    let sheet = summary.sheet.clone()?;
    let range = summary.scope.as_ref()?.range()?;
    Some(AddressRange { sheet, range })
}

/// Fingerprint changes, the engine's exact change list when it ran.
///
/// An empty cell list is not reported as zero changes: it is
/// `NoVisibleFingerprintChanges` only after an equal comparison, and
/// `NotCompared` or `Unavailable` for the paths that never compared.
fn fingerprint_changes(record: &AttemptRecord, sheet_id: Option<u32>) -> AddressEvidence {
    const LABEL: &str = "fingerprint changes";
    let diag = &record.diagnostics;
    let cells = &diag.repaint.changed_cells;
    let rows = &diag.repaint.changed_rows;
    let effective = diag.effective.or(diag.selected);

    if !cells.is_empty() || !rows.is_empty() {
        let Some(sheet_id) = sheet_id else {
            return unavailable(
                LABEL,
                EvidenceSource::FingerprintChanges,
                UnavailableReason::SectionMissing,
            );
        };
        let sheet = SheetRef::new(sheet_id, record.sheet_name.clone());
        let mut ranges: Vec<AddressRange> = cells
            .iter()
            .map(|cell| AddressRange {
                sheet: sheet.clone(),
                range: RCRange {
                    r1: cell.row,
                    c1: cell.column,
                    r2: cell.row,
                    c2: cell.column,
                },
            })
            .collect();
        if !rows.is_empty() {
            match visible_columns(diag) {
                Some((c1, c2)) => {
                    ranges.extend(rows.iter().map(|span| AddressRange {
                        sheet: sheet.clone(),
                        range: RCRange {
                            r1: span.start(),
                            c1,
                            r2: span.end(),
                            c2,
                        },
                    }));
                }
                None => {
                    return unavailable(
                        LABEL,
                        EvidenceSource::FingerprintChanges,
                        UnavailableReason::SectionMissing,
                    );
                }
            }
        }
        return AddressEvidence {
            label: LABEL,
            source: EvidenceSource::FingerprintChanges,
            precision: EvidencePrecision::Exact,
            ranges,
            clip: None,
            note: None,
        };
    }

    let reason = match diag.repaint.reason {
        Some(DiagRepaintReason::FingerprintsEqual) => {
            return AddressEvidence {
                label: LABEL,
                source: EvidenceSource::FingerprintChanges,
                precision: EvidencePrecision::Exact,
                ranges: Vec::new(),
                clip: None,
                note: Some(EvidenceNote::NoVisibleFingerprintChanges),
            };
        }
        Some(DiagRepaintReason::NoPaintedHistory) => UnavailableReason::NoPaintedHistory,
        Some(DiagRepaintReason::LayoutMismatch) => UnavailableReason::LayoutMismatch,
        Some(_) => UnavailableReason::NotCompared,
        None => match effective {
            Some(RenderStrategy::OverlayOnly) => UnavailableReason::NoGridWork,
            Some(RenderStrategy::DamagedRows)
            | Some(RenderStrategy::FullRebuild)
            | Some(RenderStrategy::ScrollBlit)
            | Some(RenderStrategy::ChangedCells) => UnavailableReason::NotCompared,
            None => UnavailableReason::SectionMissing,
        },
    };
    unavailable(LABEL, EvidenceSource::FingerprintChanges, reason)
}

/// Renderer-owned fetches, listed even on held attempts.
fn fetched_ranges(record: &AttemptRecord, sheet_id: Option<u32>) -> AddressEvidence {
    const LABEL: &str = "fetched ranges";
    let requests = &record.diagnostics.fetch.requests;
    if requests.is_empty() {
        return AddressEvidence {
            label: LABEL,
            source: EvidenceSource::FetchRequests,
            precision: EvidencePrecision::Exact,
            ranges: Vec::new(),
            clip: None,
            note: None,
        };
    }
    let Some(sheet_id) = sheet_id else {
        return unavailable(
            LABEL,
            EvidenceSource::FetchRequests,
            UnavailableReason::SectionMissing,
        );
    };
    let sheet = SheetRef::new(sheet_id, record.sheet_name.clone());
    AddressEvidence {
        label: LABEL,
        source: EvidenceSource::FetchRequests,
        precision: EvidencePrecision::Exact,
        ranges: requests
            .iter()
            .map(|request| AddressRange {
                sheet: sheet.clone(),
                range: request.range,
            })
            .collect(),
        clip: None,
        note: None,
    }
}

/// The repaint's own source envelope, one entry per recorded range.
fn source_ranges(record: &AttemptRecord, sheet_id: Option<u32>) -> AddressEvidence {
    const LABEL: &str = "repaint source ranges";
    let ranges_in = &record.diagnostics.repaint.source_ranges;
    if ranges_in.is_empty() {
        return AddressEvidence {
            label: LABEL,
            source: EvidenceSource::RepaintSourceRanges,
            precision: EvidencePrecision::Exact,
            ranges: Vec::new(),
            clip: None,
            note: None,
        };
    }
    let Some(sheet_id) = sheet_id else {
        return unavailable(
            LABEL,
            EvidenceSource::RepaintSourceRanges,
            UnavailableReason::SectionMissing,
        );
    };
    let sheet = SheetRef::new(sheet_id, record.sheet_name.clone());
    AddressEvidence {
        label: LABEL,
        source: EvidenceSource::RepaintSourceRanges,
        precision: EvidencePrecision::Exact,
        ranges: ranges_in
            .iter()
            .map(|source| AddressRange {
                sheet: sheet.clone(),
                range: source.range,
            })
            .collect(),
        clip: None,
        note: None,
    }
}

/// The repaint clip, in CSS pixels. It never mixes with the address lists.
fn repaint_clip(diag: &FrameDiagnostics) -> AddressEvidence {
    AddressEvidence {
        label: "repaint clip",
        source: EvidenceSource::RepaintClip,
        precision: EvidencePrecision::Exact,
        ranges: Vec::new(),
        clip: diag.repaint.clip,
        note: None,
    }
}

/// Derived coverage, with the note that a held attempt excludes it.
fn coverage_row(record: &AttemptRecord) -> AddressEvidence {
    const LABEL: &str = "repainted coverage";
    match repainted_coverage(&record.diagnostics) {
        Coverage::Ranges(mut ranges) => {
            for range in &mut ranges {
                range.sheet.name.clone_from(&record.sheet_name);
            }
            AddressEvidence {
                label: LABEL,
                source: EvidenceSource::RepaintedCoverage,
                precision: EvidencePrecision::Derived,
                ranges,
                clip: None,
                note: None,
            }
        }
        Coverage::None { reason } => AddressEvidence {
            label: LABEL,
            source: EvidenceSource::RepaintedCoverage,
            precision: EvidencePrecision::Unavailable(reason),
            ranges: Vec::new(),
            clip: None,
            note: (reason == UnavailableReason::HeldAttempt)
                .then_some(EvidenceNote::HeldExcludesCoverage),
        },
    }
}

/// The blit's revealed strips, with the blit clip kept apart from addresses.
fn revealed_ranges(record: &AttemptRecord, sheet_id: Option<u32>) -> AddressEvidence {
    const LABEL: &str = "revealed ranges";
    let Some(blit) = record.diagnostics.blit.as_ref() else {
        return unavailable(
            LABEL,
            EvidenceSource::RevealedRanges,
            UnavailableReason::SectionMissing,
        );
    };
    let Some(sheet_id) = sheet_id else {
        return unavailable(
            LABEL,
            EvidenceSource::RevealedRanges,
            UnavailableReason::SectionMissing,
        );
    };
    let sheet = SheetRef::new(sheet_id, record.sheet_name.clone());
    AddressEvidence {
        label: LABEL,
        source: EvidenceSource::RevealedRanges,
        precision: EvidencePrecision::Exact,
        ranges: blit
            .revealed
            .iter()
            .map(|revealed| AddressRange {
                sheet: sheet.clone(),
                range: revealed.range,
            })
            .collect(),
        clip: blit.clip,
        note: None,
    }
}

/// The visible column extent across every populated segment.
fn visible_columns(diag: &FrameDiagnostics) -> Option<(i32, i32)> {
    let geometry = diag.geometry.as_ref()?;
    let mut segments = geometry.segments.iter();
    let first = segments.next()?;
    let mut c1 = first.range.c1;
    let mut c2 = first.range.c2;
    for segment in segments {
        c1 = c1.min(segment.range.c1);
        c2 = c2.max(segment.range.c2);
    }
    Some((c1, c2))
}

fn unavailable(
    label: &'static str,
    source: EvidenceSource,
    reason: UnavailableReason,
) -> AddressEvidence {
    AddressEvidence {
        label,
        source,
        precision: EvidencePrecision::Unavailable(reason),
        ranges: Vec::new(),
        clip: None,
        note: None,
    }
}
