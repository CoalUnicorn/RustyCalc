use std::rc::Rc;

use crate::CanvasModel;
use crate::chrome::Chrome;
#[cfg(feature = "dev-diagnostics")]
use crate::chrome::GridLayout;
use crate::frame::work::RowSpan;
use crate::frame::{BlitPlan, FrameTrace};
use crate::geometry::prim::Axis;
use crate::painter::{BlitPainter, Painter};
use crate::renderer::prepared::{GridCacheCommit, PreparedGrid};

use super::RendererCore;
#[cfg(feature = "dev-diagnostics")]
use super::diagnostics;

// The successful arm deliberately owns the fixed-size cache commit; boxing
// it would add one allocation to every retained-paint frame.
#[must_use]
#[allow(clippy::large_enum_variant)]
pub(crate) enum GridPaintOutcome {
    /// The grid executed; owns the aggregate cache commit for the
    /// completion boundary to install.
    Committed(GridCacheCommit),
    /// A bridge failure held the whole grid before any paint or cache
    /// mutation; no commit exists.
    Held,
}

/// Which header strips a grid execution repaints. A scroll blit shifts only
/// the scroll-axis strip's pixels, so repainting the cross-axis strip would
/// be work the frame never invalidated.
#[derive(Clone, Copy)]
pub(super) enum GridHeaderScope {
    Both,
    Axis(Axis),
}

impl GridHeaderScope {
    pub(super) fn paints(self, axis: Axis) -> bool {
        match (self, axis) {
            (Self::Both, Axis::Row)
            | (Self::Both, Axis::Column)
            | (Self::Axis(Axis::Row), Axis::Row)
            | (Self::Axis(Axis::Column), Axis::Column) => true,
            (Self::Axis(Axis::Row), Axis::Column) | (Self::Axis(Axis::Column), Axis::Row) => false,
        }
    }
}

// Layer-facing wrappers
//
// `GridRenderer` and `OverlayRenderer` each own a `RendererCore` and re-export
// only the operations their layer is allowed to perform. `LayerOps` is the
// paint-backend-agnostic subset (just `resize_for_dpr`); the Canvas-2D
// passthroughs (`ctx_ref` for the layer's own clear/fill, `invalidate_paint_cache`)
// live as inherent methods on the `<CanvasPainter>` impl so a future SvgPainter
// can satisfy `LayerOps` without `web_sys`.

/// Backend-agnostic resize hook. Called by `LayerBase::resize` whenever the
/// backing store's DPR changes; everything else stays on the wrapper's
/// inherent surface. `Painter` ties the renderer's painter type to the
/// `LayerBase`'s `Surface::P` at the type level.
pub trait LayerOps {
    type Painter: Painter;
    fn resize_for_dpr(&mut self, dpr: f64);
}

pub struct GridRenderer<P: Painter> {
    core: RendererCore<P>,
}

impl<P: Painter> GridRenderer<P> {
    pub fn render_grid(&self, model: &dyn CanvasModel, frame: &Chrome) -> bool {
        self.core.render_grid(model, frame)
    }

    pub(crate) fn execute_grid(&self, model: &dyn CanvasModel, frame: &Chrome) -> GridPaintOutcome {
        self.core.execute_grid(model, frame)
    }

    pub fn render_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> bool {
        self.core.render_grid_damage(model, frame, spans)
    }

    pub(crate) fn execute_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> GridPaintOutcome {
        self.core.execute_grid_damage(model, frame, spans)
    }

    /// See [`RendererCore::fetch_show_grid`]. `pub(crate)` so the layer's
    /// grid paint entries can hold before a bg fill or pixel shift.
    pub(crate) fn fetch_show_grid(&self, model: &dyn CanvasModel, sheet: u32) -> Option<bool> {
        self.core.fetch_show_grid(model, sheet)
    }

    /// See [`RendererCore::prepare_fresh_grid`]. `pub(crate)`: an
    /// execution detail of the Fresh atomic paint path, reached only
    /// through [`crate::surface::LayerBase::paint_grid_fresh`].
    pub(crate) fn prepare_fresh_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
    ) -> Option<PreparedGrid> {
        self.core.prepare_fresh_grid(model, frame)
    }

    /// See [`RendererCore::execute_fresh_grid`].
    pub(crate) fn execute_fresh_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        prepared: PreparedGrid,
    ) -> GridCacheCommit {
        self.core.execute_fresh_grid(model, frame, prepared)
    }

    /// Mark retained pixels and cell buffers stale while keeping their
    /// allocations available for the next successful grid preparation.
    pub fn invalidate_grid_buffers(&self) {
        self.core.grid_cache.invalidate_buffers();
    }

    pub fn reset_trace(&self) {
        self.core.reset_trace();
    }

    pub fn trace(&self) -> FrameTrace {
        self.core.trace()
    }

    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn set_diag_enabled(&self, enabled: bool) {
        self.core.set_diag_enabled(enabled);
    }

    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn last_diag(&self) -> Option<diagnostics::FrameDiagnostics> {
        self.core.last_diag()
    }

    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn diag_reset_capture(&self) {
        self.core.diag_reset_capture();
    }
    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn diag_begin_attempt(
        &self,
        delta: diagnostics::DiagDeltaKind,
        rebuild_reason: Option<crate::frame::RebuildReason>,
    ) {
        self.core.diag_begin_attempt(delta, rebuild_reason);
    }

    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn diag_blit(
        &self,
        plan: &BlitPlan,
        result: diagnostics::DiagBlitResultTag,
        cold_cache: Option<bool>,
        previous: Option<GridLayout>,
        candidate: GridLayout,
    ) {
        self.core
            .diag_blit(plan, result, cold_cache, previous, candidate);
    }

    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn publish_diag(&self, completion: diagnostics::DiagCompletion) {
        self.core.publish_diag(completion);
    }

    pub fn for_layer(painter: Rc<P>) -> Self {
        Self {
            core: RendererCore::for_layer(painter),
        }
    }

    pub fn painter(&self) -> &P {
        self.core.painter()
    }

    pub fn invalidate_paint_cache(&mut self) {
        self.core.invalidate_paint_cache();
    }
}

impl<P: BlitPainter> GridRenderer<P> {
    /// See [`RendererCore::render_grid_blit`]. Requires `BlitPainter` (not
    /// just `Painter`) because this is now the one call that both shifts the
    /// kept band and paints the revealed strip — `LayerBase::paint_grid_blit`
    /// no longer issues `Painter::blit` itself.
    pub fn render_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> bool {
        self.core.render_grid_blit(model, frame, plan)
    }

    pub(crate) fn execute_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> GridPaintOutcome {
        self.core.execute_grid_blit(model, frame, plan)
    }

    pub(crate) fn commit_grid_cache(&self, commit: GridCacheCommit) {
        self.core.commit_grid_cache(commit);
    }
}

impl<P: Painter> LayerOps for GridRenderer<P> {
    type Painter = P;
    fn resize_for_dpr(&mut self, dpr: f64) {
        self.core.resize_for_dpr(dpr);
    }
}

pub struct OverlayRenderer<P: Painter> {
    core: RendererCore<P>,
}

impl<P: Painter> OverlayRenderer<P> {
    pub fn for_layer(painter: Rc<P>) -> Self {
        Self {
            core: RendererCore::for_layer(painter),
        }
    }

    pub fn painter(&self) -> &P {
        self.core.painter()
    }

    pub fn render_header_highlights(
        &self,
        axis: crate::geometry::prim::Axis,
        frame: &Chrome,
        selection_range: crate::address::RCRange,
    ) {
        self.core
            .render_header_highlights(axis, frame, selection_range);
    }

    pub fn repaint_active_cell(
        &self,
        model: &dyn CanvasModel,
        cell: crate::address::CellCoord,
        frame: &Chrome,
    ) {
        self.core.repaint_active_cell(model, cell, frame);
    }
}

impl<P: Painter> LayerOps for OverlayRenderer<P> {
    type Painter = P;
    fn resize_for_dpr(&mut self, dpr: f64) {
        self.core.resize_for_dpr(dpr);
    }
}
