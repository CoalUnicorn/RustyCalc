use std::rc::Rc;

use crate::CanvasModel;
use crate::frame::FrameInputs;
use crate::geometry::constants::{CELL_AREA_INSET, HEADER_ROW_HEIGHT};
use crate::geometry::prim::Point;

use super::pane_set::{PaneSet, row_header_thickness_for};
use super::{Chrome, FrameKindTag, RecycledSlots};

/// Dispatch input for `Chrome::next` — which reuse-or-rebuild strategy the
/// orchestrator selected for this frame. Exhaustive: adding a variant breaks
/// every strategy arm at compile time. The blit fast-path is *not* here — it has
/// a two-outcome result and lives in [`Chrome::next_blit`] returning
/// [`super::BlitOutcome`], so it never widens into this shared dispatch.
#[derive(Clone, Copy)]
#[must_use = "FramePath dispatches Chrome::next; dropping it skips the chosen construction strategy"]
pub enum FramePath {
    /// Full rebuild walk. `prev = Some` recycles slot Vec allocations;
    /// `prev = None` is the first-frame path.
    Fresh,
    /// Reuse prev's slot vecs verbatim; refresh per-frame state only
    /// (theme). Content scope is the caller's `GridWork` verdict rather than
    /// state stored on `Chrome`, so `SlotsReuse` following a blit cannot
    /// inherit stale shift state. Requires `prev = Some`.
    SlotsReuse,
}

/// Outcome of [`Chrome::build`] (the `FramePath::Fresh` walk). The two
/// results are a built frame or a held attempt: a transient `BridgeFailed`
/// on any row-height or column-width read makes the whole geometry
/// untrustworthy (one slot's cursor depends on every earlier extent), so the
/// walk aborts before any pixel or cache state is committed. `Held` hands
/// the drained slot pool back so the retry reuses its allocations.
#[must_use = "a held Fresh build must become an AttemptOutcome::Held, never a painted frame"]
pub(crate) enum FreshBuild {
    Ready(Chrome),
    /// Geometry reads failed; `RecycledSlots` is the drained pool for reuse.
    Held(RecycledSlots),
}

impl Chrome {
    /// Build the next-frame `Chrome` for the reuse-or-rebuild strategies. The
    /// `path` argument selects which one; the body branches once and inlines
    /// the two constructors. The blit fast-path is separate
    /// ([`Self::next_blit`]) — it has a two-outcome result, not a strategy tag.
    ///
    ///   * `Fresh` — full rebuild. `prev = Some` recycles slot Vec
    ///     allocations; `None` is the first-frame path. See the
    ///     [module docs](crate::chrome) for build phases A-E.
    ///   * `SlotsReuse` — prev's slot vecs and header labels survive
    ///     verbatim; only the captured per-attempt scalars (theme, dpr,
    ///     model generation, header visibility) are refreshed. Caller
    ///     refreshes overlay state separately (`SelectionLayer::refresh` in
    ///     the orchestrator).
    ///
    /// `SlotsReuse` requires `prev = Some`; `None` falls through to `Fresh`
    /// defensively. The orchestrator proves `prev.is_some()` before selecting
    /// that path, but the fallback keeps `Chrome::next` total.
    /// Construct the next `Chrome` on the assumption that the model's
    /// geometry/config reads succeed (`Absent` overrides select the
    /// documented defaults). A transient `BridgeFailed` on any row-height
    /// or column-width read makes geometry untrustworthy. The orchestrator
    /// uses [`Chrome::build`] and handles `FreshBuild::Held` before paint.
    /// This wrapper is for healthy-model construction and slot reuse.
    ///
    /// # Panics
    ///
    /// Panics if a Fresh build fails because an extent read failed, an
    /// extent was invalid, or slot coordinates overflowed. This also applies
    /// to `SlotsReuse` when `prev` is `None` and construction falls back to Fresh.
    pub fn next(
        prev: Option<Chrome>,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        path: FramePath,
    ) -> Self {
        match path {
            FramePath::Fresh => {
                let recycled = match prev {
                    Some(c) => RecycledSlots::from_pane_set(c.pane_set),
                    None => RecycledSlots::default(),
                };
                match Self::build(model, inputs, recycled) {
                    FreshBuild::Ready(frame) => frame,
                    FreshBuild::Held(_) => {
                        panic!(
                            "Chrome::next(Fresh) requires a geometry-healthy model; \
                                bridge failures hold through Orchestrator::render_pending"
                        )
                    }
                }
            }
            FramePath::SlotsReuse => {
                let Some(mut prev) = prev else {
                    return Self::next(None, model, inputs, FramePath::Fresh);
                };
                // Slot vecs and header labels survive verbatim; every other
                // per-attempt scalar still refreshes from the newly captured
                // inputs so committed Chrome never lags behind the frame it
                // was actually built for. Grid paint scope remains the
                // caller's `GridWork` verdict rather than state stored here.
                prev.theme = Rc::clone(inputs.theme());
                prev.metrics = inputs.metrics();
                prev.model_generation = inputs.model_generation();
                prev.show_row_headers = inputs.show_row_headers();
                prev.show_col_headers = inputs.show_col_headers();
                prev.kind = FrameKindTag::SlotsReused;
                prev
            }
        }
    }

    /// Build a `FramePath::Fresh` candidate directly from a caller-supplied
    /// `recycled` pool, bypassing `Chrome::next`'s `prev`-derived recycling.
    /// `pub(crate)` so `Orchestrator::render_full_rebuild` can build the
    /// candidate from its own standing `spare_slots` pool without handing
    /// `prev`'s ownership to this call at all — `prev` stays fully intact
    /// (and, today, still committed in `self.last_frame`) for the whole
    /// duration of the build, rather than being drained into a `RecycledSlots`
    /// derived from it as the very first step. See `chrome::recycled_slots`'s
    /// module doc for the pool's cross-attempt lifecycle.
    pub(crate) fn build(
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        recycled: RecycledSlots,
    ) -> FreshBuild {
        // `inputs` is a `FrameInputs::capture` snapshot: sheet, view, freeze
        // counts, and header visibility already read exactly once and
        // validated (a bridge failure on any of them holds the whole paint
        // attempt before `Chrome::build` is ever called — see
        // `Orchestrator::render_pending`). No fallback default is needed or
        // read here.
        //
        // Row heights and column widths are *not* capture-time scalars: they
        // are walked per visible slot below. A transient `BridgeFailed` on any
        // of those reads makes the whole geometry untrustworthy — a slot's
        // cursor position depends on every earlier extent — so `build`
        // returns `Held` (with the drained slot pool handed back for the
        // retry to reuse) and the caller holds the attempt instead of
        // committing fabricated geometry.
        let view = inputs.view();
        let sheet = inputs.sheet();

        // Visibility is modelled as thickness 0. CELL_AREA_INSET only reserves
        // the 1-px chrome border that draw_corner_box strokes; a hidden strip
        // paints no such border, so its thickness AND inset collapse to 0 and
        // cells reclaim the full edge (cell_origin follows from origin_x/_y).
        let show_row = inputs.show_row_headers();
        let show_col = inputs.show_col_headers();

        // Phase A — frozen counts, already captured.
        let frozen_row_count = inputs.frozen_rows();
        let frozen_col_count = inputs.frozen_cols();

        let mut pane_set = PaneSet::with_recycled(recycled);

        // Phase B — row walk, bounded by the model's grid (Excel's
        // LAST_ROW/LAST_COLUMN by default; finite models override).
        let canvas = inputs.size();
        let (canvas_w, canvas_h) = canvas.to_logical_extent();
        let last_row = model.last_row(sheet);
        let last_column = model.last_column(sheet);
        let origin_y = if show_col {
            HEADER_ROW_HEIGHT + CELL_AREA_INSET
        } else {
            0
        };
        if !pane_set.fill_rows(
            model,
            sheet,
            frozen_row_count,
            origin_y,
            view.top_row,
            last_row,
            canvas_h,
        ) {
            return FreshBuild::Held(RecycledSlots::from_pane_set(pane_set));
        }

        // Phase C — measure row_header_thickness from the last visible row label.
        let row_header_thickness = row_header_thickness_for(
            &pane_set.rows.scroll,
            frozen_row_count,
            view.top_row,
            show_row,
        );

        // Phase D — col walk uses the measured width to anchor `origin_x`.
        let origin_x = if show_row {
            row_header_thickness + CELL_AREA_INSET
        } else {
            0
        };
        if !pane_set.fill_cols(
            model,
            sheet,
            frozen_col_count,
            origin_x,
            view.left_column,
            last_column,
            canvas_w,
        ) {
            return FreshBuild::Held(RecycledSlots::from_pane_set(pane_set));
        }

        // Data-driven header labels in walk_header_strip (frozen ++ scroll)
        // order so header_strip can zip slots <-> labels positionally.
        pane_set.row_header_labels =
            PaneSet::resolve_row_labels(model, sheet, &pane_set.rows.frozen, &pane_set.rows.scroll);
        pane_set.col_header_labels =
            PaneSet::resolve_col_labels(model, sheet, &pane_set.cols.frozen, &pane_set.cols.scroll);

        // Phase E — assemble. `cell_origin` reuses the locals from B/D so
        // there's a single source of truth for the cell-area top-left.
        let col_header_thickness = if show_col { HEADER_ROW_HEIGHT } else { 0 };
        let cell_origin = Point {
            x: origin_x,
            y: origin_y,
        };

        FreshBuild::Ready(Chrome {
            sheet,
            pane_set,
            row_header_thickness,
            col_header_thickness,
            cell_origin,
            metrics: inputs.metrics(),
            theme: Rc::clone(inputs.theme()),
            model_generation: inputs.model_generation(),
            show_row_headers: show_row,
            show_col_headers: show_col,
            kind: FrameKindTag::Fresh,
        })
    }
}
