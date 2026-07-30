//! Layer machinery — backend-agnostic.
//!
//! `Surface` wraps a drawing target (HTML canvas, Cairo surface,
//! in-memory recorder). One Surface owns one backing store + the painter
//! that draws into it; the renderer borrows the painter via `painter()`.
//!
//! `LayerBase<S, R>` pairs a Surface with a layer-specific renderer and owns
//! the surface, resize, present, cache invalidation, and paint execution —
//! and nothing else. It holds no dirty state: all paint work is queued on
//! `Orchestrator`'s single `PendingWork` value, which decides regimes
//! globally rather than per layer. Layer-specific paint methods live on the
//! renderer wrappers (`GridRenderer<P>` / `OverlayRenderer<P>`), reached
//! through `LayerBase::renderer`.

use std::rc::Rc;

use crate::CanvasModel;
use crate::chrome::{BlitPlan, Chrome, PaneRegionMask};
use crate::decoration::{DecorationId, Layer, selection::SelectionLayer};
use crate::geometry::CanvasSize;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Axis, Point};
use crate::painter::{BlitPainter, GroupClass, PaintColor, Painter};
use crate::pending_work::RowSpan;
use crate::renderer::{GridRenderer, LayerOps, OverlayRenderer};

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

    /// Resize the backing store. `dpr` here scales the backing pixel
    /// buffer (e.g. `canvas.width = css.w * dpr`) — it does *not* set the
    /// painter's transform matrix. That side runs separately via
    /// `LayerBase::resize` -> `LayerOps::resize_for_dpr`. Two effects, one
    /// shared input; each backend resizes only what it owns.
    fn resize(&mut self, css: CanvasSize, dpr: f64);

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

    pub fn resize(&mut self, css: CanvasSize, dpr: f64) {
        self.surface.resize(css, dpr);
        self.renderer.resize_for_dpr(dpr);
    }

    /// Flush this layer's surface. Callers present a layer iff the current
    /// paint arm actually painted it — see the regime arms in
    /// `orchestrator.rs` for the per-arm "painted -> present" wiring.
    pub fn present(&self) {
        self.surface.present();
    }
}

/// Full-canvas pixel rect. Layer-wide fill / clear converge here so the
/// `f64` (CSS) -> `i32` (PixelRect) rounding lives in one place.
fn full_canvas_rect(size: CanvasSize) -> PixelRect {
    PixelRect {
        top_left: Point { x: 0, y: 0 },
        width: size.w.round() as i32,
        height: size.h.round() as i32,
    }
}

/// Outcome of the blit grid paint. `Held` = the preflight aborted before
/// any pixel shifted; the caller must not present or commit the frame.
#[must_use]
pub enum BlitPaint {
    Painted,
    Held,
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
    ///
    /// Returns the held-pane mask from `render_grid` — see its doc.
    pub fn paint_grid(&mut self, model: &dyn CanvasModel, frame: &Chrome) -> PaneRegionMask {
        if !frame.kind.reuses_slots() {
            self.surface.painter().rect_fill(
                full_canvas_rect(frame.canvas_size),
                PaintColor::from_theme_str(&frame.theme.cell_bg),
            );
        }
        self.renderer.render_grid(model, frame)
    }

    /// Scroll-blit grid paint: shift the kept band per `BlitPlan::shifts`,
    /// then run `render_grid_blit` (which only repaints the revealed strip).
    ///
    /// The preflight (`prefetch_blit_strips`) fetches and bridge-validates
    /// every revealed strip BEFORE a single pixel is shifted. If any fetch
    /// fails, the whole frame is abandoned as a no-op: no shift, no paint. This
    /// is deliberate and minimal — shifting pixels and then discovering the
    /// fetch failed is the bug being fixed (it strands stale, misplaced pixels
    /// in the revealed strip). A fallback full repaint is intentionally NOT
    /// attempted here; a future frame reconciles once the bridge recovers via
    /// the normal frame-kind dispatch, so a reader should not "upgrade" the
    /// returned `BlitPaint::Held` without re-deriving why it is sufficient.
    pub fn paint_grid_blit(
        &mut self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> BlitPaint {
        if !self.renderer.prefetch_blit_strips(model, frame, plan) {
            return BlitPaint::Held;
        }
        for s in &plan.shifts {
            self.renderer.painter_blit(s.src, s.dst);
        }
        self.renderer.render_grid_blit(model, frame, plan);
        BlitPaint::Painted
    }

    /// Damage grid paint: prior pixels stay; only the damaged full-width
    /// row bands refetch and repaint. No full-canvas bg fill by design.
    ///
    /// Returns the held-pane mask from `render_grid_damage` — see its doc.
    pub fn paint_grid_damage(
        &mut self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> PaneRegionMask {
        self.renderer.render_grid_damage(model, frame, spans)
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
        customs: &[(DecorationId, Rc<dyn Layer>)],
    ) {
        let size = frame.canvas_size;
        let painter = self.surface.painter();
        painter.clear_rect(full_canvas_rect(size));
        painter.begin_group(GroupClass::Overlay);

        // Selection paints fill (under) then stroke + handle (over) the
        // active-cell repaint. Header highlights land between selection
        // and the rest so the highlighted header strip is above the
        // selection tint.
        painter.begin_group(GroupClass::SelectionFill);
        selection.paint(frame, painter);
        painter.end_group();

        // Gate on `Some` so a tick where the model briefly has no selected
        // view (sheet swap, workbook reload) does not repaint A1 with the
        // default-zero snapshot or emit an empty bracket into recordings.
        if let Some(hook) = selection.active_cell_repaint() {
            painter.begin_group(GroupClass::ActiveCellRepaint);
            self.renderer
                .repaint_active_cell(model, hook.row, hook.col, frame);
            painter.end_group();
        }

        painter.begin_group(GroupClass::SelectionStroke);
        selection.paint_stroke(frame, painter);
        painter.end_group();

        if let Some(sel) = selection.selection_range {
            painter.begin_group(GroupClass::HeaderHighlights);
            if frame.row_header_thickness > 0 {
                self.renderer
                    .render_header_highlights(Axis::Row, frame, sel);
            }
            if frame.col_header_thickness > 0 {
                self.renderer
                    .render_header_highlights(Axis::Column, frame, sel);
            }
            painter.end_group();
        }

        // Other decorations: one group each, named by the layer itself.
        for layer in others {
            painter.begin_group(layer.group());
            layer.paint(frame, painter);
            painter.end_group();
        }
        // Consumer band — topmost, insertion order back-to-front. Same
        // bracket-per-layer contract as the built-ins above.
        for (_, layer) in customs {
            painter.begin_group(layer.group());
            layer.paint(frame, painter);
            painter.end_group();
        }
        painter.end_group();
    }
}
