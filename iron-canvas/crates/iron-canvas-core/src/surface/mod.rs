//! Layer machinery — backend-agnostic.
//!
//! `Surface` wraps a drawing target (HTML canvas, Cairo surface,
//! in-memory recorder). One Surface owns one backing store + the painter
//! that draws into it; the renderer borrows the painter via `painter()`.
//!
//! `LayerBase<S, R>` pairs a Surface with a layer-specific renderer and owns
//! the surface, resize, present, cache invalidation, and paint execution —
//! and nothing else. It holds no dirty state: all paint work is queued on
//! `Orchestrator`'s single `PendingWork` value, which decides strategies
//! globally rather than per layer. Layer-specific paint methods live on the
//! renderer wrappers (`GridRenderer<P>` / `OverlayRenderer<P>`), reached
//! through `LayerBase::renderer`.

use std::rc::Rc;

use crate::geometry::CanvasMetrics;
use crate::geometry::CanvasSize;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::painter::{BlitPainter, Painter};
use crate::renderer::LayerOps;

mod grid;
mod overlay;

/// Drawing target abstraction. Production wasm holds one Surface per
/// `<canvas>` (grid + overlay); a Cairo backend would hold one per
/// `DrawingArea`; the in-memory test surface holds a `RecorderPainter`.
///
/// Surfaces own their painter outright; renderers receive a cloned handle
/// via `clone_painter` at construction so paint methods don't need to
/// re-borrow through the surface on every call.
#[diagnostic::on_unimplemented(
    note = "a `Surface` owns one `Painter` and exposes `painter`, `clone_painter`, `resize`, `present`. Reference impls: `WebSurface` (iron-canvas-canvas2d), `SvgSurface` and `PdfSurface` (iron-canvas-export), `MemSurface` (iron-canvas-recorder)"
)]
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

    /// Resize the backing store. `metrics.dpr()` scales the backing pixel
    /// buffer (e.g. `canvas.width = css.w * dpr`) — it does *not* set the
    /// painter's transform matrix. That side runs separately via
    /// `LayerBase::resize` -> `LayerOps::resize_for_dpr`. Two effects, one
    /// shared input; each backend resizes only what it owns.
    ///
    /// Takes [`CanvasMetrics`] rather than a raw size/DPR pair so a backend
    /// never has to re-check finiteness, sign, or `u32` fit: the pair was
    /// parsed at the host boundary, and `metrics.backing_size()` is
    /// infallible from there on.
    fn resize(&mut self, metrics: CanvasMetrics);

    /// Flush the rendered frame. Backends without a back buffer no-op
    /// this; `WebSurface`'s grid surface flips its back buffer onto the
    /// visible canvas here — Canvas-2D presentation is not a no-op.
    fn present(&self);
}

pub struct LayerBase<S, R>
where
    S: Surface,
    R: LayerOps<Painter = S::P>,
{
    pub(crate) surface: S,
    pub(crate) renderer: R,
}

impl<S, R> LayerBase<S, R>
where
    S: Surface,
    R: LayerOps<Painter = S::P>,
{
    pub fn new(surface: S, renderer: R) -> Self {
        Self { surface, renderer }
    }

    pub fn resize(&mut self, metrics: CanvasMetrics) {
        self.surface.resize(metrics);
        self.renderer.resize_for_dpr(metrics.dpr());
    }

    /// Flush this layer's surface. Callers present a layer iff the current
    /// paint arm actually painted it — see the strategy arms in
    /// `orchestrator.rs` for the per-arm "painted -> present" wiring.
    pub fn present(&self) {
        self.surface.present();
    }
}

/// Full-canvas pixel rect. Layer-wide fill / clear converge here so the
/// `f64` (CSS) -> `i32` (PixelRect) rounding lives in one place.
fn full_canvas_rect(size: CanvasSize) -> PixelRect {
    let (width, height) = size.to_logical_extent();
    PixelRect {
        top_left: Point { x: 0, y: 0 },
        width,
        height,
    }
}
