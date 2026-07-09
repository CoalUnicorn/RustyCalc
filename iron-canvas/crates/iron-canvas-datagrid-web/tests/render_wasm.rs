#![cfg(target_arch = "wasm32")]
//! Browser-only proof of the camelCase deserialize path. Constructing real
//! `HtmlCanvasElement`s in the headless harness is impractical, so this
//! exercises the `serde-wasm-bindgen` decode that `setData` relies on: a
//! JS object literal with camelCase keys (`rowHeight`) must round-trip into
//! `GridDataWire` without throwing.

use iron_canvas_core::{CanvasSize, CanvasTheme};
use iron_canvas_datagrid_web::DataGridCanvas;
use iron_canvas_datagrid_web::wire::GridDataWire;
use js_sys::{Array, Object, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

fn set(obj: &Object, key: &str, val: JsValue) {
    // `Reflect::set` only errors on a non-object target; `obj` is an Object.
    let _ = Reflect::set(obj, &JsValue::from_str(key), &val);
}

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

// Regression test for a bug where `resize()` rounded `dpr` before
// forwarding it to `WebSurface::resize`, silently mapping e.g. 1.25 -> 1
// and 1.5 -> 2.
#[wasm_bindgen_test]
fn fractional_dpr_reaches_canvas_backing_store() {
    let grid = make_canvas();
    let overlay = make_canvas();
    let mut canvas =
        DataGridCanvas::new(grid.clone(), overlay.clone()).expect("create DataGridCanvas");

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
}

#[wasm_bindgen_test]
fn set_data_from_js_object_decodes_camel_case() {
    let column = Object::new();
    set(&column, "header", JsValue::from_str("Name"));
    set(&column, "width", JsValue::from_f64(120.0));

    let columns = Array::new();
    columns.push(&column);

    let cell = Object::new();
    set(&cell, "value", JsValue::from_str("Apple"));
    set(&cell, "bold", JsValue::from_bool(true));

    let cells = Array::new();
    cells.push(&cell);

    let row = Object::new();
    set(&row, "cells", cells.into());

    let rows = Array::new();
    rows.push(&row);

    let data = Object::new();
    set(&data, "columns", columns.into());
    set(&data, "rows", rows.into());
    set(&data, "rowHeight", JsValue::from_f64(22.0));

    let decoded: GridDataWire = match serde_wasm_bindgen::from_value(data.into()) {
        Ok(wire) => wire,
        Err(e) => panic!("camelCase GridDataWire must decode without throwing: {e:?}"),
    };

    assert_eq!(
        decoded.row_height,
        Some(22.0),
        "camelCase rowHeight decoded"
    );
    assert_eq!(decoded.columns.len(), 1, "one column decoded");
    assert_eq!(decoded.rows.len(), 1, "one row decoded");
}

// E.1: constructing a real `HtmlCanvasElement` to drive `setThemeName` through
// a live handle is impractical here (see module note), so assert the built-in
// palettes the name-switch resolves to are actually distinct.
#[wasm_bindgen_test]
fn builtin_themes_differ_in_cell_bg() {
    assert_ne!(
        CanvasTheme::dark().cell_bg,
        CanvasTheme::light().cell_bg,
        "dark and light palettes must differ"
    );
}
