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
use iron_canvas_web::{IronCanvas, JsPaintResult};
#[cfg(feature = "dev-tools")]
use wasm_bindgen::JsValue;

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
            let mut wrote = false;
            model.update_value(|m| {
                let view = m.get_selected_view();
                if (view.top_row, view.left_column) != (top, left) {
                    match m.set_top_left_visible_cell(top, left) {
                        Ok(()) => wrote = true,
                        Err(e) => {
                            web_sys::console::warn_1(
                                &format!("[rustycalc nav] origin sync: {e}").into(),
                            );
                        }
                    }
                }
            });
            // Notify view_changed only for the actual write, independent of
            // whatever upstream event (or none at all) triggered this tick —
            // the freeze clamp can fire on any frame the renderer detects an
            // illegal origin, not only in response to a navigation event.
            if wrote {
                canvas_handle.update_value(|slot| {
                    if let Some(ic) = slot.as_mut() {
                        ic.view_changed();
                    }
                });
            }
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
                // `scroll_to_show` already returns `None` when the target
                // matches the current origin (see its core doc), so `Some`
                // here always names a real, different origin — the write
                // below either lands it or logs why it couldn't.
                let mut wrote = false;
                model.update_value(|m| match m.set_top_left_visible_cell(top, left) {
                    Ok(()) => wrote = true,
                    Err(e) => {
                        web_sys::console::warn_1(
                            &format!("[rustycalc nav] scroll into view: {e}").into(),
                        );
                    }
                });
                if wrote {
                    canvas_handle.update_value(|slot| {
                        if let Some(ic) = slot.as_mut() {
                            ic.view_changed();
                        }
                    });
                }
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
        let mut paint_result = JsPaintResult::Idle;
        canvas_handle.update_value(|slot| {
            if let Some(ic) = slot.as_mut() {
                if theme_dirty.get_value() {
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = window().document().and_then(|d| d.document_element()) {
                        ic.set_theme_from_element(&el);
                    }
                    theme_dirty.set_value(false);
                }
                paint_result = ic.paint_if_dirty();
            }
        });
        #[cfg(feature = "dev-tools")]
        web_sys::console::time_end_with_label("render");

        // Idle touches no diagnostic; Painted counts + times; Retry publishes
        // the held-pane trace without counting a frame and forces the loop to
        // stay armed; Playback (dev-tools short-circuit) leaves every
        // diagnostic untouched. See `scheduling_after` below.
        let action = scheduling_after(paint_result, playing);
        let mut frame_trace = None;
        if trace_wanted && action.publish_trace {
            frame_trace = canvas_handle.with_value(|slot| slot.as_ref().map(|ic| ic.frame_trace()));
        }

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
        // every scroll / resize / overlay tick — and skipped on a tick that
        // didn't commit or retry a paint (Idle / Playback).
        if action.update_timing
            && let Some(app) = &app
            && app.perf.commit_start.get_untracked().is_some()
        {
            app.perf.render_ms.set(Some(crate::perf::now() - paint_t0));
        }

        // The trace deliberately skips the commit_start gate above: scrolling
        // never commits a cell, and the post-blit repaint is exactly what this
        // readout exists to catch.
        //
        // Written on every painted or retried frame, not only on change, and
        // prefixed with a frame counter: an unchanging string is otherwise
        // indistinguishable from a stale panel, and "which regime, every
        // single frame" is exactly the question being asked. A held Retry
        // publishes at the same counter value rather than a new one — it
        // names the attempt, not a committed frame.
        if let Some(app) = &app
            && let Some(trace) = frame_trace
        {
            if action.count_frame {
                painted_frames.set_value(painted_frames.get_value() + 1);
            }
            let n = painted_frames.get_value();
            app.perf.frame_trace.set(Some(format!("#{n} {trace}")));
        }

        // Structured frame diagnostics: sample `frameDiagnostics()` only when
        // the panel toggle enabled capture and this frame actually published
        // a trace. The toggle path lives entirely in `install_diag_effect`
        // (which owns the wake); this block only reads signals, so a paused
        // loop stops sampling as soon as the toggle flips back off.
        #[cfg(feature = "dev-tools")]
        if let Some(app) = &app
            && app.perf.diag_enabled.get_untracked()
            && action.publish_trace
        {
            let json = canvas_handle.with_value(|slot| {
                slot.as_ref().and_then(|ic| {
                    let value = ic.frame_diagnostics();
                    if value.is_undefined() {
                        None
                    } else {
                        // Two-space indent: the popup shows the snapshot in
                        // a bounded scrollable surface, so multi-line JSON
                        // is far more inspectable than one compact line.
                        js_sys::JSON::stringify_with_replacer_and_space(
                            &value,
                            &JsValue::NULL,
                            &JsValue::from_str("  "),
                        )
                        .ok()
                        .and_then(|text| text.as_string())
                    }
                })
            });
            if let Some(json) = json {
                app.perf.frame_diagnostics.set(Some(json));
            }
        }

        action.keep_alive
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

/// What one rAF tick does with `paint_if_dirty`'s outcome, decided once so
/// the four call sites in `install_raf_loop`'s `paint` closure don't each
/// re-derive "which variants publish / count / keep alive".
struct SchedulerAction {
    publish_trace: bool,
    count_frame: bool,
    update_timing: bool,
    keep_alive: bool,
}

/// Pure outcome policy. `playback_active` is the same
/// `playing` bool the dev-tools playback tick already computed this frame —
/// `Idle` and `Playback` simply hand it back unchanged; `Retry` forces it to
/// `true` so the one-shot loop stays armed until the held attempt commits.
/// No external bridge-recovery signal exists to wake a paused loop, so a
/// `Retry` must remain live even when the failure lasts for many frames.
fn scheduling_after(result: JsPaintResult, playback_active: bool) -> SchedulerAction {
    match result {
        JsPaintResult::Idle => SchedulerAction {
            publish_trace: false,
            count_frame: false,
            update_timing: false,
            keep_alive: playback_active,
        },
        JsPaintResult::Painted => SchedulerAction {
            publish_trace: true,
            count_frame: true,
            update_timing: true,
            keep_alive: playback_active,
        },
        JsPaintResult::Retry => SchedulerAction {
            publish_trace: true,
            count_frame: false,
            update_timing: false,
            keep_alive: true,
        },
        JsPaintResult::Playback => SchedulerAction {
            publish_trace: false,
            count_frame: false,
            update_timing: false,
            keep_alive: playback_active,
        },
    }
}

#[cfg(test)]
mod scheduling_after_tests {
    use super::*;

    #[test]
    fn idle_touches_no_diagnostic_and_preserves_keep_alive() {
        let action = scheduling_after(JsPaintResult::Idle, false);
        assert!(!action.publish_trace);
        assert!(!action.count_frame);
        assert!(!action.update_timing);
        assert!(!action.keep_alive);

        let action = scheduling_after(JsPaintResult::Idle, true);
        assert!(
            action.keep_alive,
            "idle must not clear an already-active playback tick"
        );
    }

    #[test]
    fn painted_publishes_counts_and_times() {
        let action = scheduling_after(JsPaintResult::Painted, false);
        assert!(action.publish_trace);
        assert!(action.count_frame);
        assert!(action.update_timing);
        assert!(!action.keep_alive);
    }

    #[test]
    fn retry_publishes_without_counting_and_forces_keep_alive() {
        let action = scheduling_after(JsPaintResult::Retry, false);
        assert!(action.publish_trace);
        assert!(!action.count_frame);
        assert!(!action.update_timing);
        assert!(action.keep_alive, "a held attempt must keep the loop armed");
    }

    #[test]
    fn playback_leaves_every_diagnostic_untouched() {
        let action = scheduling_after(JsPaintResult::Playback, true);
        assert!(!action.publish_trace);
        assert!(!action.count_frame);
        assert!(!action.update_timing);
        assert!(
            action.keep_alive,
            "playback keep-alive is driven by the tick, not this policy"
        );
    }

    #[test]
    fn retry_remains_live_until_a_later_attempt_commits() {
        for attempt in 1..=1_000 {
            let action = scheduling_after(JsPaintResult::Retry, false);
            assert!(action.keep_alive, "retry attempt {attempt} paused the loop");
        }

        let committed = scheduling_after(JsPaintResult::Painted, false);
        assert!(
            !committed.keep_alive,
            "a committed paint may let an otherwise-idle loop pause"
        );
    }
}
