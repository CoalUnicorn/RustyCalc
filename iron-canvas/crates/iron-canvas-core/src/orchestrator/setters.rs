use std::rc::Rc;

use crate::CanvasModel;
use crate::decoration::{DecorationId, Layer};
use crate::frame::work::{PendingWork, RowSpan};
use crate::geometry::CanvasMetrics;
use crate::painter::BlitPainter;
use crate::render_overlays::RenderOverlays;
use crate::surface::Surface;
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};

use super::Orchestrator;

impl<S> Orchestrator<S>
where
    S: Surface,
    S::P: BlitPainter,
{
    /// Resize both layers in one call. No public per-layer resize, so
    /// callers can't leave the pair half-sized. Self-invalidating: a real
    /// size or DPR change forces the next `render_pending` to `Fresh` — no
    /// caller needs a follow-up `request_repaint()`.
    ///
    /// Takes [`CanvasMetrics`]: the host boundary parses the raw
    /// width/height/DPR once (`CanvasMetrics::new`), and every backend and
    /// geometry walk downstream reads the validated value.
    pub fn resize(&mut self, metrics: CanvasMetrics) {
        if self.metrics == Some(metrics) {
            return;
        }
        self.metrics = Some(metrics);
        self.grid.resize(metrics);
        self.overlay.resize(metrics);
        // A backing-store resize may clear both canvases (Canvas2D), so
        // geometry invalidation must be atomic with the resize itself.
        self.last_frame = None;
        self.pending.mark_geometry();
        self.pending.mark_overlay();
    }

    /// Conservative repaint blanket. Marks geometry so the next
    /// `render_pending` falls to `FullRebuild` — the cheaper `ChangedCells` /
    /// `ScrollBlit` arms gate on geometry being clean. Adds geometry plus
    /// overlay work; it never *adds* content work, which is reserved for
    /// real cell-value changes via `mark_content_dirty`.
    ///
    /// Content and view work already queued is preserved rather than
    /// cleared. Dropping it here would strand an edit that arrived earlier
    /// in the same tick: the escalated `Fresh` frame would rebuild geometry
    /// but lose the content intent that the same Fresh transaction must
    /// subsume.
    ///
    /// `last_frame` is deliberately preserved (see `set_model`'s matching
    /// comment): the geometry work marked below already forces `Fresh`, so
    /// keeping the old committed frame only keeps query geometry coherent
    /// with the old pixels until that Fresh paint lands.
    pub fn request_repaint(&mut self) {
        self.pending.mark_geometry();
        self.pending.mark_overlay();
    }

    /// Bulk-push every overlay primitive in one comparison. The per-field
    /// setters each mark overlay work independently; folding them into one
    /// pass lets the Leptos host's per-frame reactive memo cost a single
    /// mark instead of four.
    pub fn set_overlays(&mut self, overlays: RenderOverlays) {
        if self.decos.set_overlays(overlays) {
            self.pending.mark_overlay();
        }
    }

    pub fn set_extend_to(&mut self, target: Option<AutofillTarget>) {
        if self.decos.set_extend_to(target) {
            self.pending.mark_overlay();
        }
    }

    pub fn set_clipboard(&mut self, area: Option<SheetArea>) {
        if self.decos.set_clipboard(area) {
            self.pending.mark_overlay();
        }
    }

    pub fn set_point_range(&mut self, range: Option<RCRange>) {
        if self.decos.set_point_range(range) {
            self.pending.mark_overlay();
        }
    }

    pub fn set_formula_refs(&mut self, refs: Vec<FormulaRef>) {
        if self.decos.set_formula_refs(refs) {
            self.pending.mark_overlay();
        }
    }

    /// Install a consumer-owned overlay decoration above every built-in.
    /// The layer paints from the next frame onward — never retroactively
    /// onto a frame already emitted — and its `hit_test` runs before every
    /// built-in zone, so returning `Some` at the autofill-handle pixel
    /// steals the handle drag: stay paint-only (the trait default) unless
    /// that shadowing is intended. The registry holds a strong `Rc`; keep
    /// a typed clone, mutate through interior mutability, and call
    /// [`Self::request_overlay_repaint`] after each change — unlike the
    /// built-in setters, nothing here compares state for you.
    pub fn add_decoration(&mut self, layer: Rc<dyn Layer>) -> DecorationId {
        let id = self.decos.add_custom(layer);
        self.pending.mark_overlay();
        id
    }

    /// Remove a custom decoration. Removal is explicit — a layer whose
    /// consumer handle was dropped still participates in the paint and hit
    /// loops (as a no-op) until removed here. Marks overlay work only when
    /// the id was found, so a stale-id call cannot trigger a repaint.
    pub fn remove_decoration(&mut self, id: DecorationId) -> bool {
        let removed = self.decos.remove_custom(id);
        if removed {
            self.pending.mark_overlay();
        }
        removed
    }

    /// Push a theme. Value-compares against `self.theme` and, on change,
    /// marks both layers dirty. `Chrome::classify` rejects a theme-mismatched
    /// frame itself, so the next paint reaches `Fresh` through the
    /// classifier's verdict — no out-of-band `last_frame` drop needed here.
    ///
    /// Deliberately does *not* invalidate the grid paint cache (Stage 6,
    /// Gate A): since the only route out of that classifier rejection is a
    /// `Fresh` walk, and `Layer::paint_grid_fresh` invalidates after its
    /// grid prepares and before its first draw, an eager call here is a
    /// second, redundant painter state transition. Leaving it out also stops
    /// a *held* theme Fresh from touching the painter at all. Cell repaint
    /// coverage does not depend on it either way: `invalidate_paint_cache`
    /// only resets painter ctx state, and a Fresh candidate forces
    /// `RepaintPlan::Full` without consulting the content-keyed fingerprint
    /// tree during full-grid preparation.
    pub fn set_theme(&mut self, theme: CanvasTheme) {
        if theme != *self.theme {
            self.theme = Rc::new(theme);
            self.pending.mark_geometry();
            self.pending.mark_overlay();
        }
    }

    pub fn set_theme_variables(&mut self, vars: ThemeVariables) {
        self.set_theme(vars.build());
    }

    /// Push a new data model. No `Rc::ptr_eq` dedupe: every call is
    /// treated as a change and forces the next paint to Fresh. JS-side
    /// typically pushes once per workbook, so the cost is one worst-case
    /// repaint after a redundant push.
    pub fn set_model(&mut self, model: Rc<dyn CanvasModel>) {
        self.model = Some(model);
        // Wrapping: correctness never depends on uniqueness after a wrap
        // (see the field doc) — this exists to classify an ordinary model
        // replacement, not to gate repaint.
        self.model_generation = self.model_generation.wrapping_add(1);
        // `last_frame` is deliberately preserved (not dropped) here: the
        // geometry + all-content + overlay work marked below already forces
        // the next paint to `Fresh` regardless of `Chrome::classify`'s
        // verdict, so retaining the old committed frame only keeps query
        // geometry (`hit_test`, `cell_rect`, ...) coherent with the old
        // pixels for the window between this call and that Fresh paint —
        // including if the new model's scalar capture temporarily fails.
        // The one setter that *discards* queued work instead of adding to
        // it: row-scoped work recorded against the outgoing model names
        // nothing in the incoming one. Replaced wholesale by the
        // worst-case value, which subsumes anything the old work could
        // have asked for.
        self.pending = PendingWork::default();
        self.pending.mark_geometry();
        self.pending.mark_all_content();
        self.pending.mark_overlay();
    }

    /// Mark the overlay dirty. Selection, autofill, formula-ref, and
    /// clipboard signals funnel through here; grid escalation on scroll /
    /// freeze / sheet / size change is owned by `render_pending` via
    /// `Chrome::classify`, not duplicated at the callsite.
    pub fn request_overlay_repaint(&mut self) {
        self.pending.mark_overlay();
    }

    /// Typed cell-content-changed signal. Marks all visible content dirty so
    /// the next `render_pending` refetches its values from the model via the
    /// grid-wide `SlotsReuse` arm —
    /// fixes the recalc bug where a formula dependent on an edited
    /// cell silently kept painting the stale cached value.
    pub fn mark_content_dirty(&mut self) {
        self.pending.mark_all_content();
    }

    /// Row-scoped `mark_content_dirty`: also names the damaged rows so
    /// `plan_frame` can clip the repaint to full-width bands. All escalation
    /// (cross-sheet rows, span-count cap, or meeting all-content work)
    /// belongs to `ContentWork`'s merge table, not to this callsite.
    ///
    /// Row precision chooses the `Damage` strategy; when that strategy is
    /// ineligible, planning widens the work to `AllContent`.
    pub fn mark_rows_damaged(&mut self, sheet: u32, span: RowSpan) {
        self.pending.mark_rows(sheet, span);
    }

    /// The view moved: scroll, selection, active cell, or sheet. Marks view
    /// plus overlay atomically — a view change always repositions overlay
    /// primitives, and splitting the two would let a caller queue movement
    /// that never repaints the selection rectangle.
    ///
    /// Intent only. Whether the movement shifts pixels (`ScrollBlit`), stays
    /// inside the painted frame (`OverlayOnly`), or needs a rebuild (`FullRebuild`) is
    /// `plan_frame`'s geometric verdict, not the caller's.
    pub fn view_changed(&mut self) {
        self.pending.mark_view();
        self.pending.mark_overlay();
    }
}
