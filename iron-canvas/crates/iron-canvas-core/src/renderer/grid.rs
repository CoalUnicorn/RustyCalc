use crate::CanvasModel;
use crate::chrome::Chrome;
use crate::frame::BlitPlan;
use crate::frame::work::RowSpan;
use crate::geometry::prim::Axis;
use crate::painter::{BlitPainter, GroupClass, Painter};
use crate::renderer::prepared::{GridCacheCommit, PreparedGrid};

use super::RendererCore;
use super::layers::{GridHeaderScope, GridPaintOutcome};

impl<P: Painter> RendererCore<P> {
    /// Paint all visible grid segments for a slot-reusing frame.
    ///
    /// Returns `true` when any segment reports a bridge failure. A held call
    /// performs no painter work and installs no cache state; `false` means the
    /// attempt completed (including a fingerprint skip) and any owned cache
    /// commit was installed.
    pub fn render_grid(&self, model: &dyn CanvasModel, frame: &Chrome) -> bool {
        match self.execute_grid(model, frame) {
            GridPaintOutcome::Committed(commit) => {
                self.commit_grid_cache(commit);
                false
            }
            GridPaintOutcome::Held => true,
        }
    }

    /// The grid group sequence every strategy shares:
    /// `Grid -> Cells -> FrozenSep -> Headers -> Corner`. Only the work
    /// inside Cells differs between ChangedCells, DamagedRows, FullRebuild,
    /// and ScrollBlit,
    /// so `execute_cells` owns that and nothing else; its typed result
    /// passes straight back out.
    ///
    /// `execute_cells` runs only after whole-grid preflight, so the groups
    /// opened here always close.
    fn execute_grid_shell<T>(
        &self,
        frame: &Chrome,
        headers: GridHeaderScope,
        execute_cells: impl FnOnce() -> T,
    ) -> T {
        self.painter.begin_group(GroupClass::Grid);

        self.painter.begin_group(GroupClass::Cells);
        let cells = execute_cells();
        self.painter.end_group();

        // Frozen separators paint AFTER cells so the thick divider wins
        // its pixels over the rightmost/bottommost frozen cell's grid stroke.
        self.painter.begin_group(GroupClass::FrozenSep);
        self.draw_frozen_separators(frame);
        self.painter.end_group();

        self.painter.begin_group(GroupClass::Headers);
        for axis in [Axis::Row, Axis::Column] {
            let thickness = match axis {
                Axis::Row => frame.row_header_thickness,
                Axis::Column => frame.col_header_thickness,
            };
            if headers.paints(axis) && thickness > 0 {
                self.render_headers_base(axis, frame);
            }
        }
        self.painter.end_group();

        self.draw_corner_box_if_needed(frame);

        self.painter.end_group();
        cells
    }

    pub(crate) fn execute_grid(&self, model: &dyn CanvasModel, frame: &Chrome) -> GridPaintOutcome {
        if self.fetch_show_grid(model, frame.sheet).is_none() {
            return GridPaintOutcome::Held;
        }
        let Some(prepared) = self.prepare_full_grid(model, frame) else {
            return GridPaintOutcome::Held;
        };
        let commit = self.execute_grid_shell(frame, GridHeaderScope::Both, || {
            self.execute_prepared_grid(frame, prepared)
        });
        GridPaintOutcome::Committed(commit)
    }

    /// Preflight every grid segment for a Fresh frame without touching the
    /// painter or committed cache state. A bridge failure returns `None` only
    /// after all earlier prepared bundles have been recycled.
    pub(crate) fn prepare_fresh_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
    ) -> Option<PreparedGrid> {
        self.prepare_full_grid(model, frame)
    }

    /// Execute a fully preflighted Fresh grid. The returned owned commit is
    /// installed only by the caller's successful completion boundary. The
    /// grid-line config was fetched by the caller before any painter op.
    pub(crate) fn execute_fresh_grid(
        &self,
        _model: &dyn CanvasModel,
        frame: &Chrome,
        prepared: PreparedGrid,
    ) -> GridCacheCommit {
        self.execute_grid_shell(frame, GridHeaderScope::Both, || {
            self.execute_prepared_grid(frame, prepared)
        })
    }

    /// Combined Fresh prepare and execute. Returns `true` with zero painter
    /// interaction on any bridge failure; otherwise commits the whole grid.
    pub fn render_grid_fresh(&self, model: &dyn CanvasModel, frame: &Chrome) -> bool {
        if self.fetch_show_grid(model, frame.sheet).is_none() {
            return true;
        }
        let Some(prepared) = self.prepare_fresh_grid(model, frame) else {
            return true;
        };
        let cache_commit = self.execute_fresh_grid(model, frame, prepared);
        self.commit_grid_cache(cache_commit);
        false
    }

    /// Damage variant: prior pixels stay; only the damaged full-width row
    /// bands refetch and repaint across every intersecting grid segment. The
    /// outer sequence is not restated
    /// here — it is the shared [`Self::execute_grid_shell`] `render_grid`
    /// also runs through, which is what guarantees the frozen separators
    /// still paint after the cells (winning their pixels back from the
    /// band's re-stroked grid lines at the freeze boundary).
    ///
    /// Returns `true` when any strip reports a bridge failure. The whole grid
    /// is held atomically and no cache state is installed; see
    /// [`Self::render_grid`] for the same SlotsReuse contract.
    pub fn render_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> bool {
        match self.execute_grid_damage(model, frame, spans) {
            GridPaintOutcome::Committed(commit) => {
                self.commit_grid_cache(commit);
                false
            }
            GridPaintOutcome::Held => true,
        }
    }

    pub(crate) fn execute_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> GridPaintOutcome {
        if self.fetch_show_grid(model, frame.sheet).is_none() {
            return GridPaintOutcome::Held;
        }
        let Some(prepared) = self.prepare_damage_grid(model, frame, spans) else {
            return GridPaintOutcome::Held;
        };
        let commit = self.execute_grid_shell(frame, GridHeaderScope::Both, || {
            self.execute_prepared_grid(frame, prepared)
        });
        GridPaintOutcome::Committed(commit)
    }

    /// Paint the header corner box, gated for *correctness*: at thickness 0 it
    /// would still stroke 0.5px border lines spanning the full canvas.
    fn draw_corner_box_if_needed(&self, frame: &Chrome) {
        if frame.row_header_thickness > 0 && frame.col_header_thickness > 0 {
            self.painter.begin_group(GroupClass::Corner);
            self.draw_corner_box(frame);
            self.painter.end_group();
        }
    }

    /// Resolve the per-sheet grid-line toggle once per grid execution and
    /// cache it for the hot per-cell `paint_borders_grid` walk. `Absent`
    /// (no override) selects the documented default — show, matching
    /// Excel's default-on. `BridgeFailed` returns `None`: the caller must
    /// hold the attempt before any painter op — painting grid lines (or not)
    /// from a fabricated answer is a config lie, not a pixel choice.
    ///
    /// `sheet` is the committed frame's own sheet (`frame.sheet`), not
    /// another `CanvasModel::get_selected_sheet()` read — the gridline
    /// lookup runs once per grid execution and must agree with the geometry
    /// it is painting over, not with whatever the live model reports this
    /// instant.
    pub(crate) fn fetch_show_grid(&self, model: &dyn CanvasModel, sheet: u32) -> Option<bool> {
        let show = match model.get_show_grid_lines(sheet) {
            crate::types::fetched::Fetched::Value(v) => v,
            crate::types::fetched::Fetched::Absent => true,
            crate::types::fetched::Fetched::BridgeFailed => return None,
        };
        self.frame_cache.show_grid.set(show);
        Some(show)
    }
}

// `render_grid_blit` needs `Painter::blit` (via `BlitPainter`) to shift the
// kept band itself, so it lives in its own `BlitPainter`-bounded block,
// mirroring `GridRenderer<P: BlitPainter>`'s own split below.
impl<P: BlitPainter> RendererCore<P> {
    /// Preflight every candidate-derived address strip before applying the
    /// plan's single pixel shift. A bridge failure returns `true` without a
    /// blit, group bracket, paint, or cache mutation. Compatible shifts repaint
    /// only the scroll-axis header; full-grid fallback repaints both headers.
    pub fn render_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> bool {
        match self.execute_grid_blit(model, frame, plan) {
            GridPaintOutcome::Committed(cache_commit) => {
                self.commit_grid_cache(cache_commit);
                false
            }
            GridPaintOutcome::Held => true,
        }
    }

    pub(crate) fn execute_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> GridPaintOutcome {
        if self.fetch_show_grid(model, frame.sheet).is_none() {
            return GridPaintOutcome::Held;
        }
        let Some(prepared) = self.prepare_blit_grid(model, frame, plan) else {
            return GridPaintOutcome::Held;
        };
        let is_shift = matches!(&prepared, PreparedGrid::Blit { .. });

        // The shifts stay ahead of the shell: a held attempt must move zero
        // pixels, and a successful one must move them all before the first
        // group opens, or the repainted strips would land under stale pixels.
        if is_shift {
            self.painter.blit(plan.shift.src, plan.shift.dst);
        }

        let headers = if is_shift {
            GridHeaderScope::Axis(plan.axis)
        } else {
            GridHeaderScope::Both
        };
        let cache_commit = self.execute_grid_shell(frame, headers, || {
            self.execute_prepared_grid(frame, prepared)
        });
        GridPaintOutcome::Committed(cache_commit)
    }
}
