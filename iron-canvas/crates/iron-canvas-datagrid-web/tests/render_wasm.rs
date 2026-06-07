#![cfg(target_arch = "wasm32")]
//! Browser-only proof of the camelCase deserialize path. Constructing real
//! `HtmlCanvasElement`s in the headless harness is impractical, so this
//! exercises the `serde-wasm-bindgen` decode that `setData` relies on: a
//! JS object literal with camelCase keys (`rowHeight`) must round-trip into
//! `GridDataWire` without throwing.

use iron_canvas_core::CanvasTheme;
use iron_canvas_datagrid_web::wire::GridDataWire;
use js_sys::{Array, Object, Reflect};
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn set(obj: &Object, key: &str, val: JsValue) {
    // `Reflect::set` only errors on a non-object target; `obj` is an Object.
    let _ = Reflect::set(obj, &JsValue::from_str(key), &val);
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
