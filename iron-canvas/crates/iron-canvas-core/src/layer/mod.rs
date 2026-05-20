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

use crate::geometry::CanvasSize;
use crate::painter::{BlitPainter, Painter};
use crate::renderer::LayerOps;
use crate::signal::GridSignals;

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

    /// Hand the renderer its own owning handle to the same painter. Production
    /// backends wrap in `Rc` so the painter is shared single-threaded.
    fn clone_painter(&self) -> Rc<Self::P>;

    /// Resize the backing store. Painter-side state updates (the renderer's
    /// `dpr` field, the paint-state cache) are routed via
    /// `LayerBase::resize` → `LayerOps::resize_for_dpr` so each backend
    /// resizes only what it owns.
    fn resize(&mut self, css: CanvasSize, dpr: i32);

    /// Flush the rendered frame. Canvas-2D auto-presents (no-op);
    /// Cairo / off-screen image backends flush here.
    fn present(&self);
}

pub struct PaintGate {
    signals: Cell<GridSignals>,
    /// Tick incremented per non-empty `drain`. Used by cross-crate tests in
    /// `iron-canvas` to assert paint cadence; the `Cell<u32>` cost is
    /// negligible and the field stays available without a `cfg(test)` gate
    /// (which doesn't reach across crate boundaries).
    pub paint_count: Cell<u32>,
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

    pub fn mark_dirty(&self) {
        self.raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
    }

    pub fn should_paint(&self) -> bool {
        !self.drain().is_empty()
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
    pub surface: S,
    gate: PaintGate,
    pub renderer: R,
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

    /// Back-compat shim: callers that don't yet know which signal they
    /// raise get the safest blanket. Per-setter routing lives in the
    /// orchestrator.
    pub fn mark_dirty(&self) {
        self.gate
            .raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
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
