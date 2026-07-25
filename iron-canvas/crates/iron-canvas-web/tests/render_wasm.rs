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

use iron_canvas_web::CanvasModel;
use iron_canvas_web::wasm::JsBackedModel;
use wasm_bindgen::JsValue;

/// Minimal duck-typed model handle: `try_from_js_value` only requires
/// `getSelectedView`; extra methods are supplied per test.
fn model_with_methods(methods: &[(&str, &js_sys::Function)]) -> JsBackedModel {
    let obj = js_sys::Object::new();
    let view = js_sys::Function::new_no_args(
        "return { sheet: 0, row: 1, column: 1, range: [1, 1, 1, 1], top_row: 1, left_column: 1 };",
    );
    js_sys::Reflect::set(&obj, &JsValue::from_str("getSelectedView"), &view)
        .expect("set getSelectedView on plain object");
    for (name, f) in methods {
        js_sys::Reflect::set(&obj, &JsValue::from_str(name), f)
            .expect("set model method on plain object");
    }
    JsBackedModel::try_from_js_value(obj.into()).expect("object passes the setModel duck-test")
}

#[wasm_bindgen_test]
fn header_visibility_defaults_to_visible_when_methods_absent() {
    let model = model_with_methods(&[]);
    assert_eq!(model.get_show_row_headers(0), Some(true));
    assert_eq!(model.get_show_col_headers(0), Some(true));
}

#[wasm_bindgen_test]
fn header_visibility_reads_supplied_false() {
    let ret_false = js_sys::Function::new_no_args("return false;");
    let model = model_with_methods(&[
        ("getShowRowHeaders", &ret_false),
        ("getShowColHeaders", &ret_false),
    ]);
    assert_eq!(model.get_show_row_headers(0), Some(false));
    assert_eq!(model.get_show_col_headers(0), Some(false));
}

// ==============================================================================
// Task 6: raster equivalence between a partial (row-band / border-fallback)
// repaint and a forced-fresh full repaint of the same final model state.
//
// A small JS "model" object is backed by a shared, mutable Rust
// `(row, col) -> FixtureCell` store: `getCellStyle` / `getFormattedCellValue`
// / `getCellType` all read the store live on every call, so mutating the
// store between two `paintIfDirty()` calls on the SAME `IronCanvas` is
// visible to the very next paint without any JS round-trip or a second
// `setModel` — exactly what's needed to drive one canvas through
// baseline-paint -> content-edit -> partial-repaint, while a second,
// independent `IronCanvas`/canvas pair paints the SAME final state fresh
// in one shot. Comparing both grid canvases' raw `ImageData` bytes is the
// raster-equivalence proof; which internal path (`RepaintPlan::Rows` vs
// `Full`) fired is proven separately at the native level
// (`crates/iron-canvas-core/tests/paint_skip.rs`,
// `row_fingerprint_repaint.rs`), which can assert on recorded `DrawOp`s —
// a capability this browser-only facade doesn't expose.
// ==============================================================================

use ironcalc_base::types as ic;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;

const FIXTURE_ROWS: i32 = 20;
const FIXTURE_COLS: i32 = 5;
const FIXTURE_CANVAS_W: f64 = 400.0;
const FIXTURE_CANVAS_H: f64 = 400.0;
const FIXTURE_DPR: f64 = 1.0;

/// One fixture cell's editable state: formatted text plus independent
/// top/bottom border flags, both of which feed
/// `RowFingerprint::has_any_explicit_border` — the single flag
/// `plan_pane_repaint`'s border-safety check reads.
#[derive(Clone, Default)]
struct FixtureCell {
    value: String,
    border_top: bool,
    border_bottom: bool,
}

type FixtureStore = Rc<RefCell<HashMap<(i32, i32), FixtureCell>>>;

/// Plain "rNcM" text, no borders, over `1..=FIXTURE_ROWS` x `1..=FIXTURE_COLS`
/// — the shared starting point every scenario below edits from.
fn plain_fixture_store() -> FixtureStore {
    let mut cells = HashMap::new();
    for row in 1..=FIXTURE_ROWS {
        for col in 1..=FIXTURE_COLS {
            cells.insert(
                (row, col),
                FixtureCell {
                    value: format!("r{row}c{col}"),
                    border_top: false,
                    border_bottom: false,
                },
            );
        }
    }
    Rc::new(RefCell::new(cells))
}

fn set_prop(obj: &js_sys::Object, name: &str, f: &js_sys::Function) {
    js_sys::Reflect::set(obj, &JsValue::from_str(name), f).expect("set fixture model method");
}

/// Build a duck-typed IronCalc model handle over `store`. Selection is
/// pinned at A1 (row 1, col 1) in every scenario below — no fixture ever
/// edits that cell — so the active-cell repaint hook never has to
/// reconcile diverging content there. Frozen rows/cols are 0 (a single
/// `BottomRight` pane covers the whole fixture grid); grid lines are shown.
fn make_fixture_model(store: FixtureStore) -> JsValue {
    let obj = js_sys::Object::new();

    let view = js_sys::Function::new_no_args(
        "return { sheet: 0, row: 1, column: 1, range: [1, 1, 1, 1], top_row: 1, left_column: 1 };",
    );
    set_prop(&obj, "getSelectedView", &view);
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

    let style_store = Rc::clone(&store);
    let get_style = Closure::wrap(Box::new(move |_sheet: u32, row: i32, col: i32| -> JsValue {
        let cell = style_store
            .borrow()
            .get(&(row, col))
            .cloned()
            .unwrap_or_default();
        let style = ic::Style {
            border: ic::Border {
                top: cell.border_top.then_some(ic::BorderItem {
                    style: ic::BorderStyle::Thin,
                    color: ic::Color::None,
                }),
                bottom: cell.border_bottom.then_some(ic::BorderItem {
                    style: ic::BorderStyle::Thin,
                    color: ic::Color::None,
                }),
                ..ic::Border::default()
            },
            ..ic::Style::default()
        };
        serde_wasm_bindgen::to_value(&style).expect("fixture Style always serializes")
    }) as Box<dyn Fn(u32, i32, i32) -> JsValue>);
    set_prop(&obj, "getCellStyle", get_style.as_ref().unchecked_ref());
    get_style.forget();

    let get_type = Closure::wrap(Box::new(|_sheet: u32, _row: i32, _col: i32| -> i32 {
        ic::CellType::Text as i32
    }) as Box<dyn Fn(u32, i32, i32) -> i32>);
    set_prop(&obj, "getCellType", get_type.as_ref().unchecked_ref());
    get_type.forget();

    let value_store = Rc::clone(&store);
    let get_value = Closure::wrap(Box::new(move |_sheet: u32, row: i32, col: i32| -> String {
        value_store
            .borrow()
            .get(&(row, col))
            .map(|c| c.value.clone())
            .unwrap_or_default()
    }) as Box<dyn Fn(u32, i32, i32) -> String>);
    set_prop(
        &obj,
        "getFormattedCellValue",
        get_value.as_ref().unchecked_ref(),
    );
    get_value.forget();

    obj.into()
}

/// Build + resize a fresh `IronCanvas` over `store`. All three scenarios
/// below share this so canvas size, DPR, and model wiring never
/// accidentally diverge between the "partial" and "forced-fresh" side of
/// a comparison.
fn canvas_over(store: FixtureStore) -> (IronCanvas, HtmlCanvasElement) {
    let grid = make_canvas();
    let overlay = make_canvas();
    let mut canvas = IronCanvas::create(grid.clone(), overlay).expect("create IronCanvas");
    canvas
        .setModel(make_fixture_model(store))
        .expect("fixture model passes the duck test");
    canvas.resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR);
    (canvas, grid)
}

/// Raw RGBA backing-store bytes for `canvas`'s current pixels, read
/// through a `CanvasRenderingContext2d` on the same element `IronCanvas`
/// painted into (`getContext("2d")` is idempotent — this doesn't disturb
/// the live painter's own handle on the same canvas).
fn grid_pixels(canvas: &HtmlCanvasElement) -> Vec<u8> {
    let ctx = canvas
        .get_context("2d")
        .expect("getContext must not throw")
        .expect("2d context must exist")
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .expect("context is CanvasRenderingContext2d");
    ctx.get_image_data(0.0, 0.0, canvas.width() as f64, canvas.height() as f64)
        .expect("get_image_data must succeed on an opaque, same-origin canvas")
        .data()
        .0
}

/// Acceptance criterion 1: a border-free single-cell value edit takes the
/// `RepaintPlan::Rows` path (proven natively by
/// `row_band_repaint_paints_only_the_changed_row_band` in
/// `paint_skip.rs`) — its raster output must be byte-identical to a second
/// canvas painting the same final state fresh in one shot.
#[wasm_bindgen_test]
fn partial_repaint_matches_forced_fresh_for_border_free_change() {
    let store = plain_fixture_store();
    let (mut canvas, grid) = canvas_over(Rc::clone(&store));
    canvas.paint_if_dirty(); // baseline

    store.borrow_mut().get_mut(&(10, 2)).expect("seeded").value = "changed-10-2".to_string();
    canvas.mark_content_dirty();
    canvas.paint_if_dirty(); // partial repaint

    let fresh_store = plain_fixture_store();
    fresh_store
        .borrow_mut()
        .get_mut(&(10, 2))
        .expect("seeded")
        .value = "changed-10-2".to_string();
    let (mut fresh_canvas, fresh_grid) = canvas_over(fresh_store);
    fresh_canvas.paint_if_dirty(); // single Fresh paint of the final state

    assert_eq!(
        grid_pixels(&grid),
        grid_pixels(&fresh_grid),
        "a border-free row-band repaint must raster identically to a forced-fresh full repaint"
    );
}

/// Acceptance criterion 2a: an explicit bottom border on the row ABOVE the
/// edited row — present and UNCHANGED across both frames — trips
/// `plan_pane_repaint`'s border-safety check (the changed row's own top
/// boundary carries risk from that neighbour) and forces the conservative
/// `RepaintPlan::Full` fallback rather than a narrow row-band repaint. The
/// fallback's raster output must still be byte-identical to forced-fresh.
#[wasm_bindgen_test]
fn partial_repaint_matches_forced_fresh_when_neighbor_row_keeps_bottom_border() {
    let build_store = || {
        let store = plain_fixture_store();
        store
            .borrow_mut()
            .get_mut(&(9, 1))
            .expect("seeded")
            .border_bottom = true;
        store
    };

    let store = build_store();
    let (mut canvas, grid) = canvas_over(Rc::clone(&store));
    canvas.paint_if_dirty(); // baseline (row 9's bottom border already present)

    store.borrow_mut().get_mut(&(10, 2)).expect("seeded").value = "changed".to_string();
    canvas.mark_content_dirty();
    canvas.paint_if_dirty(); // must fall back to Full — row 9 owns the shared edge

    let fresh_store = build_store();
    fresh_store
        .borrow_mut()
        .get_mut(&(10, 2))
        .expect("seeded")
        .value = "changed".to_string();
    let (mut fresh_canvas, fresh_grid) = canvas_over(fresh_store);
    fresh_canvas.paint_if_dirty();

    assert_eq!(
        grid_pixels(&grid),
        grid_pixels(&fresh_grid),
        "the border-safety Full fallback must raster identically to forced-fresh even when \
         the surviving border belongs to an untouched neighbour row"
    );
}

/// Acceptance criterion 2b: the CHANGED row's own bottom border disappears
/// this frame (present in `painted`, absent in `scratch`) — an internal
/// span-boundary change, not just a neighbour's static state — which also
/// forces `RepaintPlan::Full` (`fingerprint.rs`'s "old border removed"
/// arm). Byte-identical to forced-fresh proves the fallback actually
/// erases the stale stroke correctly, not just that *some* repaint
/// happened.
#[wasm_bindgen_test]
fn partial_repaint_matches_forced_fresh_when_changed_rows_own_border_is_removed() {
    let store = plain_fixture_store();
    store
        .borrow_mut()
        .get_mut(&(10, 1))
        .expect("seeded")
        .border_bottom = true;
    let (mut canvas, grid) = canvas_over(Rc::clone(&store));
    canvas.paint_if_dirty(); // baseline (row 10 has a bottom border)

    store
        .borrow_mut()
        .get_mut(&(10, 1))
        .expect("seeded")
        .border_bottom = false;
    canvas.mark_content_dirty();
    canvas.paint_if_dirty(); // must fall back to Full and erase the stroke

    let fresh_store = plain_fixture_store(); // final state: no border anywhere
    let (mut fresh_canvas, fresh_grid) = canvas_over(fresh_store);
    fresh_canvas.paint_if_dirty();

    assert_eq!(
        grid_pixels(&grid),
        grid_pixels(&fresh_grid),
        "removing the changed row's own border must fall back to Full and raster identically \
         to forced-fresh — a stale stroke left behind would be a border-erasure bug"
    );
}
