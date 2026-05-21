//! Layer machinery — backend-agnostic.
//!
//! `Surface` wraps a drawing target (HTML canvas, Cairo surface,
//! in-memory recorder). One Surface owns one backing store + the painter
//! that draws into it; the renderer borrows the painter via `painter()`.
//!
//! `LayerBase<S, R>` stacks a `PaintGate` (typed `GridSignals` dirty bits)
//! over a Surface + a layer-specific renderer. Layer-specific paint methods
//! live on the renderer wrappers (`GridRenderer<P>` / `OverlayRenderer<P>`),
//! reached through `LayerBase::renderer`.

use std::cell::Cell;
use std::rc::Rc;

use crate::chrome::{BlitPlan, Chrome, PaneRegionMask};
use crate::decoration::{selection::SelectionLayer, Layer};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Axis, Point};
use crate::geometry::CanvasSize;
use crate::painter::{BlitPainter, PaintColor, Painter};
use crate::renderer::{GridRenderer, LayerOps, OverlayRenderer};
use crate::signal::GridSignals;
use crate::CanvasModel;

/// Drawing target abstraction. Production wasm holds one Surface per
/// `<canvas>` (grid + overlay); a Cairo backend would hold one per
/// `DrawingArea`; the in-memory test surface holds a `RecorderPainter`.
///
/// Surfaces own their painter outright; renderers receive a cloned handle
/// via `clone_painter` at construction so paint methods don't need to
/// re-borrow through the surface on every call.
pub trait Surface {
    type P: Painter + BlitPainter;

    /// Borrow the painter. `&self` works because painter trait methods
    /// take `&self` and rely on interior mutability for their state caches.
    fn painter(&self) -> &Self::P;

    /// Hand the renderer its own owning handle to the same painter.
    ///
    /// `Rc` is the deliberate ownership primitive — canvas-style backends
    /// (Canvas-2D, Cairo, recorder) all run single-threaded on the same
    /// task that owns the orchestrator. A multi-threaded backend would
    /// need a different trait shape, not a swap to `Arc` here.
    fn clone_painter(&self) -> Rc<Self::P>;

    /// Resize the backing store. `dpr` here scales the backing pixel
    /// buffer (e.g. `canvas.width = css.w * dpr`) — it does *not* set the
    /// painter's transform matrix. That side runs separately via
    /// `LayerBase::resize` → `LayerOps::resize_for_dpr`. Two effects, one
    /// shared input; each backend resizes only what it owns.
    fn resize(&mut self, css: CanvasSize, dpr: i32);

    /// Flush the rendered frame. Canvas-2D auto-presents (no-op);
    /// Cairo / off-screen image backends flush here.
    fn present(&self);
}

pub struct PaintGate {
    signals: Cell<GridSignals>,
    paint_count: Cell<u32>,
}

impl PaintGate {
    pub fn new() -> Self {
        Self {
            signals: Cell::new(GridSignals::EMPTY),
            paint_count: Cell::new(0),
        }
    }

    pub fn raise(&self, sig: GridSignals) {
        self.signals.set(self.signals.get() | sig);
    }

    pub fn drain(&self) -> GridSignals {
        let drained = self.signals.replace(GridSignals::EMPTY);
        if !drained.is_empty() {
            self.paint_count.set(self.paint_count.get() + 1);
        }
        drained
    }

    pub fn should_paint(&self) -> bool {
        !self.drain().is_empty()
    }

    /// Non-empty-drain tick. Cross-crate test surface; production must not
    /// branch on it. `cfg(test)` doesn't cross crate boundaries, so the
    /// accessor stays callable for tests in sibling crates.
    #[doc(hidden)]
    pub fn paint_count(&self) -> u32 {
        self.paint_count.get()
    }
}

impl Default for PaintGate {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LayerBase<S, R>
where
    S: Surface,
    R: LayerOps<Painter = S::P>,
{
    pub(crate) surface: S,
    gate: PaintGate,
    pub(crate) renderer: R,
}

impl<S, R> LayerBase<S, R>
where
    S: Surface,
    R: LayerOps<Painter = S::P>,
{
    pub fn new(surface: S, renderer: R) -> Self {
        Self {
            surface,
            gate: PaintGate::new(),
            renderer,
        }
    }

    pub fn raise(&self, sig: GridSignals) {
        self.gate.raise(sig);
    }

    pub fn drain_signals(&self) -> GridSignals {
        self.gate.drain()
    }

    pub fn resize(&mut self, css: CanvasSize, dpr: i32) {
        self.surface.resize(css, dpr);
        self.renderer.resize_for_dpr(dpr);
    }
}

/// Full-canvas pixel rect. Layer-wide fill / clear converge here so the
/// `f64` (CSS) → `i32` (PixelRect) rounding lives in one place.
fn full_canvas_rect(size: CanvasSize) -> PixelRect {
    PixelRect {
        top_left: Point { x: 0, y: 0 },
        width: size.w.round() as i32,
        height: size.h.round() as i32,
    }
}

// Grid-layer specialization. Lives here, not on `GridRenderer<P>`, because the
// full-canvas bg fill is the *surface's* concern — the renderer paints cells
// and chrome through the painter, but the layer-wide clear is a once-per-frame
// pixel op the surface owns alongside its `present()`.
impl<S> LayerBase<S, GridRenderer<S::P>>
where
    S: Surface,
    S::P: BlitPainter,
{
    /// Full grid paint (Fresh / SlotsReuse). Fills the canvas bg only when
    /// the frame's slot vecs are fresh; SlotsReuse paths preserve last
    /// frame's pixels so per-pane fingerprint-skip wins are preserved.
    pub fn paint_grid(&mut self, model: &dyn CanvasModel, frame: &Chrome) {
        if !frame.kind.reuses_slots() {
            self.surface.painter().rect_fill(
                full_canvas_rect(frame.canvas_size),
                PaintColor::from_theme_str(&frame.theme.cell_bg),
            );
        }
        self.renderer.render_grid(model, frame);
    }

    /// Scroll-blit grid paint: shift the kept band per `BlitPlan::shifts`,
    /// then run `render_grid_blit` (which only repaints the revealed strip).
    pub fn paint_grid_blit(&mut self, model: &dyn CanvasModel, frame: &Chrome, plan: &BlitPlan) {
        for s in &plan.shifts {
            self.renderer.painter_blit(s.src, s.dst);
        }
        self.renderer.render_grid_blit(model, frame, plan);
    }

    pub fn invalidate_paint_cache(&mut self) {
        self.renderer.invalidate_paint_cache();
    }

    pub fn invalidate_pane_cache(&self, mask: PaneRegionMask) {
        self.renderer.invalidate_pane_cache(mask);
    }
}

// Overlay-layer specialization. The clear is a `Painter::clear_rect` so it
// routes through every backend uniformly; SVG / Recorder no-op / record it.
impl<S> LayerBase<S, OverlayRenderer<S::P>>
where
    S: Surface,
{
    pub fn paint_overlay_layer(
        &mut self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        selection: &SelectionLayer,
        others: &[&dyn Layer],
    ) {
        let size = frame.canvas_size;
        let painter = self.surface.painter();
        painter.clear_rect(full_canvas_rect(size));
        painter.begin_group("overlay");

        // Selection paints fill (under) then stroke + handle (over) the
        // active-cell repaint. Header highlights land between selection
        // and the rest so the highlighted header strip is above the
        // selection tint.
        selection.paint(model, frame, painter);
        if let Some(hook) = selection.after_paint_renderer_hook(model, frame) {
            self.renderer
                .repaint_active_cell(model, hook.row, hook.col, frame);
        }
        selection.paint_after_hook(model, frame, painter);

        self.renderer
            .render_header_highlights(Axis::Row, frame, selection.selection_range);
        self.renderer
            .render_header_highlights(Axis::Column, frame, selection.selection_range);

        for layer in others {
            layer.paint(model, frame, painter);
        }
        painter.end_group();
    }
}
