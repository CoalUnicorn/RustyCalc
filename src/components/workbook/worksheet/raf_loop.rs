//! Per-frame render loop — fires on demand via `use_one_shot_raf`'s
//! `poke`, coalesced to at most one paint per animation frame.
//!
//! Owns the lazy IronCanvas construction: runs every frame until both
//! `<canvas>` refs are mounted AND container dims > 0, then becomes a
//! no-op via the `slot.is_some()` short-circuit. Handles the
//! zero-size-container edge case (refs resolved but layout pass hasn't
//! measured yet) without extra ResizeObserver plumbing.
//!
//! `ic.paint_if_dirty()` is cheap and safe to call unconditionally (it
//! no-ops internally when nothing is dirty), so there is no separate
//! "should I paint" gate here beyond "did something poke() me" -- that
//! is exactly what `use_one_shot_raf` already answers.

use leptos::html;
use leptos::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use crate::app_state::AppState;
use crate::components::workbook::one_shot_raf::use_one_shot_raf;
use crate::coord::SheetRange;
use crate::input::mouse::CanvasHandle;
use crate::state::{ModelStore, Split};
use iron_canvas_core::*;
use iron_canvas_web::IronCanvas;

use super::ClipboardDraw;
use super::adapter::WorksheetModelAdapter;
use super::overlay_memo::OverlayTuple;

#[allow(clippy::too_many_arguments)]
pub(super) fn install_raf_loop(
    grid_ref: NodeRef<html::Canvas>,
    overlay_ref: NodeRef<html::Canvas>,
    canvas_handle: CanvasHandle,
    model: ModelStore,
    reactive_overlay: Memo<OverlayTuple>,
    clipboard_draw: ClipboardDraw,
    theme_dirty: StoredValue<bool>,
    app: Option<AppState>,
    show_headers: Split<bool>,
    scroll_into_view: StoredValue<bool>,
) -> impl Fn() + Clone {
    let last_pane_w = Cell::new(0.0f64);
    let last_pane_h = Cell::new(0.0f64);
    let painted_frames = StoredValue::new(0u32);

    let paint = move || -> bool {
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
                    ic.set_model(Rc::new(WorksheetModelAdapter {
                        store: model,
                        show_headers,
                    }));
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

        let constructed = canvas_handle.with_value(|slot| slot.is_some());
        if !constructed {
            return true; // keep polling every frame until constructed
        }

        // Playback tick. Runs unconditionally so play cadence is independent
        // of ordinary paint work; keeps the loop alive on its own (`playing`)
        // rather than needing a poke() per frame while playing.
        #[cfg(feature = "dev-tools")]
        let playing = {
            let mut playing = false;
            if let Some(app) = &app
                && app.playback_loaded.get_untracked()
                && app.playback_playing.get_untracked()
            {
                playing = true;
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
            playing
        };
        #[cfg(not(feature = "dev-tools"))]
        let playing = false;

        // ==============================================================
        // Viewport reconciliation — runs before the paint so a correction
        // lands on this frame rather than flashing on the next one.
        // ==============================================================

        // Freezing panes never moves `top_row`, so the model can hold an origin
        // inside the frozen run that no painted pixel agrees with. The renderer
        // clamps it silently; write the clamp back, because ironcalc's page
        // navigation derives its *new selection* from `top_row` and would
        // compute it from the stale value.
        let legal =
            canvas_handle.with_value(|slot| slot.as_ref().and_then(|ic| ic.legal_scroll_origin()));
        if let Some((top, left)) = legal {
            model.update_value(|m| {
                let view = m.get_selected_view();
                if (view.top_row, view.left_column) != (top, left)
                    && let Err(e) = m.set_top_left_visible_cell(top, left)
                {
                    web_sys::console::warn_1(&format!("[rustycalc nav] origin sync: {e}").into());
                }
            });
        }

        // A navigation asked for the active cell to be brought into view. Only
        // the renderer can say whether it already fits — it alone knows the
        // pane extent, the frozen bands and the partial trailing row.
        if scroll_into_view.get_value() {
            scroll_into_view.set_value(false);
            let (row, column) = model.with_value(|m| {
                let view = m.get_selected_view();
                (view.row, view.column)
            });
            let target = canvas_handle
                .with_value(|slot| slot.as_ref().and_then(|ic| ic.scroll_to_show(row, column)));
            if let Some((top, left)) = target {
                model.update_value(|m| {
                    if let Err(e) = m.set_top_left_visible_cell(top, left) {
                        web_sys::console::warn_1(
                            &format!("[rustycalc nav] scroll into view: {e}").into(),
                        );
                    }
                });
            }
        }

        #[cfg(feature = "dev-tools")]
        web_sys::console::time_with_label("render");
        let paint_t0 = crate::perf::now();
        // Sampling the frame trace is opt-in on the panel being visible, so a
        // closed panel costs nothing per frame.
        let trace_wanted = app
            .as_ref()
            .is_some_and(|a| a.show_perf_panel.get_untracked());
        let mut frame_trace = None;
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
                if trace_wanted {
                    frame_trace = Some(ic.frame_trace());
                }
            }
        });
        #[cfg(feature = "dev-tools")]
        web_sys::console::time_end_with_label("render");

        // Sync the *scrollable pane* extent into the model — the budget
        // ironcalc's on_arrow_* / on_page_* compare accumulated row heights
        // against when deciding to scroll the active cell into view. The
        // canvas is the wrong number: the headers and any frozen bands eat
        // into it, and a canvas-sized window lets the cursor run exactly that
        // far past the visible edge before the model scrolls. Read after the
        // paint so a freeze or header change lands on the same frame it takes
        // effect; `None` only before the very first paint.
        let pane =
            canvas_handle.with_value(|slot| slot.as_ref().and_then(|ic| ic.scroll_pane_rect()));
        if let Some(pane) = pane {
            let pane_w = f64::from(pane.width);
            let pane_h = f64::from(pane.height);
            if pane_w != last_pane_w.get() || pane_h != last_pane_h.get() {
                model.update_value(|m| {
                    m.set_window_width(pane_w);
                    m.set_window_height(pane_h);
                });
                last_pane_w.set(pane_w);
                last_pane_h.set(pane_h);
            }
        }

        // Record paint duration for the PerfPanel. Skipped until the first
        // cell commit has happened so the panel stays on its placeholder
        // ("commit a cell to measure") and we don't spam the signal on
        // every scroll / resize / overlay tick.
        if let Some(app) = &app
            && app.perf.commit_start.get_untracked().is_some()
        {
            app.perf.render_ms.set(Some(crate::perf::now() - paint_t0));
        }

        // The trace deliberately skips the commit_start gate above: scrolling
        // never commits a cell, and the post-blit repaint is exactly what this
        // readout exists to catch.
        //
        // Written on every painted frame, not only on change, and prefixed with
        // a frame counter: an unchanging string is otherwise indistinguishable
        // from a stale panel, and "which regime, every single frame" is exactly
        // the question being asked.
        if let Some(app) = &app
            && let Some(trace) = frame_trace
        {
            let n = painted_frames.get_value() + 1;
            painted_frames.set_value(n);
            app.perf.frame_trace.set(Some(format!("#{n} {trace}")));
        }

        playing
    };

    let poke = use_one_shot_raf(paint);

    // Webfont finished loading: clear the engine's text-measure memos and
    // poke the scheduler — engine-side cache clears alone never reach a
    // repaint without waking a currently-idle (self-paused) loop.
    let poke_for_fonts = poke.clone();
    let _ = leptos_use::use_event_listener(
        web_sys::EventTarget::from(document().fonts()),
        leptos::ev::Custom::<web_sys::Event>::new("loadingdone"),
        move |_| {
            canvas_handle.update_value(|slot| {
                if let Some(ic) = slot.as_mut() {
                    ic.fonts_changed();
                }
            });
            poke_for_fonts();
        },
    );

    poke
}
