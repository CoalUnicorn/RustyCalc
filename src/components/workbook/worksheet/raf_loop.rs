//! Per-frame render loop — fires on every animation frame (~60 fps).
//!
//! Renders only when `render_needed` is true; otherwise returns
//! immediately (single untracked signal read + branch).
//!
//! Owns the lazy IronCanvas construction: runs every frame until both
//! `<canvas>` refs are mounted AND container dims > 0, then becomes a
//! no-op via the `slot.is_some()` short-circuit. Handles the
//! zero-size-container edge case (refs resolved but layout pass hasn't
//! measured yet) without extra ResizeObserver plumbing.

use leptos::html;
use leptos::prelude::*;
use leptos_use::use_raf_fn;
use std::cell::Cell;
use std::rc::Rc;
use web_sys::HtmlCanvasElement;

use crate::app_state::AppState;
use crate::coord::SheetRange;
use crate::input::mouse::CanvasHandle;
use crate::state::ModelStore;
use iron_canvas_core::*;
use iron_canvas_web::IronCanvas;

use super::ClipboardDraw;
use super::adapter::WorksheetModelAdapter;
use super::overlay_memo::OverlayTuple;

pub(super) fn install_raf_loop(
    grid_ref: NodeRef<html::Canvas>,
    overlay_ref: NodeRef<html::Canvas>,
    canvas_handle: CanvasHandle,
    model: ModelStore,
    reactive_overlay: Memo<OverlayTuple>,
    clipboard_draw: ClipboardDraw,
    theme_dirty: StoredValue<bool>,
    render_needed: RwSignal<bool>,
    app: Option<AppState>,
) {
    let last_canvas_w = Cell::new(0.0f64);
    let last_canvas_h = Cell::new(0.0f64);
    let _ = use_raf_fn(move |_| {
        canvas_handle.update_value(|slot| {
            if slot.is_some() {
                return;
            }
            let Some(grid_el) = grid_ref.get_untracked() else {
                return;
            };
            let Some(overlay_el) = overlay_ref.get_untracked() else {
                return;
            };
            let w = grid_el.client_width() as f64;
            let h = grid_el.client_height() as f64;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let dpr = window().device_pixel_ratio();
            match IronCanvas::create(grid_el, overlay_el) {
                Ok(mut ic) => {
                    ic.resize(w, h, dpr);
                    // Initial state push: sync current Worksheet state to the
                    // freshly-constructed orchestrator so the first frame is
                    // correct. Subsequent pushes are driven by the reactive
                    // subscribe Effect and the workbook-switch Effect.
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = window().document().and_then(|d| d.document_element()) {
                        ic.set_theme_from_element(&el);
                    }
                    ic.set_model(Rc::new(WorksheetModelAdapter { store: model }));
                    let OverlayTuple {
                        extend_to,
                        point_range,
                        formula_refs,
                    } = reactive_overlay.get_untracked();
                    let clipboard = clipboard_draw.with_value(|opt| {
                        opt.as_ref().map(|acb| SheetRange {
                            sheet: acb.sheet,
                            area: acb.range,
                        })
                    });
                    ic.set_overlays(RenderOverlays {
                        extend_to,
                        clipboard: clipboard.map(Into::into),
                        point_range: point_range.map(Into::into),
                        formula_refs: formula_refs.into_iter().map(Into::into).collect(),
                    });
                    *slot = Some(ic);
                }
                Err(e) => web_sys::console::error_1(&e),
            }
        });

        // Playback tick. Runs unconditionally so play cadence is independent
        // of `render_needed` (which is gated on workbook state changes).
        // No-op when no recording is loaded or play is paused.
        #[cfg(feature = "dev-tools")]
        if let Some(app) = &app {
            if app.playback_loaded.get_untracked() && app.playback_playing.get_untracked() {
                canvas_handle.update_value(|slot| {
                    if let Some(ic) = slot.as_mut() {
                        if ic.tick_playback(crate::perf::now()) {
                            app.playback_frame.set(ic.recording_current_frame());
                        }
                        // Engine auto-pauses at end-of-recording — mirror it.
                        if !ic.is_playing() {
                            app.playback_playing.set(false);
                        }
                    }
                });
            }
        }

        if !render_needed.get_untracked() {
            return;
        }
        render_needed.set(false);

        let Some(canvas) = grid_ref.get_untracked() else {
            return;
        };
        let canvas_el: HtmlCanvasElement = canvas;
        // Sync canvas dimensions into the model so scroll/autofill knows the
        // visible viewport size.
        let canvas_w = canvas_el.client_width() as f64;
        let canvas_h = canvas_el.client_height() as f64;
        if canvas_w != last_canvas_w.get() || canvas_h != last_canvas_h.get() {
            model.update_value(|m| {
                m.set_window_width(canvas_w);
                m.set_window_height(canvas_h);
            });
            last_canvas_w.set(canvas_w);
            last_canvas_h.set(canvas_h);
        }
        #[cfg(feature = "dev-tools")]
        web_sys::console::time_with_label("render");
        let paint_t0 = crate::perf::now();
        canvas_handle.update_value(|slot| {
            if let Some(ic) = slot.as_mut() {
                if theme_dirty.get_value() {
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = window().document().and_then(|d| d.document_element()) {
                        ic.set_theme_from_element(&el);
                    }
                    theme_dirty.set_value(false);
                }
                ic.paint_if_dirty();
            }
        });
        #[cfg(feature = "dev-tools")]
        web_sys::console::time_end_with_label("render");

        // Record paint duration for the PerfPanel. Skipped until the first
        // cell commit has happened so the panel stays on its placeholder
        // ("commit a cell to measure") and we don't spam the signal on
        // every scroll / resize / overlay tick.
        if let Some(app) = &app {
            if app.perf.commit_start.get_untracked().is_some() {
                app.perf.render_ms.set(Some(crate::perf::now() - paint_t0));
            }
        }
    });
}
