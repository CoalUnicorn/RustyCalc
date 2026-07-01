#![cfg(target_arch = "wasm32")]
//! Browser-only proof that fractional DPR reaches the real `<canvas>`
//! backing store through the `IronCanvas` facade. Regression test for a
//! bug where `resize()` rounded `dpr` before forwarding it to
//! `WebSurface::resize`, silently mapping e.g. 1.25 -> 1 and 1.5 -> 2.

use iron_canvas_web::{CanvasSize, IronCanvas};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

fn make_canvas() -> HtmlCanvasElement {
    web_sys::window()
        .expect("browser window")
        .document()
        .expect("document")
        .create_element("canvas")
        .expect("create canvas element")
        .dyn_into::<HtmlCanvasElement>()
        .expect("element is a canvas")
}

#[wasm_bindgen_test]
fn fractional_dpr_reaches_canvas_backing_store() {
    let grid = make_canvas();
    let overlay = make_canvas();
    let mut canvas = IronCanvas::create(grid.clone(), overlay.clone()).expect("create IronCanvas");

    canvas.resize(300.0, 200.0, 1.25);

    let (expect_w, expect_h) = CanvasSize { w: 300.0, h: 200.0 }.to_backing_size(1.25);
    assert_eq!(
        grid.width(),
        expect_w,
        "grid backing width must use unrounded DPR"
    );
    assert_eq!(
        grid.height(),
        expect_h,
        "grid backing height must use unrounded DPR"
    );
    assert_eq!(
        overlay.width(),
        expect_w,
        "overlay backing width must use unrounded DPR"
    );
}
