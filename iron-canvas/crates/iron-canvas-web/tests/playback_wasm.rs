#![cfg(target_arch = "wasm32")]
#![cfg(feature = "dev-tools")]
// The replay fixtures bind a JS model handle through `setModel`.
#![cfg(feature = "js-model")]
//! Browser-only regression test: `replay_through` must `present()` the grid
//! surface after EACH replayed frame, not once at the end.
//!
//! `CanvasPainter::blit` reads its kept band from the *visible front*
//! canvas, while replay paints into the detached back canvas —
//! `WebSurface::present` is the only thing that copies back -> front. A
//! recorded `FullRebuild` -> `ScrollBlit` sequence therefore needs the
//! `FullRebuild` frame presented before the `ScrollBlit` frame's `Blit` op replays,
//! or the blit reads stale/cleared front pixels and the final composite is
//! corrupted.

use std::cell::Cell;
use std::rc::Rc;

use iron_canvas_core::RenderStrategy;
use iron_canvas_recorder::DrawOp;
use iron_canvas_recorder::recording::Recording;
use iron_canvas_web::IronCanvas;
use ironcalc_base::types as ic;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

const FIXTURE_CANVAS_W: f64 = 400.0;
const FIXTURE_CANVAS_H: f64 = 400.0;
const FIXTURE_DPR: f64 = 1.0;

fn make_canvas() -> HtmlCanvasElement {
    let Some(window) = web_sys::window() else {
        panic!("browser window");
    };
    let Some(document) = window.document() else {
        panic!("document");
    };
    let Ok(element) = document.create_element("canvas") else {
        panic!("create canvas element");
    };
    let Ok(canvas) = element.dyn_into::<HtmlCanvasElement>() else {
        panic!("element is a canvas");
    };
    canvas
}

fn set_prop(obj: &js_sys::Object, name: &str, f: &js_sys::Function) {
    let Ok(_) = js_sys::Reflect::set(obj, &JsValue::from_str(name), f) else {
        panic!("set fixture model method");
    };
}

/// Duck-typed model whose `getSelectedView` reads `top_row` live off a
/// shared `Cell` — the view analogue of `render_wasm.rs`'s shared-store
/// idiom (a mutable value visible to the very next `renderPending()` with
/// no second `setModel` round-trip), so a test can "scroll" between two
/// paints. Cell content is a pure `r{row}c{col}` function of position —
/// no shared store needed since no scenario here edits a value.
fn make_scroll_fixture_model(top_row: Rc<Cell<i32>>) -> JsValue {
    let obj = js_sys::Object::new();

    let get_view = Closure::wrap(Box::new(move || -> JsValue {
        let view = js_sys::Object::new();
        let Ok(_) = js_sys::Reflect::set(&view, &JsValue::from_str("sheet"), &JsValue::from(0u32))
        else {
            panic!("set fixture view.sheet");
        };
        let Ok(_) = js_sys::Reflect::set(&view, &JsValue::from_str("row"), &JsValue::from(1i32))
        else {
            panic!("set fixture view.row");
        };
        let Ok(_) = js_sys::Reflect::set(&view, &JsValue::from_str("column"), &JsValue::from(1i32))
        else {
            panic!("set fixture view.column");
        };
        let range = js_sys::Array::new();
        range.push(&JsValue::from(1i32));
        range.push(&JsValue::from(1i32));
        range.push(&JsValue::from(1i32));
        range.push(&JsValue::from(1i32));
        let Ok(_) = js_sys::Reflect::set(&view, &JsValue::from_str("range"), &range) else {
            panic!("set fixture view.range");
        };
        let Ok(_) = js_sys::Reflect::set(
            &view,
            &JsValue::from_str("top_row"),
            &JsValue::from(top_row.get()),
        ) else {
            panic!("set fixture view.top_row");
        };
        let Ok(_) = js_sys::Reflect::set(
            &view,
            &JsValue::from_str("left_column"),
            &JsValue::from(1i32),
        ) else {
            panic!("set fixture view.left_column");
        };
        view.into()
    }) as Box<dyn Fn() -> JsValue>);
    set_prop(&obj, "getSelectedView", get_view.as_ref().unchecked_ref());
    get_view.forget();

    // Most engine call sites read the sheet via this standalone accessor
    // rather than `getSelectedView`'s embedded `sheet` field; this fixture
    // pins sheet 0, so a missing accessor here silently throws and falls
    // back to 0 without ever failing a test.
    set_prop(
        &obj,
        "getSelectedSheet",
        &js_sys::Function::new_no_args("return 0;"),
    );

    // Required for the blit *probe* specifically: `overlaps_match` compares
    // this directly (no default-height fallback, unlike the full-rebuild
    // path) to verify the kept band's rows still match after a scroll.
    set_prop(
        &obj,
        "getRowHeight",
        &js_sys::Function::new_no_args("return 20;"),
    );
    set_prop(
        &obj,
        "getFrozenRowsCount",
        &js_sys::Function::new_no_args("return 0;"),
    );
    set_prop(
        &obj,
        "getFrozenColumnsCount",
        &js_sys::Function::new_no_args("return 0;"),
    );
    set_prop(
        &obj,
        "getShowGridLines",
        &js_sys::Function::new_no_args("return true;"),
    );

    let get_style = Closure::wrap(Box::new(|_sheet: u32, _row: i32, _col: i32| -> JsValue {
        let Ok(value) = serde_wasm_bindgen::to_value(&ic::Style::default()) else {
            panic!("fixture Style always serializes");
        };
        value
    }) as Box<dyn Fn(u32, i32, i32) -> JsValue>);
    set_prop(&obj, "getCellStyle", get_style.as_ref().unchecked_ref());
    get_style.forget();

    let get_type = Closure::wrap(Box::new(|_sheet: u32, _row: i32, _col: i32| -> i32 {
        ic::CellType::Text as i32
    }) as Box<dyn Fn(u32, i32, i32) -> i32>);
    set_prop(&obj, "getCellType", get_type.as_ref().unchecked_ref());
    get_type.forget();

    let get_value = Closure::wrap(Box::new(|_sheet: u32, row: i32, col: i32| -> String {
        format!("r{row}c{col}")
    }) as Box<dyn Fn(u32, i32, i32) -> String>);
    set_prop(
        &obj,
        "getFormattedCellValue",
        get_value.as_ref().unchecked_ref(),
    );
    get_value.forget();

    obj.into()
}

/// Raw RGBA backing-store bytes for the front (visible) canvas — same
/// `get_image_data` idiom as `render_wasm.rs`'s `grid_pixels`.
fn grid_pixels(canvas: &HtmlCanvasElement) -> Vec<u8> {
    let Ok(context_opt) = canvas.get_context("2d") else {
        panic!("getContext must not throw");
    };
    let Some(context_obj) = context_opt else {
        panic!("2d context must exist");
    };
    let Ok(ctx) = context_obj.dyn_into::<web_sys::CanvasRenderingContext2d>() else {
        panic!("context is CanvasRenderingContext2d");
    };
    let Ok(image_data) =
        ctx.get_image_data(0.0, 0.0, canvas.width() as f64, canvas.height() as f64)
    else {
        panic!("get_image_data must succeed on an opaque, same-origin canvas");
    };
    image_data.data().0
}

/// Paint the canvas solid white directly through its own 2d context,
/// bypassing `IronCanvas` entirely. Makes a non-presenting replay
/// detectable: only `replay_through`'s own present-after-each-frame call
/// can restore the correct pixels afterward.
fn clear_canvas_white(canvas: &HtmlCanvasElement) {
    let Ok(context_opt) = canvas.get_context("2d") else {
        panic!("getContext must not throw");
    };
    let Some(context_obj) = context_opt else {
        panic!("2d context must exist");
    };
    let Ok(ctx) = context_obj.dyn_into::<web_sys::CanvasRenderingContext2d>() else {
        panic!("context is CanvasRenderingContext2d");
    };
    ctx.set_fill_style_str("white");
    ctx.fill_rect(0.0, 0.0, canvas.width() as f64, canvas.height() as f64);
}

fn recorded_canvas() -> (IronCanvas, HtmlCanvasElement, HtmlCanvasElement, Recording) {
    let grid = make_canvas();
    let overlay = make_canvas();
    let mut canvas = IronCanvas::create(grid.clone(), overlay.clone()).expect("create canvas");
    canvas
        .set_model_js(make_scroll_fixture_model(Rc::new(Cell::new(1))))
        .expect("fixture model");
    canvas.resize(400.0, 400.0, 1.25).expect("live metrics");
    canvas.render_pending();
    canvas
        .start_recording(JsValue::UNDEFINED)
        .expect("start capture");
    let bytes = canvas.stop_recording().expect("stop capture").to_vec();
    let rec = Recording::deserialize(&bytes).expect("recording decodes");
    (canvas, grid, overlay, rec)
}

#[wasm_bindgen_test]
fn exit_playback_preserves_an_active_capture() {
    let (mut canvas, _, _, _) = recorded_canvas();
    canvas
        .start_recording(JsValue::UNDEFINED)
        .expect("start capture");
    canvas.exit_playback();
    let bytes = canvas
        .stop_recording()
        .expect("capture remains active")
        .to_vec();
    assert!(
        !Recording::deserialize(&bytes)
            .expect("capture decodes")
            .frames
            .is_empty()
    );
}

#[wasm_bindgen_test]
fn facade_autofit_and_export_report_read_failures() {
    let mut canvas = IronCanvas::create(make_canvas(), make_canvas()).expect("create canvas");
    assert!(canvas.fit_column_width_js(1, 1, 2).is_err());
    assert!(canvas.export_svg(200.0, 100.0).is_err());
    #[cfg(feature = "pdf")]
    assert!(canvas.export_pdf(200.0, 100.0).is_err());
    for method in ["getSelectedSheet", "getFormattedCellValue", "getCellStyle"] {
        let model = make_scroll_fixture_model(Rc::new(Cell::new(1)));
        let obj: &js_sys::Object = model.unchecked_ref();
        set_prop(
            obj,
            method,
            &js_sys::Function::new_no_args("throw new Error('read failure');"),
        );
        canvas.set_model_js(model).expect("fixture model");
        assert!(canvas.fit_column_width_js(1, 1, 2).is_err(), "{method}");
        assert!(canvas.export_svg(200.0, 100.0).is_err(), "{method}");
        #[cfg(feature = "pdf")]
        assert!(canvas.export_pdf(200.0, 100.0).is_err(), "{method}");
    }
    let model = make_scroll_fixture_model(Rc::new(Cell::new(1)));
    set_prop(
        model.unchecked_ref(),
        "getFormattedCellValue",
        &js_sys::Function::new_no_args("return '';"),
    );
    canvas.set_model_js(model).expect("empty model");
    assert_eq!(
        canvas
            .fit_column_width_js(1, 1, 2)
            .expect("valid empty range"),
        None
    );
    assert!(canvas.export_svg(200.0, 100.0).is_ok());
}

#[wasm_bindgen_test]
fn seek_reports_diagnostics_prefix_without_mutating_pixels() {
    use iron_canvas_recorder::recording::{RecordedPaintResult, TraceOutcome};
    use iron_canvas_web::ReplayResult;
    let (mut canvas, grid, overlay, mut rec) = recorded_canvas();
    let mut held = rec.frames[0].clone();
    held.result = RecordedPaintResult::Retry;
    held.trace.committed_seq = None;
    held.trace.effective = None;
    held.trace.outcome = TraceOutcome::HeldOnBridgeFailure;
    held.grid_ops.clear();
    held.overlay_ops.clear();
    rec.frames.insert(0, held);
    canvas
        .load_recording(&rec.serialize().expect("serialize prefix"))
        .expect("load prefix");
    assert_eq!(
        canvas.seek_recording(1).expect("paint anchor"),
        ReplayResult::Replayed
    );
    let before_grid = grid_pixels(&grid);
    let before_overlay = grid_pixels(&overlay);
    assert_eq!(
        canvas.seek_recording(0).expect("seek held prefix"),
        ReplayResult::NoCommittedFrame
    );
    assert_eq!(canvas.recording_current_frame(), 0);
    assert_eq!(grid_pixels(&grid), before_grid);
    assert_eq!(grid_pixels(&overlay), before_overlay);
}

#[wasm_bindgen_test]
fn invalid_resize_and_recording_preserve_live_state() {
    let (mut canvas, grid, overlay, mut rec) = recorded_canvas();
    let before_grid = grid_pixels(&grid);
    let before_overlay = grid_pixels(&overlay);
    for (w, h, dpr) in [
        (f64::NAN, 400.0, 1.0),
        (400.0, -1.0, 1.0),
        (400.0, 400.0, 0.0),
        (400.0, 400.0, f64::MAX),
    ] {
        assert!(canvas.resize(w, h, dpr).is_err());
        assert_eq!((grid.width(), grid.height()), (500, 500));
        assert_eq!(grid_pixels(&grid), before_grid);
        assert_eq!(grid_pixels(&overlay), before_overlay);
    }
    rec.frames[0].grid_ops.push(DrawOp::PopClip);
    assert!(
        canvas
            .load_recording(&rec.serialize().expect("serialize invalid recording"))
            .is_err()
    );
    assert!(!canvas.playback_active());
    assert_eq!((grid.width(), grid.height()), (500, 500));
    assert_eq!(grid_pixels(&grid), before_grid);
    assert_eq!(grid_pixels(&overlay), before_overlay);
    assert_eq!(
        grid.style().get_property_value("width").expect("CSS width"),
        ""
    );
}

#[wasm_bindgen_test]
fn replacement_recording_preserves_original_live_metrics() {
    let (mut canvas, grid, overlay, mut rec) = recorded_canvas();
    for width in [200.0, 300.0] {
        rec.header.canvas_w = width;
        rec.header.canvas_h = 100.0;
        rec.header.dpr = 1.0;
        canvas
            .load_recording(&rec.serialize().expect("serialize recording"))
            .expect("load recording");
        assert_eq!((grid.width(), grid.height()), (width as u32, 100));
    }
    let before = grid_pixels(&grid);
    rec.frames[0].overlay_ops.push(DrawOp::PopClip);
    assert!(
        canvas
            .load_recording(&rec.serialize().expect("serialize invalid recording"))
            .is_err()
    );
    assert!(canvas.playback_active());
    assert_eq!(grid_pixels(&grid), before);
    canvas.exit_playback();
    assert!(!canvas.playback_active());
    assert_eq!((grid.width(), grid.height()), (500, 500));
    assert_eq!((overlay.width(), overlay.height()), (500, 500));
    assert_eq!(
        grid.style().get_property_value("width").expect("CSS width"),
        ""
    );
}

#[wasm_bindgen_test]
fn rejected_recording_css_preserves_live_and_playback_state() {
    for playback in [false, true] {
        for failure in 0..4 {
            let (mut canvas, grid, overlay, mut rec) = recorded_canvas();
            if playback {
                rec.header.canvas_w = 200.0;
                rec.header.canvas_h = 100.0;
                canvas
                    .load_recording(&rec.serialize().expect("serialize first recording"))
                    .expect("load first recording");
            } else {
                grid.style()
                    .set_css_text("width: 80% !important; height: 90%; color: red;");
                overlay
                    .style()
                    .set_css_text("width: 75%; height: 85% !important;");
            }
            let before_css = [grid.style().css_text(), overlay.style().css_text()];
            let before_pixels = [grid_pixels(&grid), grid_pixels(&overlay)];
            let before_size = canvas.canvas_size();
            let before_frame = canvas.recording_current_frame();
            let before_count = canvas.recording_frame_count();
            let target = if failure < 2 { &grid } else { &overlay };
            let property = if failure % 2 == 0 { "width" } else { "height" };
            let reject = js_sys::Function::new_with_args(
                "name, value, priority",
                &format!(
                    "if (name === '{property}') throw new Error('injected CSS failure');
                     return CSSStyleDeclaration.prototype.setProperty.call(this, name, value, priority);"
                ),
            );
            js_sys::Reflect::set(&target.style(), &JsValue::from_str("setProperty"), &reject)
                .expect("inject CSS failure on this canvas only");
            rec.header.canvas_w = 300.0;
            rec.header.canvas_h = 150.0;
            let result = canvas.load_recording(&rec.serialize().expect("serialize replacement"));
            js_sys::Reflect::delete_property(&target.style(), &JsValue::from_str("setProperty"))
                .expect("remove CSS failure injection");
            assert!(result.is_err());
            assert_eq!(canvas.playback_active(), playback);
            assert_eq!(canvas.canvas_size(), before_size);
            assert_eq!(canvas.recording_current_frame(), before_frame);
            assert_eq!(canvas.recording_frame_count(), before_count);
            assert_eq!([grid_pixels(&grid), grid_pixels(&overlay)], before_pixels);
            assert_eq!(
                [grid.style().css_text(), overlay.style().css_text()],
                before_css
            );
        }
    }
}

/// Acceptance criterion: seeking to a recorded `ScrollBlit` frame must
/// raster identically to the live frame it was captured from. Before the
/// fix, `replay_through` never presented the grid surface mid-replay, so
/// the front canvas kept whatever `clear_canvas_white` left it at.
#[wasm_bindgen_test]
fn playback_presents_scroll_blit_frame_byte_identical_to_live() {
    let top_row = Rc::new(Cell::new(1i32));
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay) else {
        panic!("create IronCanvas");
    };
    let Ok(()) = canvas.set_model_js(make_scroll_fixture_model(Rc::clone(&top_row))) else {
        panic!("scroll fixture model passes the duck test");
    };
    canvas
        .resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR)
        .expect("fixture canvas metrics are valid");
    canvas.render_pending(); // baseline FullRebuild paint before recording starts

    let Ok(()) = canvas.start_recording(JsValue::UNDEFINED) else {
        panic!("start recording");
    };

    top_row.set(2); // scroll by one row
    // `view_changed()` declares the navigation intent; `Chrome::classify`
    // still detects the actual scroll geometrically, then `plan_frame` picks
    // the ScrollBlit strategy — this call only wakes dispatch.
    canvas.view_changed_js();
    canvas.render_pending(); // must land the ScrollBlit strategy

    let Ok(bytes_arr) = canvas.stop_recording() else {
        panic!("stop recording");
    };
    let bytes = bytes_arr.to_vec();

    // Guard the fixture: the scenario is worthless if the scroll above
    // did not record a ScrollBlit frame carrying a Blit op.
    let Ok(rec) = Recording::deserialize(&bytes) else {
        panic!("recording deserializes");
    };
    let strategies: Vec<RenderStrategy> = rec
        .frames
        .iter()
        .filter_map(|frame| frame.trace.strategy)
        .collect();
    assert!(
        rec.frames
            .iter()
            .any(|f| f.trace.strategy == Some(RenderStrategy::ScrollBlit)
                && f.grid_ops
                    .iter()
                    .any(|op| matches!(op, DrawOp::Blit { .. }))),
        "fixture must record a ScrollBlit frame containing a Blit op; got {strategies:?}"
    );

    let live_bytes = grid_pixels(&grid);
    clear_canvas_white(&grid);

    let Ok(()) = canvas.load_recording(&bytes) else {
        panic!("load recording");
    };
    let last = canvas.recording_frame_count() - 1;
    let Ok(outcome) = canvas.seek_recording(last) else {
        panic!("seek to final frame");
    };
    assert_eq!(
        outcome,
        iron_canvas_web::ReplayResult::Replayed,
        "the fixture records a committed FullRebuild anchor, so the seek must replay it"
    );

    assert_eq!(
        grid_pixels(&grid),
        live_bytes,
        "playback must present() after every replayed grid frame — a Blit frame replayed \
         without an intervening present reads stale/cleared front-canvas pixels and corrupts \
         the composite"
    );
}
