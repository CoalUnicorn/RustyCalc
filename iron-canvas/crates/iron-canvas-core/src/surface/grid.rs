use crate::CanvasModel;
use crate::chrome::Chrome;
use crate::frame::BlitPlan;
use crate::frame::work::RowSpan;
use crate::painter::{BlitPainter, PaintColor, Painter};
use crate::renderer::{GridCacheCommit, GridPaintOutcome, GridRenderer};

use super::{LayerBase, Surface, full_canvas_rect};

// Grid-layer specialization. Lives here, not on `GridRenderer<P>`, because the
// full-canvas bg fill is the *surface's* concern — the renderer paints cells
// and chrome through the painter, but the layer-wide clear is a once-per-frame
// pixel op the surface owns alongside its `present()`.
impl<S> LayerBase<S, GridRenderer<S::P>>
where
    S: Surface,
    S::P: BlitPainter,
{
    /// SlotsReuse grid paint: prior frame's pixels stay, so fingerprint-skip
    /// wins are preserved and no full-canvas background fill runs.
    /// Only ever called with a `SlotsReused`/`Blitted`-kind `frame` — see
    /// [`Self::paint_grid_fresh`] for the atomic `Fresh` counterpart, which
    /// cannot share this method's retained-pixel shape (the bg fill
    /// alone would already be an observable op on a would-be-held attempt).
    ///
    /// Returns the grid-wide committed or held outcome.
    pub(crate) fn paint_grid(
        &mut self,
        model: &dyn CanvasModel,
        frame: &Chrome,
    ) -> GridPaintOutcome {
        self.renderer.execute_grid(model, frame)
    }

    /// Fresh-frame atomic grid paint: prepares the visible grid first —
    /// bulk fetch and bridge-check only, zero painter interaction — and
    /// only once every one is confirmed clean does anything reach the
    /// painter at all, including the paint-cache invalidation and the
    /// full-canvas background fill. A held outcome leaves the painter
    /// untouched, including group brackets.
    ///
    /// Order on the healthy path (invalidate, then bg fill, then cells)
    /// matches what the pre-Stage-4 unconditional call sequence produced,
    /// so a clean Fresh paint's op stream is unchanged.
    pub(crate) fn paint_grid_fresh(
        &mut self,
        model: &dyn CanvasModel,
        frame: &Chrome,
    ) -> GridPaintOutcome {
        // Grid-line visibility is per-execution config (a toggle repaints
        // without a geometry rebuild); read it before any painter op — the
        // bg fill below is a pixel op, so a BridgeFailed must hold first.
        if self.renderer.fetch_show_grid(model, frame.sheet).is_none() {
            return GridPaintOutcome::Held;
        }
        let Some(prepared) = self.renderer.prepare_fresh_grid(model, frame) else {
            return GridPaintOutcome::Held;
        };
        self.renderer.invalidate_paint_cache();
        self.surface.painter().rect_fill(
            full_canvas_rect(frame.canvas_size()),
            PaintColor::from_theme_str(&frame.theme.cell_bg),
        );
        GridPaintOutcome::Committed(self.renderer.execute_fresh_grid(model, frame, prepared))
    }

    /// Scroll-blit grid paint: `RendererCore::render_grid_blit` prepares
    /// every required address strip (fetching and bridge-validating all of
    /// them before a single pixel moves), performs `BlitPlan::shift`, and
    /// paints the revealed strip, all in that one call. If any required
    /// fetch fails, the whole frame is abandoned as a no-op: no
    /// shift, no paint — the renderer never gets far enough to call
    /// `Painter::blit`. This is deliberate and minimal — shifting pixels and
    /// then discovering the fetch failed is the bug being fixed (it strands
    /// stale, misplaced pixels in the revealed strip). A fallback full
    /// repaint is intentionally NOT attempted here; a future frame
    /// reconciles once the bridge recovers via the normal frame-kind
    /// dispatch, so a reader should not "upgrade" the returned
    /// `GridPaintOutcome::Held` without re-deriving why it is sufficient.
    pub(crate) fn paint_grid_blit(
        &mut self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> GridPaintOutcome {
        self.renderer.execute_grid_blit(model, frame, plan)
    }

    /// Damage grid paint: prior pixels stay; only the damaged full-width
    /// row bands refetch and repaint. No full-canvas bg fill by design.
    ///
    /// Returns the grid-wide committed or held outcome.
    pub(crate) fn paint_grid_damage(
        &mut self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> GridPaintOutcome {
        self.renderer.execute_grid_damage(model, frame, spans)
    }

    pub fn invalidate_paint_cache(&mut self) {
        self.renderer.invalidate_paint_cache();
    }

    pub fn invalidate_grid_buffers(&self) {
        self.renderer.invalidate_grid_buffers();
    }

    pub(crate) fn commit_grid_cache(&self, commit: GridCacheCommit) {
        self.renderer.commit_grid_cache(commit);
    }
}
