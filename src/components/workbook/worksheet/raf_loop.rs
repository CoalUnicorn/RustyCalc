//! Per-frame render loop — fires on demand via `use_one_shot_raf`'s
//! `poke`, coalesced to at most one paint per animation frame.
//!
//! Owns the lazy IronCanvas construction: runs every frame until both
//! `<canvas>` refs are mounted AND container dims > 0, then becomes a
//! no-op via the `slot.is_some()` short-circuit. Handles the
//! zero-size-container edge case (refs resolved but layout pass hasn't
//! measured yet) without extra ResizeObserver plumbing.
//!
//! `ic.render_pending()` is cheap and safe to call unconditionally (it
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
#[cfg(feature = "dev-tools")]
use crate::perf::{AppendOutcome, AttemptOrigin, CaptureState};
use crate::state::{ModelStore, Split};
use iron_canvas_core::*;
use iron_canvas_web::{IronCanvas, RenderResult};

use super::ClipboardDraw;
use super::adapter::WorksheetModelAdapter;
#[cfg(feature = "dev-tools")]
use super::capture_collect;
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
                    // A rejected pair (non-finite/negative extent, non-positive DPR)
                    // leaves the canvas at its last valid size.
                    let _ = ic.resize(w, h, dpr);
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
        // View reconciliation — runs before the paint so a correction
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
        // Sampling the frame trace is opt-in on the inspector being open, so a
        // closed inspector costs nothing per frame.
        let trace_wanted = app
            .as_ref()
            .is_some_and(|a| a.inspector_open.get_untracked());
        let mut paint_result = RenderResult::Idle;
        // Duration of the `render_pending()` call alone: `performance.now()`
        // is read immediately before and after it, inside the same closure —
        // the theme check above is not part of the number.
        #[cfg(feature = "dev-tools")]
        let mut render_sample = None;
        canvas_handle.update_value(|slot| {
            if let Some(ic) = slot.as_mut() {
                if theme_dirty.get_value() {
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = window().document().and_then(|d| d.document_element()) {
                        ic.set_theme_from_element(&el);
                    }
                    theme_dirty.set_value(false);
                }
                #[cfg(feature = "dev-tools")]
                let started_at_ms = crate::perf::now();
                paint_result = ic.render_pending();
                #[cfg(feature = "dev-tools")]
                {
                    render_sample = Some(crate::perf::RenderSample {
                        started_at_ms,
                        ms: crate::perf::now() - started_at_ms,
                    });
                }
            }
        });
        #[cfg(feature = "dev-tools")]
        web_sys::console::time_end_with_label("render");

        // Idle touches no diagnostic; Rendered counts + times; RetryRequired
        // publishes the held-pane trace without counting a frame;
        // PlaybackActive (dev-tools short-circuit) leaves every diagnostic
        // untouched. See `scheduling_after` below.
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

        // Record the paint duration for the PerfPanel on every frame that
        // actually painted. A scroll, resize, or overlay tick measures here
        // and publishes no mutation sample — the two numbers answer
        // different questions and neither waits for the other.
        #[cfg(feature = "dev-tools")]
        if action.update_timing
            && let Some(app) = &app
            && let Some(sample) = render_sample
        {
            app.perf.render_call.set(Some(sample));
        }

        // Trace numbering uses the engine's attempt sequence, not a host
        // frame counter: the number names the attempt, so a held attempt is
        // visible as its own attempt rather than reusing the last committed
        // frame's number. The accessor allocates nothing and works with
        // detailed capture off.
        if let Some(app) = &app
            && let Some(trace) = frame_trace
        {
            let attempt = canvas_handle
                .with_value(|slot| slot.as_ref().and_then(|ic| ic.frame_attempt_seq()));
            app.perf
                .frame_trace
                .set(Some(format!("#{} {trace}", attempt.unwrap_or(0))));
        }

        // Attempt capture: one immutable record per non-idle attempt the
        // engine reported, appended only while a capture is running. The
        // record carries the attempt's own identity, so a hold and its retry
        // are two records rather than one overwritten sample.
        #[cfg(feature = "dev-tools")]
        if let Some(app) = &app
            && action.publish_trace
            && matches!(app.perf_store.state_untracked(), CaptureState::Capturing(_))
        {
            let store = app.perf_store;
            let snapshot = canvas_handle
                .with_value(|slot| slot.as_ref().and_then(|ic| ic.frame_diagnostics_snapshot()));
            if let Some(snapshot) = snapshot
                && let AppendOutcome::Rejected(_) = capture_collect::append_snapshot(
                    &store,
                    model,
                    snapshot,
                    AttemptOrigin::Live,
                    render_sample.map(|sample| sample.ms),
                )
            {
                store.request_canvas_sync();
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

/// What one rAF tick does with `render_pending`'s outcome, decided once so
/// the four call sites in `install_raf_loop`'s `paint` closure don't each
/// re-derive "which variants publish / count / keep alive".
struct SchedulerAction {
    publish_trace: bool,
    #[cfg(any(test, feature = "dev-tools"))]
    update_timing: bool,
    keep_alive: bool,
}

/// Pure outcome policy. `playback_active` is the same `playing` bool the
/// dev-tools playback tick already computed this frame, and it is also the
/// only thing that arms the one-shot loop.
///
/// A `RetryRequired` holds the last committed pixels and keeps its pending
/// work in the engine, so it needs no frame of its own: the next `poke()` —
/// a model change, a resize, a workbook switch, a recording start, a font
/// load — retries that work. Native worksheet reads return a value or
/// `Absent`, never a transient bridge failure, so a held native attempt
/// waits for a host correction rather than for time to pass; re-arming the
/// loop on the hold itself would spin it for as long as the bad value lives.
/// The standalone JS host keeps its own retry policy, where a thrown or
/// malformed bridge reply really can clear on its own.
fn scheduling_after(result: RenderResult, playback_active: bool) -> SchedulerAction {
    let keep_alive = playback_active;
    match result {
        RenderResult::Idle => SchedulerAction {
            publish_trace: false,
            #[cfg(any(test, feature = "dev-tools"))]
            update_timing: false,
            keep_alive,
        },
        RenderResult::Rendered => SchedulerAction {
            publish_trace: true,
            #[cfg(any(test, feature = "dev-tools"))]
            update_timing: true,
            keep_alive,
        },
        RenderResult::RetryRequired => SchedulerAction {
            publish_trace: true,
            #[cfg(any(test, feature = "dev-tools"))]
            update_timing: false,
            keep_alive,
        },
        RenderResult::PlaybackActive => SchedulerAction {
            publish_trace: false,
            #[cfg(any(test, feature = "dev-tools"))]
            update_timing: false,
            keep_alive,
        },
    }
}

#[cfg(test)]
mod scheduling_after_tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn idle_touches_no_diagnostic_and_preserves_keep_alive() {
        let action = scheduling_after(RenderResult::Idle, false);
        assert!(!action.publish_trace);
        assert!(!action.update_timing);
        assert!(!action.keep_alive);

        let action = scheduling_after(RenderResult::Idle, true);
        assert!(
            action.keep_alive,
            "idle must not clear an already-active playback tick"
        );
    }

    #[wasm_bindgen_test]
    fn painted_publishes_counts_and_times() {
        let action = scheduling_after(RenderResult::Rendered, false);
        assert!(action.publish_trace);
        assert!(action.update_timing);
        assert!(!action.keep_alive);
    }

    #[wasm_bindgen_test]
    fn retry_publishes_the_held_trace_without_counting_a_frame() {
        let action = scheduling_after(RenderResult::RetryRequired, false);
        assert!(
            action.publish_trace,
            "a held attempt must still publish its trace"
        );
        assert!(
            !action.update_timing,
            "a held attempt is not a painted frame"
        );
    }

    #[wasm_bindgen_test]
    fn playback_leaves_every_diagnostic_untouched() {
        let action = scheduling_after(RenderResult::PlaybackActive, true);
        assert!(!action.publish_trace);
        assert!(!action.update_timing);
    }

    /// Playback is the only wake-up this policy grants. Every outcome of an
    /// idle app leaves the loop to the next `poke()`, so a held attempt stops
    /// the loop instead of spinning on input that only the host can correct.
    #[wasm_bindgen_test]
    fn no_outcome_of_an_idle_app_keeps_the_loop_armed() {
        for result in [
            RenderResult::Idle,
            RenderResult::Rendered,
            RenderResult::RetryRequired,
            RenderResult::PlaybackActive,
        ] {
            assert!(
                !scheduling_after(result, false).keep_alive,
                "{result:?} must leave the loop to the next poke()"
            );
            assert!(
                scheduling_after(result, true).keep_alive,
                "{result:?} must not pause an active playback tick"
            );
        }
    }
}

/// A hold driven through the real loop, the real `WorksheetModelAdapter` and a
/// real `<canvas>`: the app stops scheduling, the committed pixels and query
/// geometry survive, and a corrected model plus one `poke()` renders the work
/// the engine retained.
///
/// The bad geometry is the state the XLSX importer can install: it parses the
/// `ht` attribute with `f64::from_str` (`"NaN"` is accepted) and
/// `Worksheet::set_row_height` rejects only negative heights.
#[cfg(test)]
mod held_geometry_tests {
    use super::*;
    use crate::events::EventBus;
    use crate::state::WorkbookState;
    use std::cell::{Cell, RefCell};
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_test::wasm_bindgen_test;

    const CANVAS_W: f64 = 400.0;
    const CANVAS_H: f64 = 240.0;
    /// Visible in the fixture viewport, so the fresh geometry walk reads it.
    const MALFORMED_ROW: i32 = 3;

    struct Fixture {
        state: WorkbookState,
        model: ModelStore,
        handle: CanvasHandle,
    }

    fn fixture() -> Fixture {
        Fixture {
            state: WorkbookState::new(EventBus::new()),
            model: StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("Sheet1", "en", "UTC", "en")
                    .expect("empty workbook"),
            ),
            handle: StoredValue::new_local(None),
        }
    }

    fn element() -> web_sys::HtmlCanvasElement {
        document()
            .create_element("canvas")
            .expect("create canvas element")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("element is a canvas")
    }

    fn live_canvas(
        fixture: &Fixture,
    ) -> (
        IronCanvas,
        web_sys::HtmlCanvasElement,
        web_sys::HtmlCanvasElement,
    ) {
        let (grid, overlay) = (element(), element());
        let mut canvas =
            IronCanvas::create(grid.clone(), overlay.clone()).expect("create IronCanvas");
        canvas
            .resize(CANVAS_W, CANVAS_H, 1.0)
            .expect("fixture metrics are valid");
        canvas.set_model(Rc::new(WorksheetModelAdapter {
            store: fixture.model,
            show_headers: fixture.state.show_headers,
        }));
        (canvas, grid, overlay)
    }

    fn pixels(canvas: &web_sys::HtmlCanvasElement) -> Vec<u8> {
        let ctx = canvas
            .get_context("2d")
            .expect("get canvas context")
            .expect("2d context exists")
            .dyn_into::<web_sys::CanvasRenderingContext2d>()
            .expect("context is Canvas2D");
        ctx.get_image_data(0.0, 0.0, canvas.width() as f64, canvas.height() as f64)
            .expect("read painted pixels")
            .data()
            .0
    }

    async fn next_frame() {
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            let _ = window().request_animation_frame(&resolve);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }

    fn set_row_height(model: ModelStore, row: i32, height: f64) {
        model.update_value(|m| {
            m.set_rows_height(0, row, row, height)
                .expect("a non-negative height is accepted");
        });
    }

    fn request_repaint(handle: CanvasHandle) {
        handle.update_value(|slot| {
            slot.as_mut().expect("canvas is live").request_repaint();
        });
    }

    fn cell_rect(handle: CanvasHandle, row: i32, column: i32) -> Option<PixelRect> {
        handle.with_value(|slot| {
            slot.as_ref()
                .expect("canvas is live")
                .cell_rect(row, column)
        })
    }

    #[wasm_bindgen_test]
    async fn a_hold_stops_loop_scheduling_and_a_correction_renders_retained_work() {
        let owner = Owner::new();
        let fixture = owner.with(fixture);
        let (canvas, grid, overlay) = live_canvas(&fixture);
        fixture.handle.set_value(Some(canvas));

        // The paint closure mirrors the production tail: render one frame
        // through the app's handle, then let the shared outcome policy say
        // whether the one-shot loop runs again. `false` is this app's real
        // `playing` value — the fixture loads no recording.
        let results = Rc::new(RefCell::new(Vec::new()));
        let frames = Rc::new(Cell::new(0_u32));
        let poke = owner.with(|| {
            let results = Rc::clone(&results);
            let frames = Rc::clone(&frames);
            use_one_shot_raf(move || {
                frames.set(frames.get() + 1);
                let mut result = RenderResult::Idle;
                fixture.handle.update_value(|slot| {
                    if let Some(ic) = slot.as_mut() {
                        result = ic.render_pending();
                    }
                });
                results.borrow_mut().push(result);
                scheduling_after(result, false).keep_alive
            })
        });

        next_frame().await;
        assert_eq!(results.borrow().as_slice(), [RenderResult::Rendered]);
        let committed_grid = pixels(&grid);
        let committed_overlay = pixels(&overlay);
        let committed_rect = cell_rect(fixture.handle, 1, 1);
        assert!(committed_rect.is_some(), "the first frame commits geometry");

        // A malformed import lands a non-finite row height on the live model.
        set_row_height(fixture.model, MALFORMED_ROW, f64::NAN);
        request_repaint(fixture.handle);
        poke();
        next_frame().await;

        assert_eq!(results.borrow().last(), Some(&RenderResult::RetryRequired));
        assert_eq!(
            pixels(&grid),
            committed_grid,
            "a held attempt must present no new grid pixels"
        );
        assert_eq!(
            pixels(&overlay),
            committed_overlay,
            "a held attempt must present no new overlay pixels"
        );
        assert_eq!(
            cell_rect(fixture.handle, 1, 1),
            committed_rect,
            "queries must read the last committed frame while held"
        );

        // The hold has no wake-up of its own: no further frame runs.
        let held_frames = frames.get();
        next_frame().await;
        next_frame().await;
        assert_eq!(
            frames.get(),
            held_frames,
            "a held attempt must not schedule another frame"
        );

        // The correction a host can make: a real height for that row, then one
        // poke for the work the engine retained.
        set_row_height(fixture.model, MALFORMED_ROW, 30.0);
        request_repaint(fixture.handle);
        poke();
        next_frame().await;

        assert_eq!(results.borrow().last(), Some(&RenderResult::Rendered));
        assert_ne!(
            pixels(&grid),
            committed_grid,
            "the corrected row height must reach the raster"
        );

        // Byte-identical to a canvas that paints the corrected model fresh.
        let (mut control, control_grid, _control_overlay) = live_canvas(&fixture);
        assert_eq!(control.render_pending(), RenderResult::Rendered);
        assert_eq!(pixels(&grid), pixels(&control_grid));
    }
}
