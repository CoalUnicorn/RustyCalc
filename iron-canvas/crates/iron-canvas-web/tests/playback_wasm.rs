#![cfg(target_arch = "wasm32")]
#![cfg(feature = "dev-tools")]
//! Browser-only regression test: `replay_through` must `present()` the grid
//! surface after EACH replayed frame, not once at the end.
//!
//! `CanvasPainter::blit` reads its kept band from the *visible front*
//! canvas, while replay paints into the detached back canvas —
//! `WebSurface::present` is the only thing that copies back -> front. A
//! recorded `Fresh` -> `Viewport` (blit) sequence therefore needs the
//! `Fresh` frame presented before the `Viewport` frame's `Blit` op replays,
//! or the blit reads stale/cleared front pixels and the final composite is
//! corrupted.

use std::cell::Cell;
use std::rc::Rc;

use iron_canvas_core::PaintRegimeTag;
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
/// idiom (a mutable value visible to the very next `paintIfDirty()` with
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
    // this directly (no default-height fallback, unlike the Fresh build
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

/// Acceptance criterion: seeking to a recorded `Viewport` (blit) frame must
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
    let Ok(()) = canvas.setModel(make_scroll_fixture_model(Rc::clone(&top_row))) else {
        panic!("scroll fixture model passes the duck test");
    };
    canvas.resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR);
    canvas.paint_if_dirty(); // baseline Fresh paint with real data, before recording starts

    let Ok(()) = canvas.start_recording(JsValue::UNDEFINED) else {
        panic!("start recording");
    };

    top_row.set(2); // scroll by one row
    // `view_changed()` declares the navigation intent; `decide` still
    // detects the actual scroll geometrically via `screen_for_blit` and
    // picks the Viewport (blit) regime — this call only wakes dispatch.
    canvas.view_changed_js();
    canvas.paint_if_dirty(); // must land the Viewport (blit) regime

    let Ok(bytes_arr) = canvas.stop_recording() else {
        panic!("stop recording");
    };
    let bytes = bytes_arr.to_vec();

    // Guard the fixture: the scenario is worthless if the scroll above
    // didn't actually record a Viewport frame carrying a Blit op.
    let Ok(rec) = Recording::deserialize(&bytes) else {
        panic!("recording deserializes");
    };
    let regimes: Vec<PaintRegimeTag> = rec.frames.iter().map(|f| f.regime).collect();
    assert!(
        rec.frames
            .iter()
            .any(|f| f.regime == PaintRegimeTag::Viewport
                && f.grid_ops
                    .iter()
                    .any(|op| matches!(op, DrawOp::Blit { .. }))),
        "fixture must record a Viewport frame containing a Blit op — got regimes {regimes:?}"
    );

    let live_bytes = grid_pixels(&grid);
    clear_canvas_white(&grid);

    let Ok(()) = canvas.load_recording(&bytes) else {
        panic!("load recording");
    };
    let last = canvas.recording_frame_count() - 1;
    let Ok(()) = canvas.seek_recording(last) else {
        panic!("seek to final frame");
    };

    assert_eq!(
        grid_pixels(&grid),
        live_bytes,
        "playback must present() after every replayed grid frame — a Blit frame replayed \
         without an intervening present reads stale/cleared front-canvas pixels and corrupts \
         the composite"
    );
}
