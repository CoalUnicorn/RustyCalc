//! Native integration tests for the dev-diagnostics snapshot.
//! Harness mirrors tests/orchestrator_strategies.rs: MemSurface + TestModel.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::PaneRegion;
use iron_canvas_core::geometry::prim::Axis;
use iron_canvas_core::{
    CanvasSize, DiagBlitResultTag, DiagBufferTruth, DiagCacheActionTag, DiagCacheResolution,
    DiagDeltaKind, DiagFetchPurpose, DiagFingerprintActionTag, DiagFingerprintTruth,
    DiagRepaintReason, FrameOutcome, GridVerdict, Orchestrator, PaintResult, RCRange,
    RebuildReason, RowSpan,
};
use iron_canvas_recorder::MemSurface;

use common::TestModel;

fn harness() -> (Orchestrator<MemSurface>, Rc<TestModel>) {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    (orch, model)
}

#[test]
fn disabled_by_default_publishes_no_snapshot() {
    let (mut orch, _model) = harness();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert!(orch.frame_diagnostics().is_none());
}

#[test]
fn enable_then_disable_round_trips() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().expect("enabled capture publishes");
    assert_eq!(diag.schema_version, 3);
    assert_eq!(diag.attempt_seq, 1);
    assert_eq!(diag.committed_seq, Some(1));
    orch.set_frame_diagnostics_enabled(false);
    assert!(orch.frame_diagnostics().is_none());
}

#[test]
fn capture_hold_still_publishes_and_keeps_cache_state() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    model.set_capture_fail(Some(iron_canvas_core::FrameInputFailure::SelectedSheet));
    orch.request_repaint();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.attempt_seq, 2);
    assert_eq!(diag.committed_seq, None);
    assert_eq!(
        diag.outcome,
        iron_canvas_core::FrameOutcome::HeldOnInputFailure(
            iron_canvas_core::FrameInputFailure::SelectedSheet
        )
    );
    assert_eq!(
        diag.cache.committed_before,
        Some(diag.cache.committed_after.clone()),
        "a held attempt never presents changed cache state"
    );
}

#[test]
fn overlay_only_attempt_commits_without_cache_work() {
    // Live recipe from orchestrator_strategies.rs: an in-viewport
    // selection move is a committed Overlay strategy with NO grid cache
    // commit. It must not be mislabelled as held.
    let model = Rc::new(TestModel::synthetic_grid().with_active(5, 2));
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_active(6, 2);
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(
        diag.cache.resolution,
        iron_canvas_core::DiagCacheResolution::Committed,
        "an OverlayOnly strategy commits a transaction; it is not held"
    );
    assert!(diag.committed_seq.is_some());
    assert_eq!(diag.cache.planned_action, None);
    assert!(!diag.painted_layers.grid);
    assert!(diag.painted_layers.overlay);
    assert_eq!(
        diag.cache.committed_before,
        Some(diag.cache.committed_after.clone())
    );
    assert!(diag.geometry.is_none());
}
#[test]
fn cold_start_reports_no_committed_frame_reason_and_delta_rebuild() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.delta, Some(DiagDeltaKind::Rebuild));
    assert_eq!(diag.rebuild_reason, Some(RebuildReason::NoCommittedFrame));
}

#[test]
fn freeze_rebuild_reports_reason_and_exact_segments() {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40).with_frozen(2, 1));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_frozen_rows(3);
    orch.request_repaint();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.rebuild_reason, Some(RebuildReason::Freeze));
    let geo = diag.geometry.expect("grid-visited attempt has geometry");
    assert_eq!(geo.shape.frozen_rows(), 3);
    assert_eq!(geo.shape.frozen_cols(), 1);
    // Canonical TL/TR/BL/BR order, every region populated.
    let regions: Vec<PaneRegion> = geo.segments.iter().map(|s| s.region).collect();
    assert_eq!(
        regions,
        vec![
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight
        ]
    );
    // Frozen band: the TL/TR segments span rows 1..=3.
    assert_eq!(geo.segments[0].range.r2, 3);
    assert_eq!(geo.segments[1].range.r2, 3);
    assert_eq!(geo.segments[0].range.c2, 1);
    assert_eq!(geo.segments[2].range.c2, 1);
}

#[test]
fn probe_reports_exact_containing_segment_and_is_consumed() {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40).with_frozen(2, 1));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    // Probe the frozen top-left corner: exactly TL contains it.
    orch.set_frame_diagnostics_probe(RCRange {
        r1: 1,
        c1: 1,
        r2: 1,
        c2: 1,
    });
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(
        diag.probe,
        Some(RCRange {
            r1: 1,
            c1: 1,
            r2: 1,
            c2: 1
        })
    );
    assert_eq!(diag.probe_segments, vec![PaneRegion::TopLeft]);

    // The probe is attempt-scoped: the next attempt consumes nothing.
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.probe, None);
    assert!(diag.probe_segments.is_empty());
}

#[test]
fn probe_outside_all_segments_reports_empty_attribution() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    orch.set_frame_diagnostics_probe(RCRange {
        r1: 999,
        c1: 999,
        r2: 999,
        c2: 999,
    });
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert!(diag.probe.is_some());
    assert!(diag.probe_segments.is_empty());
}

#[test]
fn overlay_only_attempt_has_no_geometry_and_no_probe_segments() {
    let model = Rc::new(TestModel::synthetic_grid().with_active(5, 2));
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_active(6, 2);
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.delta, Some(DiagDeltaKind::Stable));
    assert!(diag.geometry.is_none());
    assert_eq!(diag.repaint.verdict, None);
    assert!(diag.probe_segments.is_empty());
}

#[test]
fn unchanged_content_skip_reports_fingerprints_equal() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Skip));
    assert_eq!(
        diag.repaint.reason,
        Some(DiagRepaintReason::FingerprintsEqual)
    );
    assert!(diag.repaint.changed_rows.is_empty());
    assert!(diag.repaint.changed_cells.is_empty());
    assert_eq!(diag.repaint.clip, None);
    assert!(diag.repaint.source_ranges.is_empty());
}

#[test]
fn one_changed_cell_reports_exact_evidence_and_executed_envelope() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    model.set_cell(4, 2, "new value");
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Cell));
    assert_eq!(diag.repaint.reason, Some(DiagRepaintReason::ChangedCell));
    assert_eq!(diag.repaint.changed_rows, vec![RowSpan { r1: 4, r2: 4 }]);
    assert_eq!(diag.repaint.changed_cells.len(), 1);
    assert_eq!(diag.repaint.changed_cells[0].row, 4);
    assert_eq!(diag.repaint.changed_cells[0].column, 2);
    assert!(diag.repaint.clip.is_some());
    assert!(!diag.repaint.source_ranges.is_empty());
    assert!(diag.paint_counts.cells < diag.fetch.addressed_cells);
}

#[test]
fn span_cap_disables_rows_but_keeps_a_bounded_range() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    // Nine disjoint changed rows exceed the row-sweep alternative's 8-span cap.
    for (i, row) in [1, 3, 5, 7, 9, 11, 13, 15, 17].iter().enumerate() {
        model.set_cell(*row, 2, &format!("v{i}"));
    }
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Range));
    assert_eq!(diag.repaint.reason, Some(DiagRepaintReason::ChangedCells));
    assert_eq!(diag.repaint.changed_rows.len(), 9);
    assert_eq!(diag.repaint.changed_cells.len(), 9);
    assert!(diag.repaint.clip.is_some());
}

#[test]
fn border_change_uses_changed_cell_envelope() {
    use iron_canvas_core::{Border, BorderItem, BorderStyle, CellStyle};
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    // Exact leaf evidence bypasses the old whole-row border promotion.
    model.set_style(
        4,
        2,
        CellStyle {
            border: Border {
                top: Some(BorderItem {
                    style: BorderStyle::Thin,
                    color: Some("#000000".to_string()),
                }),
                ..Border::default()
            },
            ..CellStyle::default()
        },
    );
    model.set_cell(4, 2, "bordered");
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Cell));
    assert_eq!(diag.repaint.reason, Some(DiagRepaintReason::ChangedCell));
    assert_eq!(diag.repaint.changed_cells.len(), 1);
    assert!(diag.repaint.clip.is_some());
}

#[test]
fn fresh_rebuild_full_carries_no_fingerprint_reason() {
    // A freeze rebuild has painted history but the comparison never ran:
    // the snapshot must not fabricate `noPaintedHistory`. The captured
    // rebuildReason is the authority instead.
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40).with_frozen(2, 1));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_frozen_rows(3);
    orch.request_repaint();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Full));
    assert_eq!(diag.repaint.reason, None);
    assert_eq!(diag.rebuild_reason, Some(RebuildReason::Freeze));
}

#[test]
fn damaged_rows_strip_reports_strip_verdict_without_reason() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    model.set_cell(4, 2, "damage edit");
    orch.mark_rows_damaged(0, RowSpan { r1: 4, r2: 4 });
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Strip));
    assert_eq!(diag.repaint.reason, None);
}

#[test]
fn fetch_requests_sum_to_totals_and_match_segments() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.fetch.requests.len(), diag.fetch.batches);
    assert_eq!(
        diag.fetch.requests.iter().map(|r| r.cells).sum::<usize>(),
        diag.fetch.addressed_cells
    );
    assert_eq!(
        diag.fetch.requests.iter().map(|r| r.slots).sum::<usize>(),
        diag.fetch.logical_slots
    );
    // The segment cell counts are the renderer's own fetch accounting:
    // their sum equals the addressed-cell total.
    let geo = diag.geometry.unwrap();
    let cells: usize = geo.segments.iter().map(|s| s.cells).sum();
    assert_eq!(cells, diag.fetch.addressed_cells);
    // Every request's range lives inside its segment and every request is
    // a full-segment fetch on the cold Fresh frame.
    for request in &diag.fetch.requests {
        assert_eq!(request.purpose, DiagFetchPurpose::FullSegment);
        let region = request.region.unwrap();
        let segment = geo
            .segments
            .iter()
            .find(|s| s.region == region)
            .expect("request region has a segment");
        assert!(request.range.r1 >= segment.range.r1);
        assert!(request.range.r2 <= segment.range.r2);
        assert!(request.range.c1 >= segment.range.c1);
        assert!(request.range.c2 <= segment.range.c2);
    }
}

#[test]
fn row_blit_reports_shift_revealed_strip_and_effective_clip() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    model.set_top_row(5);
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.delta, Some(DiagDeltaKind::Scroll));
    let blit = diag.blit.expect("one-axis scroll blits");
    assert_eq!(blit.axis, Axis::Row);
    assert_eq!(blit.delta, 4);
    assert_eq!(blit.result, DiagBlitResultTag::Shifted);
    assert!(blit.cold_cache.is_none());
    // The revealed strips are the renderer's actual widened repaint bands:
    // `revealed_strip` includes a boundary-overlap row (r1 = prev.r2) and
    // `widen_to_pixel_clip` extends to every row intersecting the strip
    // band, so the band covers at least the delta's newly scrolled-in rows.
    let revealed_rows: i32 = blit
        .revealed
        .iter()
        .map(|strip| strip.range.r2 - strip.range.r1 + 1)
        .sum();
    assert!(revealed_rows >= blit.delta);
    // The blit's source and destination rectangles differ by the shift.
    assert_ne!(blit.src, blit.dst);
    assert!(blit.strip.width > 0 && blit.strip.height > 0);
    // Today's finalized blit work hands `plan.pixel_strip` to push_clip
    // (blit_work.rs:113), so the effective clip equals the repaint band —
    // the snapshot must record the actual push_clip argument, as Some.
    assert_eq!(blit.clip, Some(blit.strip));
    // Fetch requests for a clean shift are reveal-purpose only.
    assert!(
        diag.fetch
            .requests
            .iter()
            .all(|r| r.purpose == DiagFetchPurpose::BlitReveal)
    );
}

#[test]
fn geometry_reports_css_and_backing_size() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    let geo = diag.geometry.expect("grid-visited attempt has geometry");
    assert_eq!(geo.canvas, CanvasSize { w: 800.0, h: 600.0 });
    // dpr 1.0: derived backing size equals the CSS size.
    assert_eq!(geo.backing_size, (800, 600));

    // dpr 2.0: the derived backing size doubles, matching browser
    // rounding of CSS x DPR.
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 2.0);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    let geo = diag.geometry.expect("grid-visited attempt has geometry");
    assert_eq!(geo.dpr, 2.0);
    assert_eq!(geo.backing_size, (1600, 1200));
}

#[test]
fn damaged_rows_with_frozen_columns_reports_one_painted_row() {
    // Frozen columns split one damaged row band into left and right
    // segments. `paint.rows` counts DISTINCT grid rows, so the same
    // absolute row visited in both segments must count exactly once.
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40).with_frozen(2, 1));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_cell(4, 2, "damaged");
    orch.mark_rows_damaged(0, RowSpan { r1: 4, r2: 4 });
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Strip));
    assert_eq!(
        diag.paint_counts.rows, 1,
        "one damaged row split across frozen segments must count once"
    );
    // The cells stay disjoint across segments: exactly the frozen BL
    // column plus every visible BR column, from the segment ranges the
    // renderer actually painted.
    let geo = diag.geometry.expect("grid-visited attempt has geometry");
    let br = geo
        .segments
        .iter()
        .find(|s| s.region == PaneRegion::BottomRight)
        .expect("frozen columns create a BR segment");
    // The frozen BL column contributes exactly one cell (row 4, col 1).
    assert_eq!(
        diag.paint_counts.cells,
        1 + (br.range.c2 - br.range.c1 + 1) as usize
    );
}

#[test]
fn fresh_fallback_blit_reports_no_clip_and_full_verdict() {
    // Row-header digit boundary (last visible row 999 -> 1000 widens the
    // header): `Chrome::prepare_blit` rejects in-place reuse and the
    // attempt demotes to a full Fresh rebuild. The blit record must say
    // FreshFallback and must NOT fabricate a clip rectangle.
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(980)
            .with_active(980, 1),
    );
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 600.0, h: 400.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_top_row(981);
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let diag = orch.frame_diagnostics().unwrap();
    let blit = diag.blit.expect("viewport attempt records blit detail");
    assert_eq!(blit.result, DiagBlitResultTag::FreshFallback);
    assert_eq!(
        blit.clip, None,
        "a Fresh fallback never reaches push_clip and must not fabricate a clip"
    );
    assert!(
        matches!(diag.repaint.verdict, Some(GridVerdict::Full)),
        "the fallback repaints the whole grid; got {:?}",
        diag.repaint.verdict
    );
}

#[test]
fn committed_attempt_records_cache_transition() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.cache.resolution, DiagCacheResolution::Committed);
    assert_eq!(diag.cache.planned_action, Some(DiagCacheActionTag::Replace));
    assert_eq!(
        diag.cache.fingerprint_action,
        Some(DiagFingerprintActionTag::Install)
    );
    let before = diag
        .cache
        .committed_before
        .expect("a dispatched attempt samples its starting cache truth");
    assert!(before.layout.is_none());
    assert_eq!(before.buffer_truth, DiagBufferTruth::Stale);
    assert_eq!(before.fingerprint_truth, DiagFingerprintTruth::Stale);
    assert!(diag.cache.committed_after.layout.is_some());
    assert_eq!(
        diag.cache.committed_after.buffer_truth,
        DiagBufferTruth::Valid
    );
    assert_eq!(
        diag.cache.committed_after.fingerprint_truth,
        DiagFingerprintTruth::Exact
    );
    assert!(diag.paint_counts.rows > 0);
    assert!(diag.paint_counts.cells > diag.paint_counts.rows);
}

#[test]
fn held_attempt_keeps_committed_cache_state() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    model.set_bulk_bridge_fail(true);
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.outcome, FrameOutcome::HeldOnBridgeFailure);
    assert_eq!(diag.cache.resolution, DiagCacheResolution::HeldForRetry);
    assert_eq!(
        diag.cache.committed_before,
        Some(diag.cache.committed_after.clone()),
        "a held attempt must not present candidate cache state as committed"
    );
    assert_eq!(
        diag.repaint.verdict,
        Some(GridVerdict::Held),
        "a held Fresh attempt must stamp the final grid verdict, not null"
    );
}

#[test]
fn held_damage_attempt_reports_held_verdict() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    // Fail the damage-strip fetch: the Damage strategy must hold.
    model.set_bulk_bridge_fail(true);
    model.set_cell(4, 2, "damaged");
    orch.mark_rows_damaged(0, RowSpan { r1: 4, r2: 4 });
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.outcome, FrameOutcome::HeldOnBridgeFailure);
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Held));
}

#[test]
fn held_blit_preflight_reports_held_preflight_result() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    // Fail the revealed-strip fetch: the blit preflight must hold.
    model.set_bulk_bridge_fail(true);
    model.set_top_row(5);
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    let diag = orch.frame_diagnostics().unwrap();
    let blit = diag.blit.expect("scroll attempt records blit detail");
    assert_eq!(blit.result, DiagBlitResultTag::HeldPreflight);
    assert_eq!(
        diag.repaint.verdict,
        Some(GridVerdict::Held),
        "a held blit attempt must stamp the final grid verdict, not null"
    );
    assert_eq!(
        blit.clip, None,
        "a held blit never reached push_clip and must not fabricate a clip"
    );
}
