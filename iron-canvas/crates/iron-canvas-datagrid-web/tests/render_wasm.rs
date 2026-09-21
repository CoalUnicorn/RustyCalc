#![cfg(target_arch = "wasm32")]
//! Browser checks for canvas sizing, JS input, and serialized hit tests.

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

    canvas
        .resize(300.0, 200.0, 1.25)
        .expect("fixture canvas metrics are valid");

    let (expect_w, expect_h) =
        iron_canvas_core::CanvasMetrics::new(CanvasSize { w: 300.0, h: 200.0 }, 1.25)
            .expect("fixture canvas metrics are valid")
            .backing_size();
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
    assert_eq!(
        overlay.height(),
        expect_h,
        "overlay backing height must use unrounded DPR"
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

#[wasm_bindgen_test]
fn autofill_handle_hit_reaches_js_with_zero_based_coordinates() {
    let mut canvas = DataGridCanvas::new(make_canvas(), make_canvas()).unwrap();
    canvas.resize(240.0, 120.0, 1.25).unwrap();
    canvas
        .set_data(
            js_sys::JSON::parse(
                r#"{"columns":[{"header":"A","width":80},{"header":"B","width":80},
                              {"header":"C","width":32}],
                    "rows":[{"cells":[{"value":"a"},{"value":"b"}]},
                            {"cells":[{"value":"c"},{"value":"d"}]},
                            {"cells":[{"value":"e"}]}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
    // Leave a row and column beyond the selection so the handle is available.
    canvas.set_selection(0, 0, 1, 1);
    canvas.render_pending();

    // Find the painted handle without duplicating the core's header geometry.
    for y in (0..120).step_by(2) {
        for x in (0..240).step_by(2) {
            let hit = canvas.hit_test(f64::from(x), f64::from(y)).unwrap();
            if Reflect::get(&hit, &"kind".into())
                .unwrap()
                .as_string()
                .as_deref()
                == Some("autofillHandle")
            {
                assert_eq!(
                    Reflect::get(&hit, &"row".into()).unwrap().as_f64(),
                    Some(1.0)
                );
                assert_eq!(
                    Reflect::get(&hit, &"col".into()).unwrap().as_f64(),
                    Some(1.0)
                );
                return;
            }
        }
    }
    panic!("the selected range must expose an autofillHandle hit to JS");
}

#[wasm_bindgen_test]
fn invalid_alignment_rejects_set_data_without_replacing_the_model() {
    let mut canvas = DataGridCanvas::new(make_canvas(), make_canvas()).unwrap();
    canvas
        .set_data(
            js_sys::JSON::parse(
                r#"{"columns":[{"header":"A","align":"right"}],
                    "rows":[{"cells":[{"value":"a","align":"center"}]}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
    canvas.sort_by_column(0, true);

    for invalid in [
        r#"{"columns":[{"header":"A","align":"LEFT"}],"rows":[]}"#,
        r#"{"columns":[{"header":"A"}],"rows":[{"cells":[{"value":"b","align":"LEFT"}]}]}"#,
    ] {
        assert!(
            canvas
                .set_data(js_sys::JSON::parse(invalid).unwrap())
                .is_err()
        );
        let sort = canvas.current_sort().unwrap();
        assert_eq!(
            Reflect::get(&sort, &"column".into()).unwrap().as_f64(),
            Some(0.0)
        );
        assert_eq!(
            Reflect::get(&sort, &"ascending".into()).unwrap().as_bool(),
            Some(true)
        );
    }
}

// Check that the built-in palettes have distinct cell backgrounds.
#[wasm_bindgen_test]
fn builtin_themes_differ_in_cell_bg() {
    assert_ne!(
        CanvasTheme::dark().cell_bg,
        CanvasTheme::light().cell_bg,
        "dark and light palettes must differ"
    );
}
