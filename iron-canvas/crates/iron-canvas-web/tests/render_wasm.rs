#![cfg(target_arch = "wasm32")]
//! Browser-only proof that fractional DPR reaches the real `<canvas>`
//! backing store through the `IronCanvas` facade. Regression test for a
//! bug where `resize()` rounded `dpr` before forwarding it to
//! `WebSurface::resize`, silently mapping e.g. 1.25 -> 1 and 1.5 -> 2.

use iron_canvas_web::{CanvasSize, CanvasView, IronCanvas, JsPaintResult, RCRange};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

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

#[wasm_bindgen_test]
fn fractional_dpr_reaches_canvas_backing_store() {
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay.clone()) else {
        panic!("create IronCanvas");
    };

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

use iron_canvas_core::{CellDecoration, CellKind, CellStyle, Fetched};
use iron_canvas_web::wasm::JsBackedModel;
use iron_canvas_web::{CanvasModel, CellContentQuery};
use wasm_bindgen::JsValue;

/// Minimal duck-typed model handle: `try_from_js_value` requires
/// `getSelectedView`, `getSelectedSheet`, `getFrozenRowsCount`, and
/// `getFrozenColumnsCount`; extra methods are supplied per test.
fn model_with_methods(methods: &[(&str, &js_sys::Function)]) -> JsBackedModel {
    let obj = js_sys::Object::new();
    let view = js_sys::Function::new_no_args(
        "return { sheet: 0, row: 1, column: 1, range: [1, 1, 1, 1], top_row: 1, left_column: 1 };",
    );
    let Ok(_) = js_sys::Reflect::set(&obj, &JsValue::from_str("getSelectedView"), &view) else {
        panic!("set getSelectedView on plain object");
    };
    // Most engine call sites read the sheet via this standalone accessor
    // rather than `getSelectedView`'s embedded `sheet` field; every fixture
    // in this file pins sheet 0, so a missing accessor here silently throws
    // and falls back to 0 without ever failing a test.
    let get_sheet = js_sys::Function::new_no_args("return 0;");
    let Ok(_) = js_sys::Reflect::set(&obj, &JsValue::from_str("getSelectedSheet"), &get_sheet)
    else {
        panic!("set getSelectedSheet on plain object");
    };
    // `try_from_js_value` requires these two structurally alongside
    // `getSelectedSheet`/`getSelectedView` (Stage 3 Task 1) — every fixture
    // in this file is unfrozen, so both return `0`.
    let get_frozen_rows = js_sys::Function::new_no_args("return 0;");
    let Ok(_) = js_sys::Reflect::set(
        &obj,
        &JsValue::from_str("getFrozenRowsCount"),
        &get_frozen_rows,
    ) else {
        panic!("set getFrozenRowsCount on plain object");
    };
    let get_frozen_cols = js_sys::Function::new_no_args("return 0;");
    let Ok(_) = js_sys::Reflect::set(
        &obj,
        &JsValue::from_str("getFrozenColumnsCount"),
        &get_frozen_cols,
    ) else {
        panic!("set getFrozenColumnsCount on plain object");
    };
    for (name, f) in methods {
        let Ok(_) = js_sys::Reflect::set(&obj, &JsValue::from_str(name), f) else {
            panic!("set model method on plain object");
        };
    }
    let Ok(model) = JsBackedModel::try_from_js_value(obj.into()) else {
        panic!("object passes the setModel duck-test");
    };
    model
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
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;

const FIXTURE_ROWS: i32 = 20;
const FIXTURE_COLS: i32 = 5;
const FIXTURE_CANVAS_W: f64 = 400.0;
const FIXTURE_CANVAS_H: f64 = 400.0;
const FIXTURE_DPR: f64 = 1.0;

/// One fixture cell's editable state: formatted text, independent top/bottom
/// border flags — both of which feed `RowFingerprint::has_any_explicit_border`,
/// the single flag `plan_pane_repaint`'s border-safety check reads — plus the
/// two channels Stage 6 Task 7's offset raster cases need.
///
/// `border_left` is a *vertical* edge whose weight is chosen per cell, because
/// Task 7's border case is specifically a medium/thick stroke: a thin stroke
/// stays inside its own cell rect, while a medium or thick one is drawn wide
/// enough to bleed into the neighbouring row's pixels, which is exactly the
/// stale-stroke risk a retained blit band must not hide.
///
/// `fill` is the conditional-format channel *as the browser host expresses it*.
/// `JsBackedModel` has no `getCellDecorations` accessor at all — its
/// `get_cell_style` doc states the JS `getCellStyle` extern must return the
/// **dxf-merged** style — so a CF change reaches this renderer as a changed
/// fill colour, never as a `CellDecoration`. Driving CF through `fill` is
/// therefore the faithful browser analogue, not a simplification of one.
#[derive(Clone, Default)]
struct FixtureCell {
    value: String,
    border_top: Option<ic::BorderStyle>,
    border_bottom: Option<ic::BorderStyle>,
    border_left: Option<ic::BorderStyle>,
    border_right: Option<ic::BorderStyle>,
    fill: Option<String>,
    wrap: bool,
}

#[derive(Clone, Copy)]
enum FixtureSide {
    Top,
    Right,
    Bottom,
    Left,
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
                    ..FixtureCell::default()
                },
            );
        }
    }
    Rc::new(RefCell::new(cells))
}

fn set_prop(obj: &js_sys::Object, name: &str, f: &js_sys::Function) {
    let Ok(_) = js_sys::Reflect::set(obj, &JsValue::from_str(name), f) else {
        panic!("set fixture model method");
    };
}

fn set_value_prop(obj: &js_sys::Object, name: &str, value: &JsValue) {
    let Ok(_) = js_sys::Reflect::set(obj, &JsValue::from_str(name), value) else {
        panic!("set fixture model value");
    };
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
    // Most engine call sites read the sheet via this standalone accessor
    // rather than `getSelectedView`'s embedded `sheet` field; this fixture
    // pins sheet 0, so a missing accessor here silently throws and falls
    // back to 0 without ever failing a test.
    set_prop(
        &obj,
        "getSelectedSheet",
        &js_sys::Function::new_no_args("return 0;"),
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

    let style_store = Rc::clone(&store);
    let get_style = Closure::wrap(Box::new(move |_sheet: u32, row: i32, col: i32| -> JsValue {
        let cell = style_store
            .borrow()
            .get(&(row, col))
            .cloned()
            .unwrap_or_default();
        let style = ic::Style {
            alignment: cell.wrap.then_some(ic::Alignment {
                wrap_text: true,
                ..ic::Alignment::default()
            }),
            border: ic::Border {
                top: cell.border_top.map(|style| ic::BorderItem {
                    style,
                    color: ic::Color::None,
                }),
                bottom: cell.border_bottom.map(|style| ic::BorderItem {
                    style,
                    color: ic::Color::None,
                }),
                left: cell.border_left.map(|style| ic::BorderItem {
                    style,
                    color: ic::Color::None,
                }),
                right: cell.border_right.map(|style| ic::BorderItem {
                    style,
                    color: ic::Color::None,
                }),
                ..ic::Border::default()
            },
            fill: ic::Fill {
                color: match &cell.fill {
                    Some(rgb) => ic::Color::Rgb(rgb.clone()),
                    None => ic::Color::None,
                },
            },
            ..ic::Style::default()
        };
        let Ok(value) = serde_wasm_bindgen::to_value(&style) else {
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

struct ScrollFailureControls {
    top_row: Rc<Cell<i32>>,
    fail_from_row: Rc<Cell<Option<i32>>>,
}

/// Build the same fixture with a live scroll origin and a controllable invalid
/// style payload from a chosen row onward. Keeping the active cell outside
/// those rows lets `Chrome::classify` approve the Viewport plan; decoding then
/// yields `BridgeFailed` during revealed-strip preflight, where the transaction
/// must hold.
fn make_scroll_failure_fixture_model(
    store: FixtureStore,
    top_row: i32,
) -> (JsValue, ScrollFailureControls) {
    let model = make_fixture_model(store);
    let obj: js_sys::Object = model.unchecked_into();

    let top_row = Rc::new(Cell::new(top_row));
    let view_top_row = Rc::clone(&top_row);
    let get_view = Closure::wrap(Box::new(move || -> JsValue {
        let view = js_sys::Object::new();
        set_value_prop(&view, "sheet", &JsValue::from_f64(0.0));
        set_value_prop(&view, "row", &JsValue::from_f64(5.0));
        set_value_prop(&view, "column", &JsValue::from_f64(1.0));
        let range = js_sys::Array::new();
        for value in [5.0, 1.0, 5.0, 1.0] {
            range.push(&JsValue::from_f64(value));
        }
        set_value_prop(&view, "range", &range.into());
        set_value_prop(
            &view,
            "top_row",
            &JsValue::from_f64(f64::from(view_top_row.get())),
        );
        set_value_prop(&view, "left_column", &JsValue::from_f64(1.0));
        view.into()
    }) as Box<dyn Fn() -> JsValue>);
    set_prop(&obj, "getSelectedView", get_view.as_ref().unchecked_ref());
    get_view.forget();
    // The blit overlap probe requires an explicit row-height accessor; the
    // Fresh builder's default-height fallback is intentionally not used there.
    set_prop(
        &obj,
        "getRowHeight",
        &js_sys::Function::new_no_args("return 20;"),
    );

    let fail_from_row = Rc::new(Cell::new(None));
    let style_fail_from_row = Rc::clone(&fail_from_row);
    let get_style = Closure::wrap(
        Box::new(move |_sheet: u32, row: i32, _col: i32| -> JsValue {
            if style_fail_from_row
                .get()
                .is_some_and(|first_failed| row >= first_failed)
            {
                JsValue::NULL
            } else {
                let Ok(value) = serde_wasm_bindgen::to_value(&ic::Style::default()) else {
                    panic!("default fixture Style always serializes");
                };
                value
            }
        }) as Box<dyn Fn(u32, i32, i32) -> JsValue>,
    );
    set_prop(&obj, "getCellStyle", get_style.as_ref().unchecked_ref());
    get_style.forget();

    (
        obj.into(),
        ScrollFailureControls {
            top_row,
            fail_from_row,
        },
    )
}

/// Build + resize a fresh `IronCanvas` over `store`. All three scenarios
/// below share this so canvas size, DPR, and model wiring never
/// accidentally diverge between the "partial" and "forced-fresh" side of
/// a comparison.
fn canvas_over(store: FixtureStore) -> (IronCanvas, HtmlCanvasElement) {
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay) else {
        panic!("create IronCanvas");
    };
    let Ok(()) = canvas.set_model_js(make_fixture_model(store)) else {
        panic!("fixture model passes the duck test");
    };
    canvas.resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR);
    (canvas, grid)
}

/// Raw RGBA backing-store bytes for `canvas`'s current pixels, read
/// through a `CanvasRenderingContext2d` on the same element `IronCanvas`
/// painted into (`getContext("2d")` is idempotent — this doesn't disturb
/// the live painter's own handle on the same canvas).
fn canvas_pixels(canvas: &HtmlCanvasElement) -> Vec<u8> {
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

fn grid_pixels(canvas: &HtmlCanvasElement) -> Vec<u8> {
    canvas_pixels(canvas)
}

/// A failed Viewport strip fetch is transactional: the visible front canvas
/// and query geometry stay at the prior committed frame, the retained work
/// succeeds without another invalidation after the bridge recovers, and the
/// recovered raster matches a forced-fresh render of the same final viewport.
#[wasm_bindgen_test]
fn held_viewport_recovers_byte_identical_to_forced_fresh() {
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay) else {
        panic!("create IronCanvas");
    };
    let (model, controls) = make_scroll_failure_fixture_model(plain_fixture_store(), 1);
    let Ok(()) = canvas.set_model_js(model) else {
        panic!("scroll-failure fixture model passes the duck test");
    };
    canvas.resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR);
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);

    let baseline_pixels = grid_pixels(&grid);
    let Some(baseline_a1) = canvas.cell_rect(1, 1) else {
        panic!("A1 is visible in the baseline viewport");
    };
    let Some(revealed_row) = (2..=FIXTURE_ROWS + 2).find(|row| canvas.cell_rect(*row, 1).is_none())
    else {
        panic!("fixture has an off-screen row to reveal");
    };

    controls.top_row.set(2);
    controls.fail_from_row.set(Some(revealed_row));
    canvas.view_changed();

    assert_eq!(
        canvas.paint_if_dirty(),
        JsPaintResult::Retry,
        "the revealed-row bridge failure must hold the Viewport transaction"
    );
    assert_eq!(
        grid_pixels(&grid),
        baseline_pixels,
        "a held Viewport attempt must not present partial back-buffer pixels"
    );
    assert_eq!(
        canvas.cell_rect(1, 1),
        Some(baseline_a1),
        "queries must continue to read the last committed frame while held"
    );

    controls.fail_from_row.set(None);
    assert_eq!(
        canvas.paint_if_dirty(),
        JsPaintResult::Painted,
        "retained Viewport work must commit after recovery without a new signal"
    );
    assert!(
        canvas.cell_rect(1, 1).is_none(),
        "A1 must be off-screen after the recovered scroll commits"
    );

    let fresh_grid = make_canvas();
    let fresh_overlay = make_canvas();
    let Ok(mut fresh_canvas) = IronCanvas::create(fresh_grid.clone(), fresh_overlay) else {
        panic!("create forced-fresh IronCanvas");
    };
    let (fresh_model, _) = make_scroll_failure_fixture_model(plain_fixture_store(), 2);
    let Ok(()) = fresh_canvas.set_model_js(fresh_model) else {
        panic!("forced-fresh fixture model passes the duck test");
    };
    fresh_canvas.resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR);
    assert_eq!(fresh_canvas.paint_if_dirty(), JsPaintResult::Painted);

    assert_eq!(
        grid_pixels(&grid),
        grid_pixels(&fresh_grid),
        "recovered Viewport raster must be byte-identical to forced fresh"
    );
}

struct FreshFailureControls {
    fail: Rc<Cell<bool>>,
}

/// Build the shared fixture with a controllable style-fetch failure. Unlike
/// `make_scroll_failure_fixture_model`, no scroll/top-row bookkeeping is
/// needed: with no committed `Chrome` yet, the very first `paint_if_dirty()`
/// on a resized canvas is necessarily a Fresh attempt (`GridWork::Fresh`),
/// so failing every cell's style fetch while `fail` is set strikes Fresh's
/// own bulk grid-segment prepare directly — not `FrameInputs::capture` (already
/// covered by
/// `selected_sheet_bridge_failure_holds_then_recovers_without_another_signal`)
/// and not a Viewport strip reveal (covered above). `getCellStyle` returning
/// `null` is the same decode-failure mechanism
/// `make_scroll_failure_fixture_model` uses: `serde_wasm_bindgen` cannot
/// decode `null` into `Style`, so `JsBackedModel::get_cell_style` reports
/// `Fetched::BridgeFailed`.
fn make_fresh_failure_fixture_model(store: FixtureStore) -> (JsValue, FreshFailureControls) {
    let model = make_fixture_model(store);
    let obj: js_sys::Object = model.unchecked_into();

    let fail = Rc::new(Cell::new(false));
    let style_fail = Rc::clone(&fail);
    let get_style = Closure::wrap(
        Box::new(move |_sheet: u32, _row: i32, _col: i32| -> JsValue {
            if style_fail.get() {
                JsValue::NULL
            } else {
                let Ok(value) = serde_wasm_bindgen::to_value(&ic::Style::default()) else {
                    panic!("default fixture Style always serializes");
                };
                value
            }
        }) as Box<dyn Fn(u32, i32, i32) -> JsValue>,
    );
    set_prop(&obj, "getCellStyle", get_style.as_ref().unchecked_ref());
    get_style.forget();

    (obj.into(), FreshFailureControls { fail })
}

/// A failed Fresh bulk-cell fetch is transactional. With no committed
/// `Chrome` yet, the very first paint attempt is necessarily Fresh
/// (`GridWork::Fresh`); `paint_fresh_regime` prepares every pane (via
/// `build_and_paint_fresh` -> `paint_grid_fresh`) before touching the
/// painter at all, so a bulk style-fetch failure there must leave the front
/// canvas exactly as `resize` left it, answer no committed query geometry,
/// and recover on the very next `paint_if_dirty()` — no `view_changed()` /
/// `markContentDirty()` / `requestRepaint()` call needed — landing
/// byte-identical to a second canvas that painted the same healthy final
/// state fresh in one shot.
#[wasm_bindgen_test]
fn held_fresh_recovers_byte_identical_to_forced_fresh() {
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay) else {
        panic!("create IronCanvas");
    };
    let (model, controls) = make_fresh_failure_fixture_model(plain_fixture_store());
    let Ok(()) = canvas.set_model_js(model) else {
        panic!("fresh-failure fixture model passes the duck test");
    };
    canvas.resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR);

    controls.fail.set(true);
    assert_eq!(
        canvas.paint_if_dirty(),
        JsPaintResult::Retry,
        "a bulk style-fetch failure during the first (Fresh) attempt must hold, not commit"
    );
    assert!(
        grid_pixels(&grid).iter().all(|&b| b == 0),
        "a held Fresh attempt must emit no grid ops and present nothing — the backing store \
         stays exactly as `resize` left it"
    );
    assert!(
        canvas.cell_rect(1, 1).is_none(),
        "no committed frame exists yet, so query geometry must still answer None while held"
    );

    // No `view_changed()` / `markContentDirty()` / `requestRepaint()` call
    // here: a held Fresh attempt leaves `last_frame` untouched and the
    // retry contract merges the complete consumed work back into `pending`
    // (see `paint_fresh_regime`), so the very next `paintIfDirty()` alone
    // must recover.
    controls.fail.set(false);
    assert_eq!(
        canvas.paint_if_dirty(),
        JsPaintResult::Painted,
        "the recovered attempt must paint without another invalidation call"
    );
    assert!(
        canvas.cell_rect(1, 1).is_some(),
        "A1 must be on screen once the recovered Fresh attempt commits"
    );

    let fresh_grid = make_canvas();
    let fresh_overlay = make_canvas();
    let Ok(mut fresh_canvas) = IronCanvas::create(fresh_grid.clone(), fresh_overlay) else {
        panic!("create forced-fresh IronCanvas");
    };
    let Ok(()) = fresh_canvas.set_model_js(make_fixture_model(plain_fixture_store())) else {
        panic!("forced-fresh fixture model passes the duck test");
    };
    fresh_canvas.resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR);
    assert_eq!(fresh_canvas.paint_if_dirty(), JsPaintResult::Painted);

    assert_eq!(
        grid_pixels(&grid),
        grid_pixels(&fresh_grid),
        "recovered Fresh raster must be byte-identical to a forced-fresh render of the same \
         healthy final state"
    );
}

// ==============================================================================
// Stage 4 Task 6: no browser-level gate for selected-Viewport/effective-
// FreshFallback bulk-bridge failure at a row-header digit boundary.
//
// `crates/iron-canvas-core/tests/blit_fallback.rs`'s
// `held_fresh_fallback_at_row_header_digit_boundary_holds_atomically`
// already proves this exact scenario end to end — `top_row` moved from 980
// to 981 so the last visible row crosses 999 -> 1000 (3 -> 4 header digits),
// which rejects in-place blit reuse (`Chrome::next_blit` returns
// `BlitOutcome::FreshFallback`), and a bulk bridge failure on that demoted
// Fresh candidate holds atomically: zero new grid ops, zero presents, and
// query geometry pinned to the pre-attempt frame — with `last_regime`
// staying `Viewport` while `last_trace().effective` reads `None`.
//
// Reproducing that fixture through this file's duck-typed JS harness would
// need a synthetic ~1000-row model plus pixel-exact control over where the
// real (not stubbed) Canvas2D `measureText`-based row-header width actually
// widens — extra fixture surface bought for no new coverage: `FreshFallback`
// converges on the exact same `build_and_paint_fresh` prepare-then-decide
// tail and the same `finish_attempt` completion boundary that
// `held_fresh_recovers_byte_identical_to_forced_fresh` (above) and
// `held_viewport_recovers_byte_identical_to_forced_fresh` already drive
// through the real `WebSurface`/`CanvasPainter` stack. A browser `ImageData`
// comparison here would only vary the dispatch *entry path* (arrival via
// blit demotion instead of direct `GridWork::Fresh` selection) — exactly
// the part the native recorder-backed test already isolates, and with
// strictly stronger assertions (op *counts*, not just unchanged pixels).
// ==============================================================================

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

    {
        let mut cells = store.borrow_mut();
        let Some(cell) = cells.get_mut(&(10, 2)) else {
            panic!("seeded");
        };
        cell.value = "changed-10-2".to_string();
    }
    canvas.mark_content_dirty();
    canvas.paint_if_dirty(); // partial repaint

    let fresh_store = plain_fixture_store();
    {
        let mut cells = fresh_store.borrow_mut();
        let Some(cell) = cells.get_mut(&(10, 2)) else {
            panic!("seeded");
        };
        cell.value = "changed-10-2".to_string();
    }
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
        {
            let mut cells = store.borrow_mut();
            let Some(cell) = cells.get_mut(&(9, 1)) else {
                panic!("seeded");
            };
            cell.border_bottom = Some(ic::BorderStyle::Thin);
        }
        store
    };

    let store = build_store();
    let (mut canvas, grid) = canvas_over(Rc::clone(&store));
    canvas.paint_if_dirty(); // baseline (row 9's bottom border already present)

    {
        let mut cells = store.borrow_mut();
        let Some(cell) = cells.get_mut(&(10, 2)) else {
            panic!("seeded");
        };
        cell.value = "changed".to_string();
    }
    canvas.mark_content_dirty();
    canvas.paint_if_dirty(); // must fall back to Full — row 9 owns the shared edge

    let fresh_store = build_store();
    {
        let mut cells = fresh_store.borrow_mut();
        let Some(cell) = cells.get_mut(&(10, 2)) else {
            panic!("seeded");
        };
        cell.value = "changed".to_string();
    }
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
    {
        let mut cells = store.borrow_mut();
        let Some(cell) = cells.get_mut(&(10, 1)) else {
            panic!("seeded");
        };
        cell.border_bottom = Some(ic::BorderStyle::Thin);
    }
    let (mut canvas, grid) = canvas_over(Rc::clone(&store));
    canvas.paint_if_dirty(); // baseline (row 10 has a bottom border)

    {
        let mut cells = store.borrow_mut();
        let Some(cell) = cells.get_mut(&(10, 1)) else {
            panic!("seeded");
        };
        cell.border_bottom = None;
    }
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

// ==============================================================================
// Task 4 Step 6 (deferred): `Orchestrator::resize` is self-invalidating — a
// real size or DPR change alone must force the next `paintIfDirty` to Fresh,
// with no follow-up `requestRepaint()` needed. Proven the same way as Task 6
// above: raster bytes after the self-invalidating path must be identical to
// a second canvas painting the same final size/DPR fresh in one shot.
// ==============================================================================

/// Acceptance criterion: a full resize (both CSS size and DPR change)
/// followed by a bare `paintIfDirty()` — no `requestRepaint()` — must
/// raster identically to a canvas built directly at the new size/DPR.
#[wasm_bindgen_test]
fn resize_self_invalidates_without_explicit_repaint() {
    const OLD_W: f64 = 300.0;
    const OLD_H: f64 = 250.0;
    const OLD_DPR: f64 = 1.0;
    const NEW_W: f64 = 450.0;
    const NEW_H: f64 = 320.0;
    const NEW_DPR: f64 = 1.5;

    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay) else {
        panic!("create IronCanvas");
    };
    let Ok(()) = canvas.set_model_js(make_fixture_model(plain_fixture_store())) else {
        panic!("fixture model passes the duck test");
    };
    canvas.resize(OLD_W, OLD_H, OLD_DPR);
    canvas.paint_if_dirty(); // baseline Fresh paint at the old size

    canvas.resize(NEW_W, NEW_H, NEW_DPR);
    canvas.paint_if_dirty(); // bare paintIfDirty — no requestRepaint()

    let fresh_grid = make_canvas();
    let fresh_overlay = make_canvas();
    let Ok(mut fresh_canvas) = IronCanvas::create(fresh_grid.clone(), fresh_overlay) else {
        panic!("create IronCanvas");
    };
    let Ok(()) = fresh_canvas.set_model_js(make_fixture_model(plain_fixture_store())) else {
        panic!("fixture model passes the duck test");
    };
    fresh_canvas.resize(NEW_W, NEW_H, NEW_DPR);
    fresh_canvas.paint_if_dirty(); // single Fresh paint straight at the new size/DPR

    assert_eq!(
        grid_pixels(&grid),
        grid_pixels(&fresh_grid),
        "resize must self-invalidate so a bare paintIfDirty() after it matches a forced-fresh \
         render at the new size/DPR, with no requestRepaint() needed"
    );
}

/// Acceptance criterion: a DPR-only resize (CSS size unchanged) followed
/// by a bare `paintIfDirty()` must raster identically to a canvas built
/// directly at the new DPR.
#[wasm_bindgen_test]
fn dpr_only_resize_self_invalidates_without_explicit_repaint() {
    const W: f64 = 300.0;
    const H: f64 = 250.0;
    const OLD_DPR: f64 = 1.0;
    const NEW_DPR: f64 = 2.0;

    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay) else {
        panic!("create IronCanvas");
    };
    let Ok(()) = canvas.set_model_js(make_fixture_model(plain_fixture_store())) else {
        panic!("fixture model passes the duck test");
    };
    canvas.resize(W, H, OLD_DPR);
    canvas.paint_if_dirty(); // baseline Fresh paint at the old DPR

    canvas.resize(W, H, NEW_DPR); // CSS size unchanged, DPR-only change
    canvas.paint_if_dirty(); // bare paintIfDirty — no requestRepaint()

    let fresh_grid = make_canvas();
    let fresh_overlay = make_canvas();
    let Ok(mut fresh_canvas) = IronCanvas::create(fresh_grid.clone(), fresh_overlay) else {
        panic!("create IronCanvas");
    };
    let Ok(()) = fresh_canvas.set_model_js(make_fixture_model(plain_fixture_store())) else {
        panic!("fixture model passes the duck test");
    };
    fresh_canvas.resize(W, H, NEW_DPR);
    fresh_canvas.paint_if_dirty(); // single Fresh paint straight at the new DPR

    assert_eq!(
        grid_pixels(&grid),
        grid_pixels(&fresh_grid),
        "a DPR-only resize must self-invalidate so a bare paintIfDirty() after it matches a \
         forced-fresh render at the new DPR, with no requestRepaint() needed"
    );
}

// ==============================================================================
// Task 3 (Stage 2): the `view_changed()` / `viewChanged()` API and its
// dispatch-matrix guarantees — a navigation-only notification still wakes a
// paint, a no-shift notification never touches grid pixels, and a sheet
// switch at identical coordinates can't reuse the wrong sheet's cached text.
// ==============================================================================

/// Acceptance criterion: `view_changed()` alone — no scroll, no content
/// change — still wakes `paint_if_dirty()` (proving navigation-only intent
/// reaches a paint attempt instead of going `Idle`), and because nothing
/// actually shifted on screen the dispatch matrix's `Overlay` fallback
/// applies: the GRID canvas's pixels must be untouched. Only the overlay
/// layer (selection rectangle, etc.) may have repainted.
#[wasm_bindgen_test]
fn view_changed_wakes_a_paint_without_shifting_grid_pixels() {
    let (mut canvas, grid) = canvas_over(plain_fixture_store());
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted); // baseline
    let baseline_pixels = grid_pixels(&grid);

    canvas.view_changed();
    assert_eq!(
        canvas.paint_if_dirty(),
        JsPaintResult::Painted,
        "view_changed() alone must wake the next paint_if_dirty(), not go Idle"
    );
    assert_eq!(
        grid_pixels(&grid),
        baseline_pixels,
        "a view notification with no pixel shift must not repaint the grid layer"
    );
}

/// Duck-typed model whose active sheet is driven by a shared `Cell` and
/// whose formatted value is a pure function of `(sheet, row, col)` — the
/// same (row, col) coordinates resolve to a DIFFERENT string per sheet.
/// Every other fixture in this file pins `sheet: 0` throughout; this is the
/// one axis that needs to vary to prove pane buffers don't survive a sheet
/// switch at identical visible coordinates.
fn make_active_sheet_fixture_model(active_sheet: Rc<Cell<u32>>) -> JsValue {
    let obj = js_sys::Object::new();

    let view_sheet = Rc::clone(&active_sheet);
    let get_view = Closure::wrap(Box::new(move || -> JsValue {
        let view = js_sys::Object::new();
        set_value_prop(&view, "sheet", &JsValue::from(view_sheet.get()));
        set_value_prop(&view, "row", &JsValue::from(1i32));
        set_value_prop(&view, "column", &JsValue::from(1i32));
        let range = js_sys::Array::new();
        for value in [1i32, 1, 1, 1] {
            range.push(&JsValue::from(value));
        }
        set_value_prop(&view, "range", &range.into());
        set_value_prop(&view, "top_row", &JsValue::from(1i32));
        set_value_prop(&view, "left_column", &JsValue::from(1i32));
        view.into()
    }) as Box<dyn Fn() -> JsValue>);
    set_prop(&obj, "getSelectedView", get_view.as_ref().unchecked_ref());
    get_view.forget();

    // `getSelectedView`'s embedded `sheet` field is not the same call the
    // engine uses everywhere else: chrome building, selection decoration,
    // autofit, slot geometry, and the renderer all call the standalone
    // `getSelectedSheet()` accessor directly. Every other fixture in this
    // file pins sheet 0 throughout, so an unimplemented `getSelectedSheet`
    // silently throwing-and-defaulting to 0 was never observable before
    // this fixture — here it must track the same `active_sheet` cell.
    let selected_sheet = Rc::clone(&active_sheet);
    let get_sheet =
        Closure::wrap(Box::new(move || -> u32 { selected_sheet.get() }) as Box<dyn Fn() -> u32>);
    set_prop(&obj, "getSelectedSheet", get_sheet.as_ref().unchecked_ref());
    get_sheet.forget();

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
            panic!("default fixture Style always serializes");
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

    // Pure function of the live (sheet, row, col) the bridge passes on every
    // call — no shared store needed, since the engine always queries with
    // whichever sheet is currently active.
    let get_value = Closure::wrap(Box::new(|sheet: u32, row: i32, col: i32| -> String {
        format!("sheet{sheet}-r{row}c{col}")
    }) as Box<dyn Fn(u32, i32, i32) -> String>);
    set_prop(
        &obj,
        "getFormattedCellValue",
        get_value.as_ref().unchecked_ref(),
    );
    get_value.forget();

    obj.into()
}

/// Regression test for the Stage 2 global constraint: "pane-buffer ranges
/// do not carry sheet identity, so an active-sheet change with identical
/// coordinates must not reuse the prior sheet's values." Both sheets show
/// the exact same visible (row, col) window — only the active-sheet id
/// differs — so a cache keyed on screen position alone (not sheet) would
/// silently keep painting sheet 0's text after switching to sheet 1.
#[wasm_bindgen_test]
fn active_sheet_change_repaints_new_sheets_values_at_identical_coordinates() {
    let active_sheet = Rc::new(Cell::new(0u32));
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay) else {
        panic!("create IronCanvas");
    };
    let Ok(()) = canvas.set_model_js(make_active_sheet_fixture_model(Rc::clone(&active_sheet))) else {
        panic!("active-sheet fixture model passes the duck test");
    };
    canvas.resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR);
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted); // sheet 0 baseline
    let sheet0_pixels = grid_pixels(&grid);

    active_sheet.set(1);
    canvas.view_changed(); // sheet switch: view changed, same coordinates
    assert_eq!(
        canvas.paint_if_dirty(),
        JsPaintResult::Painted,
        "an active-sheet change must reach a paint"
    );
    let sheet1_pixels = grid_pixels(&grid);

    assert_ne!(
        sheet1_pixels, sheet0_pixels,
        "identical visible coordinates on a different sheet must repaint with that sheet's \
         own values — a stale pane buffer keyed only on screen position would silently keep \
         painting sheet 0's text"
    );
}

// ==============================================================================
// Stage 3 Task 1: `FrameInputs::capture` failure holds the whole paint
// attempt (no ops, `Retry`); the recovered attempt paints on the very next
// `paintIfDirty()` alone, with no further host signal — the retry contract
// merges the original queued work back into `pending` rather than requiring
// the host to re-raise it.
// ==============================================================================

/// Build the plain fixture model, then replace `getSelectedSheet` with a
/// genuine JS closure (via `eval`, not a Rust `Closure`, so the throw is
/// unambiguous JS-runtime behavior rather than relying on wasm-bindgen's
/// `Result`-return glue) that throws exactly once and returns `0` on every
/// call after.
fn make_sheet_throws_once_fixture_model(store: FixtureStore) -> JsValue {
    let model = make_fixture_model(store);
    let obj: js_sys::Object = model.unchecked_into();

    let throws_once_factory = "\
        (function () { \
            var thrown = false; \
            return function () { \
                if (thrown) { return 0; } \
                thrown = true; \
                throw new Error('simulated getSelectedSheet bridge failure (throws once)'); \
            }; \
        })()";
    let Ok(get_sheet) = js_sys::eval(throws_once_factory) else {
        panic!("eval must build the throws-once getSelectedSheet closure");
    };
    let get_sheet: js_sys::Function = get_sheet.unchecked_into();
    set_prop(&obj, "getSelectedSheet", &get_sheet);

    obj.into()
}

#[wasm_bindgen_test]
fn selected_sheet_bridge_failure_holds_then_recovers_without_another_signal() {
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay) else {
        panic!("create IronCanvas");
    };
    let Ok(()) = canvas.set_model_js(make_sheet_throws_once_fixture_model(plain_fixture_store()))
    else {
        panic!("sheet-throws-once fixture model passes the duck test");
    };
    canvas.resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR);

    assert_eq!(
        canvas.paint_if_dirty(),
        JsPaintResult::Retry,
        "a getSelectedSheet throw during capture must hold the first paint attempt"
    );
    assert!(
        grid_pixels(&grid).iter().all(|&b| b == 0),
        "a held capture-failure attempt must emit no grid ops at all — the backing store \
         stays exactly as `resize` left it"
    );
    assert!(
        canvas.cell_rect(1, 1).is_none(),
        "no committed frame exists yet, so query geometry must still answer None"
    );

    // No `view_changed()` / `markContentDirty()` / `requestRepaint()` call
    // here: the retry contract merges the original queued work back into
    // `pending`, so the very next `paintIfDirty()` alone must recover.
    assert_eq!(
        canvas.paint_if_dirty(),
        JsPaintResult::Painted,
        "the recovered attempt must paint without another invalidation call"
    );
    assert!(
        !grid_pixels(&grid).iter().all(|&b| b == 0),
        "the recovered paint must actually draw the grid"
    );
}

// ==============================================================================
// Stage 6, Task 1: ignored browser timing probes (W4-W8).
//
// End-to-end elapsed time through the real `WebSurface`/Canvas2D stack is the
// one quantity neither the native traffic matrix
// (`iron-canvas-core/tests/stage6_measurements.rs`) nor the private fingerprint
// A/B can supply. These probes measure it and print machine-copyable JSON for
// `docs/performance/2026-08-02-stage-6-render-costs.md`.
//
// Every one of them is `#[ignore]`d: a timing number is evidence for a human
// review, never a CI gate — asserting on it would fail on a slow or loaded
// runner and tell us nothing about correctness. The normal browser suite
// compiles them and skips them. Task 2 runs them explicitly:
//
//   cargo test --release --target wasm32-unknown-unknown -p iron-canvas-web \
//     --test render_wasm stage6_perf -- --ignored --nocapture
//
// One test per workload, deliberately: the browser runner applies its timeout
// per test, and a single test carrying the whole matrix would spend it.
// ==============================================================================

/// Warm-up iterations discarded before the clock matters, per the Stage 6
/// protocol.
const STAGE6_WARMUP: usize = 20;
/// Measured pairs per workload. The plan's floor is 100.
const STAGE6_PAIRS: usize = 100;
/// Height of the one oversized row W7 uses to change the scroll-axis extent by
/// one; every other row keeps the fixture's uniform `STAGE6_ROW_H`.
const STAGE6_TALL_ROW_H: f64 = 45.0;

// ------------------------------------------------------------------------------
// Production-shaped pane geometry.
//
// These probes deliberately do NOT reuse `FIXTURE_CANVAS_W/H/DPR`: that 400x400
// fixture shows a 20 x 7 / 140-cell pane, roughly a quarter of the pane the
// native traffic matrix measures, and Gate C's threshold is stated against the
// production shape. The constants below reproduce
// `stage6_measurements.rs`'s `Shape::production_plain()` exactly — same 29 x 21
// slot count, same 20 px rows, same 80 px columns — so a browser millisecond and
// a native op count describe the same frame. Every other test in this file keeps
// the shared 400x400 constants untouched.
// ------------------------------------------------------------------------------

/// Rows the Stage 6 pane shows, matching `Shape::rows()` for `Production`.
const STAGE6_ROWS: i32 = 29;
/// Columns the Stage 6 pane shows, matching `Shape::cols()` for `Production`.
const STAGE6_COLS: i32 = 21;
/// Uniform row height, matching the native probe's `ROW_H`.
const STAGE6_ROW_H: f64 = 20.0;
/// Uniform column width, matching the native probe's `COL_W`. The fixture model
/// must publish this explicitly: without a `getColumnWidth` accessor the
/// renderer falls back to `DEFAULT_COL_WIDTH` (64 px) and the same canvas would
/// show a different column count.
const STAGE6_COL_W: f64 = 80.0;

/// Canvas that shows exactly `STAGE6_ROWS` x `STAGE6_COLS` slots: a header plus
/// a slot run one slot short on each axis, because the visible range always
/// admits the trailing slot whose origin lands on the canvas edge.
///
/// Width: 30 px row-header + 20 x 80 px = 1630.
///
/// Note the absence of a `CELL_AREA_INSET` term, which the native probe's
/// equivalent formula carries. The two paths disagree about it by 3 px, and the
/// achieved slot count — asserted by `stage6_assert_full_pane` — is the
/// authority, exactly as `fetch_ranges` is on the native side. Each dimension
/// has a full slot of tolerance either way, so this is a convention difference,
/// not a knife edge.
const STAGE6_CANVAS_W: f64 = 1630.0;
/// Height: 28 px column-header + 28 x 20 px = 588.
const STAGE6_CANVAS_H: f64 = 588.0;
/// DPR 1.0, as in the native matrix, so backing-store scaling is not a variable.
const STAGE6_DPR: f64 = 1.0;

/// `fetched_cell_slots` for one full-grid fetch: four bulk content accessors
/// over 29 x 21 = 609 logical cells. The native matrix reports the same 2,436
/// for every prod29x21 full-grid row.
const STAGE6_FULL_PANE_SLOTS: i32 = 4 * STAGE6_ROWS * STAGE6_COLS;

/// Plain "rNcM" text over the whole Stage 6 pane, one row and one column deeper
/// than the pane shows so a single-step scroll on either axis reveals a
/// populated slot rather than a blank one. `plain_fixture_store`'s 20 x 5 grid
/// would leave five sixths of a production-shaped pane empty and understate
/// every painter cost measured here.
fn stage6_fixture_store() -> FixtureStore {
    let mut cells = HashMap::new();
    for row in 1..=STAGE6_ROWS + 1 {
        for col in 1..=STAGE6_COLS + 1 {
            cells.insert(
                (row, col),
                FixtureCell {
                    value: format!("r{row}c{col}"),
                    ..FixtureCell::default()
                },
            );
        }
    }
    Rc::new(RefCell::new(cells))
}

#[derive(Clone)]
struct StableViewFixture {
    active_row: Rc<Cell<i32>>,
    active_col: Rc<Cell<i32>>,
    selection: Rc<Cell<[i32; 4]>>,
    top_row: Rc<Cell<i32>>,
    left_column: Rc<Cell<i32>>,
    frozen_rows: Rc<Cell<i32>>,
    frozen_cols: Rc<Cell<i32>>,
    show_selection: Rc<Cell<bool>>,
    row_heights: Rc<RefCell<HashMap<i32, f64>>>,
    column_widths: Rc<RefCell<HashMap<i32, f64>>>,
}

impl StableViewFixture {
    fn new(row: i32, col: i32) -> Self {
        Self {
            active_row: Rc::new(Cell::new(row)),
            active_col: Rc::new(Cell::new(col)),
            selection: Rc::new(Cell::new([row, col, row, col])),
            top_row: Rc::new(Cell::new(1)),
            left_column: Rc::new(Cell::new(1)),
            frozen_rows: Rc::new(Cell::new(0)),
            frozen_cols: Rc::new(Cell::new(0)),
            show_selection: Rc::new(Cell::new(true)),
            row_heights: Rc::new(RefCell::new(HashMap::new())),
            column_widths: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    fn with_frozen(self, rows: i32, cols: i32) -> Self {
        self.frozen_rows.set(rows);
        self.frozen_cols.set(cols);
        self
    }

    fn set_active(&self, row: i32, col: i32) {
        self.active_row.set(row);
        self.active_col.set(col);
        self.selection.set([row, col, row, col]);
    }

    fn set_show_selection(&self, show: bool) {
        self.show_selection.set(show);
    }

    fn set_row_height(&self, row: i32, height: f64) {
        self.row_heights.borrow_mut().insert(row, height);
    }

    fn set_column_width(&self, column: i32, width: f64) {
        self.column_widths.borrow_mut().insert(column, width);
    }

    fn snapshot(&self) -> CanvasView {
        CanvasView {
            sheet: 0,
            row: self.active_row.get(),
            column: self.active_col.get(),
            selection: RCRange::from(self.selection.get()),
            top_row: self.top_row.get(),
            left_column: self.left_column.get(),
        }
    }
}

struct StableFixtureModel {
    content: JsBackedModel,
    view: StableViewFixture,
}

impl CellContentQuery for StableFixtureModel {
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellStyle> {
        self.content.get_cell_style(sheet, row, column)
    }

    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellKind> {
        self.content.get_cell_type(sheet, row, column)
    }

    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Fetched<String> {
        self.content.get_formatted_cell_value(sheet, row, column)
    }

    fn get_extended_cell_style(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Fetched<CellDecoration> {
        self.content.get_extended_cell_style(sheet, row, column)
    }

    fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellStyle>>) {
        self.content.get_cell_styles_in(sheet, range, out);
    }

    fn get_formatted_cell_values_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Fetched<String>>,
    ) {
        self.content.get_formatted_cell_values_in(sheet, range, out);
    }

    fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellKind>>) {
        self.content.get_cell_types_in(sheet, range, out);
    }

    fn get_cell_decorations_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Fetched<CellDecoration>>,
    ) {
        self.content.get_cell_decorations_in(sheet, range, out);
    }
}

impl CanvasModel for StableFixtureModel {
    fn get_selected_sheet(&self) -> Option<u32> {
        Some(0)
    }

    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(self.view.snapshot())
    }

    fn get_frozen_rows_count(&self, _sheet: u32) -> Option<i32> {
        Some(self.view.frozen_rows.get())
    }

    fn get_frozen_columns_count(&self, _sheet: u32) -> Option<i32> {
        Some(self.view.frozen_cols.get())
    }

    fn get_row_height(&self, _sheet: u32, row: i32) -> Option<f64> {
        Some(
            self.view
                .row_heights
                .borrow()
                .get(&row)
                .copied()
                .unwrap_or(STAGE6_ROW_H),
        )
    }

    fn get_column_width(&self, _sheet: u32, column: i32) -> Option<f64> {
        Some(
            self.view
                .column_widths
                .borrow()
                .get(&column)
                .copied()
                .unwrap_or(STAGE6_COL_W),
        )
    }

    fn get_show_grid_lines(&self, _sheet: u32) -> Option<bool> {
        Some(true)
    }

    fn get_show_selection(&self) -> bool {
        self.view.show_selection.get()
    }
}

fn stable_canvas_over(
    store: FixtureStore,
    view: StableViewFixture,
) -> (IronCanvas, HtmlCanvasElement, HtmlCanvasElement) {
    stable_canvas_over_at(store, view, STAGE6_CANVAS_W, STAGE6_CANVAS_H, STAGE6_DPR)
}

fn stable_canvas_over_at(
    store: FixtureStore,
    view: StableViewFixture,
    width: f64,
    height: f64,
    dpr: f64,
) -> (IronCanvas, HtmlCanvasElement, HtmlCanvasElement) {
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay.clone()) else {
        panic!("create stable-view IronCanvas");
    };
    let Ok(content) = JsBackedModel::try_from_js_value(make_fixture_model(store)) else {
        panic!("stable-view fixture content model passes the duck test");
    };
    canvas.set_model(Rc::new(StableFixtureModel { content, view }));
    canvas.resize(width, height, dpr);
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    (canvas, grid, overlay)
}

/// Fail loudly when a frame that should have covered the whole production-shaped
/// pane did not. Geometry drift — a moved constant, a `getColumnWidth` accessor
/// that stopped being read, a default height sneaking back in — would silently
/// retune every timing number these probes emit, so the achieved shape is
/// checked rather than assumed.
fn stage6_assert_full_pane(trace: &str, phase: &str) {
    let expected = format!("fetched={STAGE6_FULL_PANE_SLOTS}");
    assert!(
        trace.ends_with(&expected),
        "Stage 6 {phase} must fetch the production-shaped pane \
         ({STAGE6_FULL_PANE_SLOTS} logical slots = 4 accessors x {rows} x {cols} cells), \
         but its frame trace was `{trace}`",
        rows = STAGE6_ROWS,
        cols = STAGE6_COLS,
    );
}

/// `performance.now()` reached through `js_sys::Reflect` rather than
/// `web_sys::Window::performance`, which would need a `web-sys` feature this
/// crate does not enable.
struct PerfClock {
    perf: JsValue,
    now: js_sys::Function,
}

impl PerfClock {
    fn new() -> Self {
        let global = js_sys::global();
        let Ok(perf) = js_sys::Reflect::get(&global, &JsValue::from_str("performance")) else {
            panic!("the browser test runner must expose a global `performance` object");
        };
        let Ok(now) = js_sys::Reflect::get(&perf, &JsValue::from_str("now")) else {
            panic!("`performance.now` must exist");
        };
        Self {
            perf,
            now: now.unchecked_into(),
        }
    }

    fn now_ms(&self) -> f64 {
        let Ok(value) = self.now.call0(&self.perf) else {
            panic!("`performance.now()` must not throw");
        };
        let Some(ms) = value.as_f64() else {
            panic!("`performance.now()` must return a number");
        };
        ms
    }
}

/// Build the plain fixture with a live scroll origin and, optionally, one
/// oversized row. Unlike `make_scroll_failure_fixture_model` this keeps the
/// real store-backed style/value accessors — nothing here simulates a bridge
/// failure; these probes measure the healthy path only.
fn make_scrollable_fixture_model(
    store: FixtureStore,
    top_row: Rc<Cell<i32>>,
    left_column: Rc<Cell<i32>>,
    tall_row: Option<i32>,
) -> JsValue {
    let model = make_fixture_model(store);
    let obj: js_sys::Object = model.unchecked_into();

    let view_top_row = Rc::clone(&top_row);
    let view_left_column = Rc::clone(&left_column);
    let get_view = Closure::wrap(Box::new(move || -> JsValue {
        let view = js_sys::Object::new();
        set_value_prop(&view, "sheet", &JsValue::from_f64(0.0));
        // The active cell stays inside the viewport across every scroll these
        // probes perform, so `Chrome::classify` keeps approving the blit.
        set_value_prop(&view, "row", &JsValue::from_f64(5.0));
        set_value_prop(&view, "column", &JsValue::from_f64(3.0));
        let range = js_sys::Array::new();
        for value in [5.0, 3.0, 5.0, 3.0] {
            range.push(&JsValue::from_f64(value));
        }
        set_value_prop(&view, "range", &range.into());
        set_value_prop(
            &view,
            "top_row",
            &JsValue::from_f64(f64::from(view_top_row.get())),
        );
        set_value_prop(
            &view,
            "left_column",
            &JsValue::from_f64(f64::from(view_left_column.get())),
        );
        view.into()
    }) as Box<dyn Fn() -> JsValue>);
    set_prop(&obj, "getSelectedView", get_view.as_ref().unchecked_ref());
    get_view.forget();

    // An explicit row-height accessor is required for the blit overlap probe;
    // the Fresh builder's default-height fallback is not used there.
    let get_row_height = Closure::wrap(Box::new(move |_sheet: u32, row: i32| -> f64 {
        if tall_row == Some(row) {
            STAGE6_TALL_ROW_H
        } else {
            STAGE6_ROW_H
        }
    }) as Box<dyn Fn(u32, i32) -> f64>);
    set_prop(
        &obj,
        "getRowHeight",
        get_row_height.as_ref().unchecked_ref(),
    );
    get_row_height.forget();

    // Without this the renderer falls back to `DEFAULT_COL_WIDTH`, and the
    // Stage 6 canvas would show 26 columns instead of the native matrix's 21.
    let get_column_width =
        Closure::wrap(
            Box::new(move |_sheet: u32, _col: i32| -> f64 { STAGE6_COL_W })
                as Box<dyn Fn(u32, i32) -> f64>,
        );
    set_prop(
        &obj,
        "getColumnWidth",
        get_column_width.as_ref().unchecked_ref(),
    );
    get_column_width.forget();

    obj.into()
}

/// Build, size and cold-Fresh-paint one Stage 6 canvas over a caller-supplied
/// store and scroll origin, returning the grid element so raster cases can read
/// its `ImageData`. Both sides of every offset raster comparison come through
/// here, so canvas size, DPR, column width and model wiring can never diverge
/// between the retained-pixel path and its forced-Fresh reference.
fn stage6_canvas_over(
    store: FixtureStore,
    top_row: Rc<Cell<i32>>,
    left_column: Rc<Cell<i32>>,
    tall_row: Option<i32>,
) -> (IronCanvas, HtmlCanvasElement) {
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay) else {
        panic!("create IronCanvas");
    };
    let model = make_scrollable_fixture_model(store, top_row, left_column, tall_row);
    let Ok(()) = canvas.set_model_js(model) else {
        panic!("scrollable fixture model passes the duck test");
    };
    canvas.resize(STAGE6_CANVAS_W, STAGE6_CANVAS_H, STAGE6_DPR);
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    // The cold Fresh covers the whole pane, so it is the cheapest place to
    // prove the geometry before any sample or pixel is taken. A tall row
    // deliberately costs the pane one row, so it is exempt.
    if tall_row.is_none() {
        stage6_assert_full_pane(&canvas.frame_trace(), "cold Fresh");
    }
    (canvas, grid)
}

/// A canvas driven by a scrollable fixture, returned alongside the shared
/// store and the live scroll origin the probes mutate.
fn stage6_scrollable_canvas(tall_row: Option<i32>) -> (IronCanvas, FixtureStore, Rc<Cell<i32>>) {
    let store = stage6_fixture_store();
    let top_row = Rc::new(Cell::new(1));
    let (canvas, _grid) = stage6_canvas_over(
        Rc::clone(&store),
        Rc::clone(&top_row),
        Rc::new(Cell::new(1)),
        tall_row,
    );
    (canvas, store, top_row)
}

/// Move the scroll origin and paint, without timing — the setup half of every
/// measured pair.
fn stage6_scroll_to(canvas: &mut IronCanvas, top_row: &Rc<Cell<i32>>, row: i32) {
    top_row.set(row);
    canvas.view_changed_js();
    canvas.paint_if_dirty();
}

/// Time one `paintIfDirty()` in milliseconds. Asserts the attempt actually
/// painted: a held or idle attempt would contribute a meaningless sample.
fn stage6_timed_paint(canvas: &mut IronCanvas, clock: &PerfClock) -> f64 {
    let start = clock.now_ms();
    let result = canvas.paint_if_dirty();
    let elapsed = clock.now_ms() - start;
    assert_eq!(
        result,
        JsPaintResult::Painted,
        "a timed Stage 6 paint must commit — a held or idle attempt is not a sample"
    );
    elapsed
}

fn stage6_percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let last = sorted.len() - 1;
    let rank = (fraction * last as f64).round() as usize;
    sorted[rank.min(last)]
}

/// `console.log` reached through the global object, the same way `PerfClock`
/// reaches `performance.now` and for the same reason: no `web-sys` feature has
/// to be enabled for a test-only diagnostic. `println!` cannot be used —
/// `stdout` is a discard sink on `wasm32-unknown-unknown`, and the runner's
/// `--nocapture` uncaptures `console.*()` only.
fn stage6_console_line(line: &str) {
    let global = js_sys::global();
    let Ok(console) = js_sys::Reflect::get(&global, &JsValue::from_str("console")) else {
        panic!("the browser test runner must expose a global `console` object");
    };
    let Ok(log) = js_sys::Reflect::get(&console, &JsValue::from_str("log")) else {
        panic!("`console.log` must exist");
    };
    let log: js_sys::Function = log.unchecked_into();
    let Ok(_) = log.call1(&console, &JsValue::from_str(line)) else {
        panic!("`console.log()` must not throw");
    };
}

/// One JSON object per workload phase, on its own line: median, p95 and the
/// raw sample vector, plus the `frameTrace` string of the last measured frame
/// so the report can confirm which path was actually timed.
fn stage6_emit(workload: &str, phase: &str, trace: &str, samples: &[f64]) {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = if sorted.len() % 2 == 1 {
        stage6_percentile(&sorted, 0.5)
    } else {
        f64::midpoint(sorted[sorted.len() / 2 - 1], sorted[sorted.len() / 2])
    };
    let raw: Vec<String> = samples.iter().map(|ms| format!("{ms:.4}")).collect();
    let line = format!(
        "{{\"probe\":\"stage6-browser\",\"workload\":\"{workload}\",\"phase\":\"{phase}\",\
\"n\":{n},\"median_ms\":{median:.4},\"p95_ms\":{p95:.4},\"min_ms\":{min:.4},\"max_ms\":{max:.4},\
\"trace\":\"{trace}\",\"samples_ms\":[{samples_json}]}}",
        n = samples.len(),
        p95 = stage6_percentile(&sorted, 0.95),
        min = sorted.first().copied().unwrap_or(0.0),
        max = sorted.last().copied().unwrap_or(0.0),
        samples_json = raw.join(",")
    );
    stage6_console_line(&line);
}

/// W4 — the qualifying one-axis row scroll, the Viewport strip baseline.
/// Alternates direction so each pair returns the fixture to its origin without
/// rebuilding the canvas.
#[wasm_bindgen_test]
#[ignore = "Stage 6 manual browser timing probe; run with --release --ignored --nocapture"]
fn stage6_perf_w4_row_scroll() {
    let clock = PerfClock::new();
    let (mut canvas, _store, top_row) = stage6_scrollable_canvas(None);

    for _ in 0..STAGE6_WARMUP {
        stage6_scroll_to(&mut canvas, &top_row, 2);
        stage6_scroll_to(&mut canvas, &top_row, 1);
    }

    let mut down = Vec::with_capacity(STAGE6_PAIRS);
    let mut up = Vec::with_capacity(STAGE6_PAIRS);
    for _ in 0..STAGE6_PAIRS {
        top_row.set(2);
        canvas.view_changed_js();
        down.push(stage6_timed_paint(&mut canvas, &clock));
        top_row.set(1);
        canvas.view_changed_js();
        up.push(stage6_timed_paint(&mut canvas, &clock));
    }
    let trace = canvas.frame_trace();

    stage6_emit("W4", "scroll_down", &trace, &down);
    stage6_emit("W4", "scroll_up", &trace, &up);
}

/// A row well inside the Stage 6 pane at every scroll origin these probes use,
/// carrying no border in any fixture — the Damage strip that forces
/// `PreparedFingerprintUpdate::MarkStale` in W5's stale half, and the healing
/// case's damaged row.
const STAGE6_DAMAGE_ROW: i32 = 12;

/// W5's **stale half** — one timed sample of the post-blit content check a user
/// gets when the row blit could *not* carry its fingerprint history forward.
///
/// The staleness is produced by production behaviour, not by a harness switch:
/// a Damage strip commits `PreparedFingerprintUpdate::MarkStale` unconditionally
/// (a repainted band is never proof that a medium/thick stroke stopped bleeding
/// outside it), so the row blit that follows finds `FingerprintTruth::Stale`,
/// `build_row_shift_candidate` refuses to rotate, and the retained tree keeps
/// describing the pre-scroll range. The unchanged-content notification then
/// compares a new-range candidate against an old-range tree, misses, and pays
/// the full five-pass cell walk. That is exactly the frame every post-blit
/// content check cost before Tasks 5-7 — and still costs today whenever a
/// Damage strip precedes the scroll.
fn stage6_w5_stale_half(
    canvas: &mut IronCanvas,
    top_row: &Rc<Cell<i32>>,
    away: i32,
    clock: &PerfClock,
) -> (f64, String) {
    canvas.mark_rows_damaged(0, STAGE6_DAMAGE_ROW, STAGE6_DAMAGE_ROW);
    canvas.paint_if_dirty();
    stage6_scroll_to(canvas, top_row, away);
    canvas.mark_content_dirty();
    let elapsed = stage6_timed_paint(canvas, clock);
    (elapsed, canvas.frame_trace())
}

/// W5's **rotated half** — one timed sample of the same post-blit content check
/// when the row blit *could* carry its history forward: the preceding frames
/// left `FingerprintTruth::Exact`, `build_row_shift_candidate` rotated the
/// overlapping rows into the new range and rebuilt the revealed strip from the
/// values the strip painter was about to consume, and the blit installed that
/// tree. The unchanged-content notification now matches and skips the cell
/// painter.
///
/// Every non-painter phase is identical to the stale half — same SlotsReuse
/// regime, same whole-grid fetch, same candidate build, same shell, headers and
/// presentation — so the paired difference is the five-pass cell walk and
/// nothing else.
fn stage6_w5_rotated_half(
    canvas: &mut IronCanvas,
    top_row: &Rc<Cell<i32>>,
    away: i32,
    clock: &PerfClock,
) -> (f64, String) {
    stage6_scroll_to(canvas, top_row, away);
    canvas.mark_content_dirty();
    let elapsed = stage6_timed_paint(canvas, clock);
    (elapsed, canvas.frame_trace())
}

/// W5 — the phase-attribution control, re-shaped for Task 7.
///
/// Task 2 measured the pair as "post-blit Full, then the Skip its reseed
/// enabled", because before Task 6 a blit *always* left the tree stale and the
/// first content check was always the reseeding Full. Task 6 removed that
/// reseed frame: a qualifying row blit now installs `Exact` history directly,
/// so both halves of the old sequence Skip and there is no within-run Full left
/// to attribute against.
///
/// The pair below restores the control by naming the two states explicitly —
/// rotation unavailable versus rotation applied — and forces the unavailable
/// side with the one production mechanism that legitimately produces it
/// (`Damage` marks history stale). The measured quantity is unchanged: the
/// median paired difference between an identically-fetched SlotsReuse frame
/// that walks the pane and one that does not.
#[wasm_bindgen_test]
#[ignore = "Stage 6 manual browser timing probe; run with --release --ignored --nocapture"]
fn stage6_perf_w5_post_blit_full_then_skip() {
    let clock = PerfClock::new();
    let (mut canvas, _store, top_row) = stage6_scrollable_canvas(None);

    for pair in 0..STAGE6_WARMUP {
        let home = 1 + i32::from(pair % 2 == 1);
        stage6_scroll_to(&mut canvas, &top_row, home);
        let _ = stage6_w5_stale_half(&mut canvas, &top_row, home + 1, &clock);
        stage6_scroll_to(&mut canvas, &top_row, home);
        let _ = stage6_w5_rotated_half(&mut canvas, &top_row, home + 1, &clock);
        stage6_scroll_to(&mut canvas, &top_row, home);
    }

    let mut full = Vec::with_capacity(STAGE6_PAIRS);
    let mut skip = Vec::with_capacity(STAGE6_PAIRS);
    let mut full_trace = String::new();
    let mut skip_trace = String::new();
    for pair in 0..STAGE6_PAIRS {
        // Two independent alternations, at different periods: which half is
        // timed first (so ordering cannot bias one side), and which absolute
        // row range the pair scrolls over (so the shift is a real one every
        // time rather than a no-op re-request of the current origin).
        let stale_first = pair % 2 == 0;
        let home = 1 + i32::from((pair / 2) % 2 == 1);
        let away = home + 1;
        stage6_scroll_to(&mut canvas, &top_row, home);

        let mut run_stale = |canvas: &mut IronCanvas, top_row: &Rc<Cell<i32>>| {
            let (ms, trace) = stage6_w5_stale_half(canvas, top_row, away, &clock);
            full.push(ms);
            full_trace = trace;
        };
        let mut run_rotated = |canvas: &mut IronCanvas, top_row: &Rc<Cell<i32>>| {
            let (ms, trace) = stage6_w5_rotated_half(canvas, top_row, away, &clock);
            skip.push(ms);
            skip_trace = trace;
        };

        if stale_first {
            run_stale(&mut canvas, &top_row);
            stage6_scroll_to(&mut canvas, &top_row, home);
            run_rotated(&mut canvas, &top_row);
        } else {
            run_rotated(&mut canvas, &top_row);
            stage6_scroll_to(&mut canvas, &top_row, home);
            run_stale(&mut canvas, &top_row);
        }
        stage6_scroll_to(&mut canvas, &top_row, home);
    }

    // Gate C is decided on this pair, and the rule names the production shape.
    // Both halves must have fetched the identical full pane, or the delta below
    // is not the quantity the gate asks about.
    stage6_assert_full_pane(&full_trace, "W5 post_blit_full");
    stage6_assert_full_pane(&skip_trace, "W5 post_blit_skip");
    // And they must have reached the two verdicts the pair is named for: a
    // silently-Skipping "full" half would report a saving of zero and a
    // silently-Full "skip" half would report the whole frame.
    assert!(
        full_trace.contains("grid:FULL"),
        "W5's stale half must walk the pane; trace was `{full_trace}`"
    );
    assert!(
        skip_trace.contains("grid:skip"),
        "W5's rotated half must skip the cell painter; trace was `{skip_trace}`"
    );

    stage6_emit("W5", "post_blit_full", &full_trace, &full);
    stage6_emit("W5", "post_blit_skip", &skip_trace, &skip);
}

/// W6 — the real post-scroll edit path: a qualifying row blit followed by a
/// borderless overlapping-row edit. Row 10 carries no border in either
/// direction of the scroll, and its value changes every iteration so the edit
/// is always a genuine content change.
#[wasm_bindgen_test]
#[ignore = "Stage 6 manual browser timing probe; run with --release --ignored --nocapture"]
fn stage6_perf_w6_post_blit_borderless_edit() {
    const EDITED_ROW: i32 = 10;
    const EDITED_COL: i32 = 2;

    let clock = PerfClock::new();
    let (mut canvas, store, top_row) = stage6_scrollable_canvas(None);

    let edit = |generation: usize| {
        let mut cells = store.borrow_mut();
        let Some(cell) = cells.get_mut(&(EDITED_ROW, EDITED_COL)) else {
            panic!("the fixture seeds every cell in range");
        };
        cell.value = format!("edit-{generation}");
    };

    for generation in 0..STAGE6_WARMUP {
        stage6_scroll_to(&mut canvas, &top_row, 2);
        edit(generation);
        canvas.mark_content_dirty();
        canvas.paint_if_dirty();
        stage6_scroll_to(&mut canvas, &top_row, 1);
    }

    let mut samples = Vec::with_capacity(STAGE6_PAIRS);
    let mut trace = String::new();
    for pair in 0..STAGE6_PAIRS {
        let scrolled = if pair % 2 == 0 { 2 } else { 1 };
        let back = if pair % 2 == 0 { 1 } else { 2 };
        stage6_scroll_to(&mut canvas, &top_row, scrolled);

        edit(STAGE6_WARMUP + pair);
        canvas.mark_content_dirty();
        samples.push(stage6_timed_paint(&mut canvas, &clock));
        // Captured here, before the untimed cleanup scroll below can
        // overwrite it with `Viewport ... grid:strip` — the trace must belong
        // to the timed paint it is emitted alongside.
        trace = canvas.frame_trace();

        stage6_scroll_to(&mut canvas, &top_row, back);
    }

    assert!(
        trace.contains("SlotsReuse"),
        "W6's timed paint must be the qualifying row-blit content check; trace was `{trace}`"
    );
    assert!(
        trace.contains("grid:rows"),
        "W6's timed paint must reach the rotated row-only painter path; trace was `{trace}`"
    );

    stage6_emit("W6", "post_blit_edit", &trace, &samples);
}

/// W7 — the `IncompatibleRange` full fallback. Row 1 is taller than the rest,
/// so scrolling it out of view frees enough pixels for one extra row: the
/// scroll-axis extent changes by one and `shift_is_safe` rejects the shift,
/// costing a whole-grid fetch and paint on a frame that planned a strip.
#[wasm_bindgen_test]
#[ignore = "Stage 6 manual browser timing probe; run with --release --ignored --nocapture"]
fn stage6_perf_w7_incompatible_range_scroll() {
    let clock = PerfClock::new();
    let (mut canvas, _store, top_row) = stage6_scrollable_canvas(Some(1));

    for _ in 0..STAGE6_WARMUP {
        stage6_scroll_to(&mut canvas, &top_row, 2);
        stage6_scroll_to(&mut canvas, &top_row, 1);
    }

    let mut samples = Vec::with_capacity(STAGE6_PAIRS);
    for _ in 0..STAGE6_PAIRS {
        top_row.set(2);
        canvas.view_changed_js();
        samples.push(stage6_timed_paint(&mut canvas, &clock));
        stage6_scroll_to(&mut canvas, &top_row, 1);
    }
    let trace = canvas.frame_trace();

    stage6_emit("W7", "edge_row_extent", &trace, &samples);
}

/// W8 — theme change then Fresh. Both directions are timed because the
/// duplicate-invalidation hypothesis applied to either palette. Task 3
/// settled it: `set_theme`'s eager grid invalidation is gone and the healthy
/// Fresh prologue is the sole source of the pair, so this probe now
/// re-measures a one-pair frame.
#[wasm_bindgen_test]
#[ignore = "Stage 6 manual browser timing probe; run with --release --ignored --nocapture"]
fn stage6_perf_w8_theme_change_fresh() {
    let clock = PerfClock::new();
    let (mut canvas, _store, _top_row) = stage6_scrollable_canvas(None);

    for _ in 0..STAGE6_WARMUP {
        canvas.set_theme_name("dark");
        canvas.paint_if_dirty();
        canvas.set_theme_name("light");
        canvas.paint_if_dirty();
    }

    let mut to_dark = Vec::with_capacity(STAGE6_PAIRS);
    let mut to_light = Vec::with_capacity(STAGE6_PAIRS);
    for _ in 0..STAGE6_PAIRS {
        canvas.set_theme_name("dark");
        to_dark.push(stage6_timed_paint(&mut canvas, &clock));
        canvas.set_theme_name("light");
        to_light.push(stage6_timed_paint(&mut canvas, &clock));
    }
    let trace = canvas.frame_trace();

    stage6_emit("W8", "theme_to_dark", &trace, &to_dark);
    stage6_emit("W8", "theme_to_light", &trace, &to_light);
}

// ==============================================================================
// Stage 6, Task 7: offset raster gates for row-axis fingerprint rotation.
//
// Tasks 5 and 6 taught a qualifying row blit to carry its fingerprint history
// across the shift instead of abandoning it. Everything that path saves, it
// saves by NOT repainting pixels — so a native operation log can only ever show
// that fewer draws happened, never that the surviving pixels were right. These
// six cases close that gap the only way it can be closed: drive one canvas
// through the real interaction, then compare its raw Canvas2D `ImageData`
// against a second, independent canvas that paints the SAME final state in one
// forced-Fresh shot.
//
// Unlike the `stage6_perf_*` probes above, these are ordinary browser tests: a
// stale pixel is a correctness failure and belongs in CI, while a millisecond
// is evidence for a human. They share the perf probes' production-shaped
// geometry so a raster failure and a timing number describe the same frame.
// ==============================================================================

/// Every raster case below scrolls by exactly one row, which reveals this row
/// at the bottom of the pane: the pane's last visible row after a `top_row`
/// 1 -> 2 step. The store seeds one row past `STAGE6_ROWS` precisely so this row
/// carries real content.
const STAGE6_REVEALED_ROW: i32 = STAGE6_ROWS + 1;
/// A column inside the pane at both `left_column` origins these cases use, and
/// clear of the pinned active cell at column 3.
const STAGE6_EDIT_COL: i32 = 5;

/// Paint `store` at `(top_row, left_column)` on a fresh canvas in one shot and
/// return its raw grid bytes — the reference every retained-pixel path is
/// measured against. A brand-new `IronCanvas` has no cached buffers, no painted
/// fingerprint tree and no prior pixels, so its first paint is unconditionally
/// a whole-grid Fresh: nothing it produces can be inherited from an earlier
/// frame.
fn stage6_forced_fresh_pixels(store: FixtureStore, top_row: i32, left_column: i32) -> Vec<u8> {
    let (_canvas, grid) = stage6_canvas_over(
        store,
        Rc::new(Cell::new(top_row)),
        Rc::new(Cell::new(left_column)),
        None,
    );
    grid_pixels(&grid)
}

/// Move the horizontal scroll origin and paint — `stage6_scroll_to`'s
/// column-axis twin, used only by the column-blit control.
fn stage6_scroll_columns_to(canvas: &mut IronCanvas, left_column: &Rc<Cell<i32>>, column: i32) {
    left_column.set(column);
    canvas.view_changed_js();
    canvas.paint_if_dirty();
}

/// Build a Stage 6 canvas at the origin every raster case starts from, plus the
/// mutable store and both scroll origins.
fn stage6_raster_canvas(
    store: FixtureStore,
) -> (IronCanvas, HtmlCanvasElement, Rc<Cell<i32>>, Rc<Cell<i32>>) {
    let top_row = Rc::new(Cell::new(1));
    let left_column = Rc::new(Cell::new(1));
    let (canvas, grid) =
        stage6_canvas_over(store, Rc::clone(&top_row), Rc::clone(&left_column), None);
    (canvas, grid, top_row, left_column)
}

fn stage6_set_value(store: &FixtureStore, row: i32, col: i32, value: &str) {
    let mut cells = store.borrow_mut();
    let Some(cell) = cells.get_mut(&(row, col)) else {
        panic!("the Stage 6 fixture seeds every cell in and one step past the pane");
    };
    cell.value = value.to_string();
}

fn stage6_set_left_border(
    store: &FixtureStore,
    row: i32,
    col: i32,
    style: Option<ic::BorderStyle>,
) {
    stage6_set_border(store, row, col, FixtureSide::Left, style);
}

fn stage6_set_border(
    store: &FixtureStore,
    row: i32,
    col: i32,
    side: FixtureSide,
    style: Option<ic::BorderStyle>,
) {
    let mut cells = store.borrow_mut();
    let Some(cell) = cells.get_mut(&(row, col)) else {
        panic!("the Stage 6 fixture seeds every cell in and one step past the pane");
    };
    match side {
        FixtureSide::Top => cell.border_top = style,
        FixtureSide::Right => cell.border_right = style,
        FixtureSide::Bottom => cell.border_bottom = style,
        FixtureSide::Left => cell.border_left = style,
    }
}

fn stage6_set_fill(store: &FixtureStore, row: i32, col: i32, fill: Option<&str>) {
    let mut cells = store.borrow_mut();
    let Some(cell) = cells.get_mut(&(row, col)) else {
        panic!("the Stage 6 fixture seeds every cell in and one step past the pane");
    };
    cell.fill = fill.map(str::to_string);
}

fn stage6_set_wrap(store: &FixtureStore, row: i32, col: i32, wrap: bool) {
    let mut cells = store.borrow_mut();
    let Some(cell) = cells.get_mut(&(row, col)) else {
        panic!("the Stage 6 fixture seeds every cell in and one step past the pane");
    };
    cell.wrap = wrap;
}

/// Every byte offset at which `actual` and `expected` disagree.
fn stage6_pixel_diff(actual: &[u8], expected: &[u8]) -> Vec<usize> {
    actual
        .iter()
        .zip(expected.iter())
        .enumerate()
        .filter(|(_, (got, want))| got != want)
        .map(|(offset, _)| offset)
        .collect()
}

fn stable_forced_fresh_pixels_at(
    store: FixtureStore,
    view: StableViewFixture,
    width: f64,
    height: f64,
    dpr: f64,
) -> (Vec<u8>, Vec<u8>) {
    let (_canvas, grid, overlay) = stable_canvas_over_at(store, view, width, height, dpr);
    (canvas_pixels(&grid), canvas_pixels(&overlay))
}

fn stable_second_paint_seam_at(
    store: FixtureStore,
    view: StableViewFixture,
    fresh_grid: &[u8],
    width: f64,
    height: f64,
    dpr: f64,
) -> Vec<usize> {
    let (mut canvas, grid, _overlay) = stable_canvas_over_at(store, view, width, height, dpr);
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_pixel_diff(&canvas_pixels(&grid), fresh_grid)
}

fn stable_pixel_failure(
    case: &str,
    layer: &str,
    width: usize,
    diff: &[usize],
    actual: &[u8],
    expected: &[u8],
) -> ! {
    let sample: Vec<String> = diff
        .iter()
        .take(8)
        .map(|&offset| {
            let pixel = offset / 4;
            format!(
                "({x},{y})ch{ch} got {got} want {want}",
                x = pixel % width,
                y = pixel / width,
                ch = offset % 4,
                got = actual[offset],
                want = expected[offset],
            )
        })
        .collect();
    panic!(
        "stable repaint case `{case}` left {n} unexpected {layer} byte(s): {sample}",
        n = diff.len(),
        sample = sample.join("; "),
    );
}

fn stable_assert_matches_forced_fresh(
    grid: &HtmlCanvasElement,
    overlay: &HtmlCanvasElement,
    store: &FixtureStore,
    final_view: &StableViewFixture,
    case: &str,
) {
    stable_assert_matches_forced_fresh_at(
        grid,
        overlay,
        store,
        final_view,
        case,
        STAGE6_CANVAS_W,
        STAGE6_CANVAS_H,
        STAGE6_DPR,
    );
}

#[allow(clippy::too_many_arguments)]
fn stable_assert_matches_forced_fresh_at(
    grid: &HtmlCanvasElement,
    overlay: &HtmlCanvasElement,
    store: &FixtureStore,
    final_view: &StableViewFixture,
    case: &str,
    width: f64,
    height: f64,
    dpr: f64,
) {
    let (fresh_grid, fresh_overlay) =
        stable_forced_fresh_pixels_at(Rc::clone(store), final_view.clone(), width, height, dpr);
    let actual_grid = canvas_pixels(grid);
    let actual_overlay = canvas_pixels(overlay);
    assert_eq!(actual_grid.len(), fresh_grid.len());
    assert_eq!(actual_overlay.len(), fresh_overlay.len());

    let seam = stable_second_paint_seam_at(
        Rc::clone(store),
        final_view.clone(),
        fresh_grid.as_slice(),
        width,
        height,
        dpr,
    );
    let unexpected_grid: Vec<usize> = stage6_pixel_diff(&actual_grid, &fresh_grid)
        .into_iter()
        .filter(|offset| !seam.contains(offset))
        .collect();
    if !unexpected_grid.is_empty() {
        stable_pixel_failure(
            case,
            "grid",
            grid.width() as usize,
            &unexpected_grid,
            &actual_grid,
            &fresh_grid,
        );
    }

    let overlay_diff = stage6_pixel_diff(&actual_overlay, &fresh_overlay);
    if !overlay_diff.is_empty() {
        stable_pixel_failure(
            case,
            "overlay",
            overlay.width() as usize,
            &overlay_diff,
            &actual_overlay,
            &fresh_overlay,
        );
    }
}

fn stable_assert_slots_reuse_trace(trace: &str, case: &str) {
    assert!(
        trace.starts_with("SlotsReuse[WorkFlags(VIEW | CONTENT | OVERLAY)]"),
        "stable repaint case `{case}` must carry VIEW | CONTENT | OVERLAY through SlotsReuse; \
         trace was `{trace}`"
    );
}

#[wasm_bindgen_test]
fn cell_repaint_plain_commit_and_selection_move_matches_forced_fresh() {
    const CASE: &str = "stable plain commit and selection move";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_value(&store, 5, 3, "plain-commit");
    view.set_active(6, 3);
    canvas.mark_rows_damaged(0, 5, 5);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let trace = canvas.frame_trace();
    stable_assert_slots_reuse_trace(&trace, CASE);
    stage6_assert_verdict(&trace, "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_clipped_long_text_matches_forced_fresh() {
    const CASE: &str = "clipped long text";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_value(
        &store,
        5,
        3,
        "a deliberately long value that must be clipped to the edited cell",
    );
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_wrapped_text_matches_forced_fresh() {
    const CASE: &str = "wrapped text";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_value(&store, 5, 3, "wrapped text across more than one line");
    stage6_set_wrap(&store, 5, 3, true);
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_fill_change_matches_forced_fresh() {
    const CASE: &str = "fill change";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_fill(&store, 5, 3, Some("#FFCC00"));
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_viewport_top_left_matches_forced_fresh() {
    const CASE: &str = "viewport top-left cell";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_value(&store, 1, 1, "top-left-change");
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_viewport_bottom_right_matches_forced_fresh() {
    const CASE: &str = "viewport bottom-right cell";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_value(&store, STAGE6_ROWS, STAGE6_COLS, "bottom-right-change");
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_multi_row_dependants_match_forced_fresh() {
    const CASE: &str = "stable multi-row dependants";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_value(&store, 5, 3, "edited");
    stage6_set_value(&store, 9, 5, "dependant");
    view.set_active(6, 3);
    canvas.mark_rows_damaged(0, 5, 5);
    canvas.mark_rows_damaged(0, 9, 9);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let trace = canvas.frame_trace();
    stable_assert_slots_reuse_trace(&trace, CASE);
    stage6_assert_verdict(&trace, "grid:range", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_range_crosses_hidden_row_and_column() {
    const CASE: &str = "range across hidden row and column";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(4, 3);
    view.set_row_height(5, 0.0);
    view.set_column_width(4, 0.0);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_value(&store, 4, 3, "before-hidden-slot");
    stage6_set_value(&store, 6, 5, "after-hidden-slot");
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:range", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn stable_unchanged_recalc_skips_grid_and_moves_overlay() {
    const CASE: &str = "stable unchanged recalc";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let trace = canvas.frame_trace();
    stable_assert_slots_reuse_trace(&trace, CASE);
    stage6_assert_verdict(&trace, "grid:skip", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_medium_left_border_removal_matches_fresh() {
    const CASE: &str = "stable explicit border removal";
    let store = stage6_fixture_store();
    stage6_set_left_border(&store, 5, 3, Some(ic::BorderStyle::Medium));
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_left_border(&store, 5, 3, None);
    view.set_active(6, 3);
    canvas.mark_rows_damaged(0, 5, 5);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let trace = canvas.frame_trace();
    stable_assert_slots_reuse_trace(&trace, CASE);
    stage6_assert_verdict(&trace, "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_thick_top_border_addition_matches_fresh() {
    const CASE: &str = "thick top border addition";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_border(&store, 5, 3, FixtureSide::Top, Some(ic::BorderStyle::Thick));
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_thick_bottom_border_addition_matches_fresh() {
    const CASE: &str = "thick bottom border addition";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_border(
        &store,
        5,
        3,
        FixtureSide::Bottom,
        Some(ic::BorderStyle::Thick),
    );
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_double_right_border_removal_matches_fresh() {
    const CASE: &str = "double right border removal";
    let store = stage6_fixture_store();
    stage6_set_border(
        &store,
        5,
        3,
        FixtureSide::Right,
        Some(ic::BorderStyle::Double),
    );
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_border(&store, 5, 3, FixtureSide::Right, None);
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_exposes_neighbour_owned_competing_edge() {
    const CASE: &str = "neighbour-owned competing edge";
    let store = stage6_fixture_store();
    stage6_set_border(
        &store,
        5,
        2,
        FixtureSide::Right,
        Some(ic::BorderStyle::Medium),
    );
    stage6_set_border(
        &store,
        5,
        3,
        FixtureSide::Left,
        Some(ic::BorderStyle::Double),
    );
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_border(&store, 5, 3, FixtureSide::Left, None);
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn cell_repaint_double_border_at_dpr_2_matches_fresh() {
    const CASE: &str = "double border at DPR 2";
    const DPR: f64 = 2.0;
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over_at(
        Rc::clone(&store),
        view.clone(),
        STAGE6_CANVAS_W,
        STAGE6_CANVAS_H,
        DPR,
    );

    stage6_set_border(
        &store,
        5,
        3,
        FixtureSide::Right,
        Some(ic::BorderStyle::Double),
    );
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh_at(
        &grid,
        &overlay,
        &store,
        &view,
        CASE,
        STAGE6_CANVAS_W,
        STAGE6_CANVAS_H,
        DPR,
    );
}

#[wasm_bindgen_test]
fn cell_repaint_medium_border_at_fractional_dpr_matches_fresh() {
    const CASE: &str = "medium border at fractional DPR";
    const DPR: f64 = 1.25;
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over_at(
        Rc::clone(&store),
        view.clone(),
        STAGE6_CANVAS_W,
        STAGE6_CANVAS_H,
        DPR,
    );

    stage6_set_border(
        &store,
        5,
        3,
        FixtureSide::Left,
        Some(ic::BorderStyle::Medium),
    );
    view.set_active(6, 3);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", CASE);

    stable_assert_matches_forced_fresh_at(
        &grid,
        &overlay,
        &store,
        &view,
        CASE,
        STAGE6_CANVAS_W,
        STAGE6_CANVAS_H,
        DPR,
    );
}

#[wasm_bindgen_test]
fn cell_repaint_frozen_grid_commit_uses_one_verdict_and_matches_fresh() {
    const CASE: &str = "stable frozen-grid commit";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 5).with_frozen(2, 2);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_value(&store, 5, 5, "frozen-pane-commit");
    view.set_active(6, 5);
    canvas.mark_rows_damaged(0, 5, 5);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let trace = canvas.frame_trace();
    stable_assert_slots_reuse_trace(&trace, CASE);
    stage6_assert_verdict(&trace, "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

#[wasm_bindgen_test]
fn stable_hidden_selection_commit_matches_fresh() {
    const CASE: &str = "stable hidden-selection commit";
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(5, 3);
    let (mut canvas, grid, overlay) = stable_canvas_over(Rc::clone(&store), view.clone());

    stage6_set_value(&store, 5, 3, "hidden-selection-commit");
    view.set_active(6, 3);
    view.set_show_selection(false);
    canvas.mark_rows_damaged(0, 5, 5);
    canvas.mark_content_dirty();
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let trace = canvas.frame_trace();
    stable_assert_slots_reuse_trace(&trace, CASE);
    stage6_assert_verdict(&trace, "grid:cell", CASE);

    stable_assert_matches_forced_fresh(&grid, &overlay, &store, &view, CASE);
}

/// The byte offsets at which a canvas that has simply painted *twice* differs
/// from one that painted once, over identical content at identical geometry.
///
/// This is not zero, and it is not a Stage 6 defect. Two pixels on the seam row
/// between the column header and the cell area — `(407, 28)` and `(1222, 28)`
/// at this canvas size — read 226 instead of 232 after any second paint,
/// including a plain `RepaintPlan::Skip` that draws nothing at all in the cell
/// area. The header band is repainted every frame while the cell area
/// deliberately is not cleared, so the antialiased tail of a header stroke
/// composites onto its own previous output. It is reproduced here rather than
/// hard-coded because a hard-coded coordinate would silently stop describing
/// the artefact the day the header geometry moves.
///
/// The gate below therefore does not weaken to "close enough": it requires the
/// retained-pixel path to differ from forced Fresh in a SUBSET of exactly these
/// offsets and nowhere else. A stale stroke, an unshifted band or a missed
/// strip row all land outside the set and fail. (A subset rather than an equal
/// set because a conservative whole-grid repaint clears the cell background
/// first, which erases the seam pixel and matches forced Fresh exactly.)
fn stage6_repaint_seam(
    store: FixtureStore,
    top_row: i32,
    left_column: i32,
    fresh: &[u8],
) -> Vec<usize> {
    let (mut canvas, grid) = stage6_canvas_over(
        store,
        Rc::new(Cell::new(top_row)),
        Rc::new(Cell::new(left_column)),
        None,
    );
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(
        &canvas.frame_trace(),
        "grid:skip",
        "repaint-seam control (unchanged content must skip)",
    );
    stage6_pixel_diff(&grid_pixels(&grid), fresh)
}

/// The offset raster gate: `grid`'s current pixels against a forced-Fresh
/// render of `store` at `(top_row, left_column)`.
///
/// `store` is the scenario's own live store, so the reference paints the
/// scenario's FINAL state — the comparison is "did the retained path arrive
/// where a from-scratch paint would have", not "did it change anything".
fn stage6_assert_matches_forced_fresh(
    grid: &HtmlCanvasElement,
    store: &FixtureStore,
    top_row: i32,
    left_column: i32,
    case: &str,
) {
    let fresh = stage6_forced_fresh_pixels(Rc::clone(store), top_row, left_column);
    let actual = grid_pixels(grid);
    assert_eq!(
        actual.len(),
        fresh.len(),
        "Stage 6 raster case `{case}`: both canvases must have identical backing stores"
    );

    let seam = stage6_repaint_seam(Rc::clone(store), top_row, left_column, &fresh);
    let diff = stage6_pixel_diff(&actual, &fresh);
    let stale: Vec<usize> = diff
        .iter()
        .copied()
        .filter(|offset| !seam.contains(offset))
        .collect();
    if stale.is_empty() {
        return;
    }

    // `assert_eq!` on the two ~3.8 MB byte vectors cannot be used to report
    // this: the failure message alone exceeds the browser runner's 10 MB
    // response limit and aborts the whole suite with a transport error instead
    // of naming the case. The comparison is still every byte; only the
    // diagnosis is summarised, in the pixel coordinates a stale-stroke or
    // unshifted-band bug is actually identified by.
    let width = STAGE6_CANVAS_W as usize;
    let sample: Vec<String> = stale
        .iter()
        .take(8)
        .map(|&offset| {
            let pixel = offset / 4;
            format!(
                "({x},{y})ch{ch} got {got} want {want}",
                x = pixel % width,
                y = pixel / width,
                ch = offset % 4,
                got = actual[offset],
                want = fresh[offset],
            )
        })
        .collect();
    panic!(
        "Stage 6 raster case `{case}` left {n} byte(s) that forced Fresh does not have, \
         beyond the {seam_n}-byte header-seam control: {sample}",
        n = stale.len(),
        seam_n = seam.len(),
        sample = sample.join("; "),
    );
}

fn stage6_assert_verdict(trace: &str, expected: &str, case: &str) {
    assert!(
        trace.contains(expected),
        "Stage 6 raster case `{case}` must reach `{expected}`, but its frame trace was `{trace}`"
    );
}

/// Case 1 — a qualifying row blit followed by an unchanged-content
/// notification. Rotation makes this the first frame in the project's history
/// that can answer "nothing changed" *immediately* after a scroll; the pixels
/// it keeps are the ones the blit shifted plus the strip it painted, and they
/// must equal a forced-Fresh render of the post-scroll viewport exactly.
#[wasm_bindgen_test]
fn stage6_post_blit_unchanged_content_skips_and_matches_forced_fresh() {
    let store = stage6_fixture_store();
    let (mut canvas, grid, top_row, _left_column) = stage6_raster_canvas(Rc::clone(&store));

    stage6_scroll_to(&mut canvas, &top_row, 2);
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(
        &canvas.frame_trace(),
        "grid:skip",
        "post-blit unchanged content",
    );

    stage6_assert_matches_forced_fresh(&grid, &store, 2, 1, "post-blit unchanged content");
}

/// Case 2 — a qualifying row blit followed by a borderless overlapping-row
/// edit. Before rotation this frame could only be a range-mismatch `Full`;
/// with rotation the planner can name the one changed row. The narrow band it
/// repaints must reconstruct the same raster the whole-grid path would have.
#[wasm_bindgen_test]
fn stage6_post_blit_borderless_edit_repaints_cell_and_matches_forced_fresh() {
    const EDITED_ROW: i32 = 12;

    let store = stage6_fixture_store();
    let (mut canvas, grid, top_row, _left_column) = stage6_raster_canvas(Rc::clone(&store));

    stage6_scroll_to(&mut canvas, &top_row, 2);
    stage6_set_value(&store, EDITED_ROW, STAGE6_EDIT_COL, "post-blit-edit");
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(
        &canvas.frame_trace(),
        "grid:cell",
        "post-blit borderless edit",
    );

    stage6_assert_matches_forced_fresh(&grid, &store, 2, 1, "post-blit borderless edit");
}

/// Case 3 — the border case the whole truth machinery exists for. A MEDIUM
/// vertical border lives on the row the scroll reveals, so it is painted by the
/// blit's own strip; the next frame removes it. A medium stroke is drawn wider
/// than a thin one and does not stay inside its cell rect, so clearing only the
/// changed row's band would leave a stale stub above it. The planner must
/// therefore include the neighbouring contributor cells in the cell envelope.
/// Whole-canvas byte equality against forced Fresh proves no stub survived
/// anywhere — including the rows the blit merely shifted and never repainted.
#[wasm_bindgen_test]
fn stage6_post_blit_revealed_row_border_removal_is_border_safe_against_forced_fresh() {
    let store = stage6_fixture_store();
    stage6_set_left_border(
        &store,
        STAGE6_REVEALED_ROW,
        STAGE6_EDIT_COL,
        Some(ic::BorderStyle::Medium),
    );
    let (mut canvas, grid, top_row, _left_column) = stage6_raster_canvas(Rc::clone(&store));

    // The blit reveals the bordered row and its strip paints the stroke.
    stage6_scroll_to(&mut canvas, &top_row, 2);

    stage6_set_left_border(&store, STAGE6_REVEALED_ROW, STAGE6_EDIT_COL, None);
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(
        &canvas.frame_trace(),
        "grid:cell",
        "revealed-row medium border removal",
    );

    stage6_assert_matches_forced_fresh(&grid, &store, 2, 1, "revealed-row medium border removal");
}

/// Case 4 — a conditional-format change on a row the blit revealed. The browser
/// host expresses CF as a dxf-MERGED style (see `FixtureCell::fill`), so this
/// removes a CF fill rather than a `CellDecoration`: `JsBackedModel` has no
/// decoration accessor, and inventing one here would test a channel the browser
/// host does not have. The fill covers the whole cell rect, so a missed repaint
/// is a large, unmistakable pixel difference.
#[wasm_bindgen_test]
fn stage6_post_blit_revealed_row_cf_change_matches_forced_fresh() {
    let store = stage6_fixture_store();
    stage6_set_fill(
        &store,
        STAGE6_REVEALED_ROW,
        STAGE6_EDIT_COL,
        Some("#FFCC00"),
    );
    let (mut canvas, grid, top_row, _left_column) = stage6_raster_canvas(Rc::clone(&store));

    stage6_scroll_to(&mut canvas, &top_row, 2);

    stage6_set_fill(&store, STAGE6_REVEALED_ROW, STAGE6_EDIT_COL, None);
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let trace = canvas.frame_trace();
    assert!(
        !trace.contains("grid:skip"),
        "a CF fill that disappeared from a revealed row must repaint something; trace was `{trace}`"
    );

    stage6_assert_matches_forced_fresh(&grid, &store, 2, 1, "revealed-row CF fill removal");
}

/// Case 5 — the healing path, and the same sequence the redesigned W5 stale
/// half times. A Damage strip commits `MarkStale`, so the row blit after it
/// cannot rotate; the retained tree still describes the pre-scroll range, and
/// the following edit therefore takes the conservative whole-grid repaint that
/// reseeds it. This is the case that must stay conservative: the assertion is
/// that it does, and that the frame it produces is raster-correct.
#[wasm_bindgen_test]
fn stage6_damage_then_blit_then_edit_heals_and_matches_forced_fresh() {
    const EDITED_ROW: i32 = 14;

    let store = stage6_fixture_store();
    let (mut canvas, grid, top_row, _left_column) = stage6_raster_canvas(Rc::clone(&store));

    canvas.mark_rows_damaged(0, STAGE6_DAMAGE_ROW, STAGE6_DAMAGE_ROW);
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:strip", "Damage strip");

    stage6_scroll_to(&mut canvas, &top_row, 2);

    stage6_set_value(&store, EDITED_ROW, STAGE6_EDIT_COL, "healed");
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(
        &canvas.frame_trace(),
        "grid:FULL",
        "Damage-then-blit-then-edit",
    );

    stage6_assert_matches_forced_fresh(&grid, &store, 2, 1, "Damage-then-blit-then-edit healing");
}

/// Case 6 — the column-axis control. Stage 6 limits rotation to the row axis,
/// so a column blit must still mark history stale and leave the next content
/// frame conservative. Asserting the conservative verdict here is what would
/// catch a future rotation quietly widening to the column axis without its own
/// geometry and raster design.
#[wasm_bindgen_test]
fn stage6_column_blit_stays_conservative_and_matches_forced_fresh() {
    const EDITED_ROW: i32 = 12;

    let store = stage6_fixture_store();
    let (mut canvas, grid, _top_row, left_column) = stage6_raster_canvas(Rc::clone(&store));

    stage6_scroll_columns_to(&mut canvas, &left_column, 2);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:strip", "column blit");

    stage6_set_value(&store, EDITED_ROW, STAGE6_EDIT_COL, "post-column-blit-edit");
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:FULL", "post-column-blit edit");

    stage6_assert_matches_forced_fresh(&grid, &store, 1, 2, "post-column-blit edit");
}

// ==============================================================================
// Stage 6, Task 5: dev-diagnostics wire smoke.
//
// The native wire-shape contract is pinned by
// `crates/iron-canvas-web/src/wire.rs`'s `frame_diagnostics_wire_matches_declared_shape`
// (host target, `--features dev-tools`). This browser test proves the facade
// end to end: enabled capture publishes a snapshot object the browser mirrors
// can parse, and disabled capture returns `undefined`.
// ==============================================================================

/// Dev-diagnostics wire smoke: enabled capture returns a snapshot object
/// with the attempt fields; disabled capture returns `undefined`.
#[cfg(feature = "dev-tools")]
#[wasm_bindgen_test]
fn stage6_frame_diagnostics_wire_smoke() {
    let store = stage6_fixture_store();
    let top_row = Rc::new(Cell::new(1));
    let left_column = Rc::new(Cell::new(1));
    let (mut canvas, _grid) = stage6_canvas_over(store, top_row, left_column, None);

    assert!(canvas.frame_diagnostics().is_undefined());

    canvas.set_frame_diagnostics_enabled(true);
    // The cold Fresh already ran before this function returned; force a
    // new attempt so an enabled capture actually publishes.
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let value = canvas.frame_diagnostics();
    assert!(!value.is_undefined(), "enabled capture must publish");

    let diag: DiagWireMirror = serde_wasm_bindgen::from_value(value).expect("snapshot parses");
    assert_eq!(diag.schema_version, 2);
    assert_eq!(diag.attempt_seq, 2);
    assert!(matches!(diag.outcome, FrameOutcomeMirror::Painted));
    let geo = diag
        .geometry
        .as_ref()
        .expect("grid-visited attempt has geometry");
    assert_eq!(geo.segments.len(), 1);
    // cssSize is the CSS size the grid planned against; backingSize is the
    // ACTUAL grid canvas backing store (STAGE6_DPR == 1.0, so they agree).
    assert_eq!(
        geo.css_size,
        SizeScenario {
            w: STAGE6_CANVAS_W,
            h: STAGE6_CANVAS_H
        }
    );
    assert_eq!(
        geo.backing_size,
        SizeScenario {
            w: STAGE6_CANVAS_W,
            h: STAGE6_CANVAS_H
        }
    );

    canvas.set_frame_diagnostics_enabled(false);
    assert!(canvas.frame_diagnostics().is_undefined());
}

#[cfg(feature = "dev-tools")]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagWireMirror {
    schema_version: u8,
    attempt_seq: u64,
    outcome: FrameOutcomeMirror,
    geometry: Option<DiagGeometryMirror>,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)] // mirror carries the full wire shape; the smoke asserts a subset
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum FrameOutcomeMirror {
    Painted,
    HeldOnBridgeFailure,
    HeldOnInputFailure { input: String },
}

#[cfg(feature = "dev-tools")]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagGeometryMirror {
    css_size: SizeScenario,
    backing_size: SizeScenario,
    segments: Vec<DiagSegmentMirror>,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)] // mirror carries the full wire shape; the smoke asserts a subset
#[derive(serde::Deserialize)]
struct DiagSegmentMirror {
    region: String,
    cells: usize,
}

// ==============================================================================
// Stage 6, Task 7: dev-diagnostics browser scenarios.
//
// These drive the structured capture pipeline end to end through the web
// facade. The assertions are deterministic, not adjust-to-observed: a freeze
// toggle rebuilds Fresh with a named reason and exact segment accounting;
// isolated edits land in exactly the probed segment and either skip with a
// named fingerprint reason or repaint with changed rows intersecting the
// probe; deep scrolls expose exact blit geometry. Raster truth stays under
// the retained-pixel gates above — the snapshot explains, those prove.
// ==============================================================================

// Dev-diagnostics wire mirrors for scenario assertions. Field names are
// pinned by the native wire conversion test in iron-canvas-web/src/wire.rs;
// keep this mirror in exact correspondence with it.

#[cfg(feature = "dev-tools")]
#[allow(dead_code)] // mirrors carry the full wire shape; each scenario asserts a subset
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagScenario {
    schema_version: u8,
    attempt_seq: u64,
    rebuild_reason: Option<String>,
    outcome: FrameOutcomeMirror,
    probe: Option<RcRangeScenario>,
    probe_segments: Vec<String>,
    geometry: Option<DiagGeometryScenario>,
    fetch: DiagFetchScenario,
    repaint: DiagRepaintScenario,
    cache: DiagCacheScenario,
    blit: Option<DiagBlitScenario>,
    paint_counts: DiagPaintCountsScenario,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagGeometryScenario {
    css_size: SizeScenario,
    backing_size: SizeScenario,
    dpr: f64,
    top_row: i32,
    left_column: i32,
    frozen_rows: i32,
    frozen_cols: i32,
    segments: Vec<DiagSegmentScenario>,
}

#[cfg(feature = "dev-tools")]
#[derive(serde::Deserialize, Clone, Debug, PartialEq)]
struct SizeScenario {
    w: f64,
    h: f64,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagSegmentScenario {
    region: String,
    range: RcRangeScenario,
    cells: usize,
}

#[cfg(feature = "dev-tools")]
#[derive(serde::Deserialize, Clone, Debug, PartialEq)]
struct RcRangeScenario {
    r1: i32,
    c1: i32,
    r2: i32,
    c2: i32,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagFetchScenario {
    batches: usize,
    addressed_cells: usize,
    logical_slots: usize,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)] // mirror carries every schema-v2 repaint field
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagRepaintScenario {
    verdict: Option<VerdictScenario>,
    reason: Option<String>,
    changed_rows: Vec<RowSpanScenario>,
    changed_cells: Vec<ChangedCellScenario>,
    clip: Option<RectScenario>,
    source_ranges: Vec<SourceRangeScenario>,
}

#[cfg(feature = "dev-tools")]
#[derive(serde::Deserialize)]
struct DiagPaintCountsScenario {
    rows: usize,
    cells: usize,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)]
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum VerdictScenario {
    Skip,
    Cell,
    Range,
    Rows { spans: u8, rows: u16 },
    Full,
    Strip,
    Held,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)] // retained for Rows verdict diagnostics
#[derive(serde::Deserialize)]
struct RowSpanScenario {
    r1: i32,
    r2: i32,
}

#[cfg(feature = "dev-tools")]
#[derive(serde::Deserialize, Clone, Debug, PartialEq)]
struct ChangedCellScenario {
    row: i32,
    column: i32,
}

#[cfg(feature = "dev-tools")]
#[derive(serde::Deserialize, Clone, Debug, PartialEq)]
struct SourceRangeScenario {
    region: String,
    range: RcRangeScenario,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagCacheScenario {
    resolution: String,
    #[serde(rename = "committedBefore")]
    committed_before: Option<TruthScenario>,
    #[serde(rename = "committedAfter")]
    committed_after: TruthScenario,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TruthScenario {
    layout: Option<serde_json::Value>,
    buffer_truth: String,
    fingerprint_truth: String,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagBlitScenario {
    axis: String,
    delta: i32,
    src: RectScenario,
    dst: RectScenario,
    // `null` for held and fallback blits — a shift is the only outcome
    // whose execution reaches `push_clip`.
    clip: Option<RectScenario>,
    strip: RectScenario,
    result: String,
    cold_cache: Option<bool>,
    revealed: Vec<DiagRevealedScenario>,
}

#[cfg(feature = "dev-tools")]
#[derive(serde::Deserialize)]
struct RectScenario {
    width: f64,
    height: f64,
}

#[cfg(feature = "dev-tools")]
#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagRevealedScenario {
    region: String,
    range: RcRangeScenario,
}

#[cfg(feature = "dev-tools")]
fn diag_snapshot(canvas: &IronCanvas) -> DiagScenario {
    let value = canvas.frame_diagnostics();
    assert!(
        !value.is_undefined(),
        "frameDiagnostics must publish while enabled"
    );
    serde_wasm_bindgen::from_value(value).expect("snapshot parses")
}

#[cfg(feature = "dev-tools")]
/// `stable_canvas_over` with structured diagnostics enabled BEFORE the
/// cold Fresh paint, so the first published snapshot exists on return.
/// Freeze and scroll controls come from the caller's `StableViewFixture`.
fn stable_diag_canvas_over(
    store: FixtureStore,
    view: StableViewFixture,
) -> (IronCanvas, HtmlCanvasElement, HtmlCanvasElement) {
    stable_diag_canvas_over_at(store, view, STAGE6_CANVAS_W, STAGE6_CANVAS_H, STAGE6_DPR)
}

#[cfg(feature = "dev-tools")]
fn stable_diag_canvas_over_at(
    store: FixtureStore,
    view: StableViewFixture,
    width: f64,
    height: f64,
    dpr: f64,
) -> (IronCanvas, HtmlCanvasElement, HtmlCanvasElement) {
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay.clone()) else {
        panic!("create stable-view IronCanvas");
    };
    let Ok(content) = JsBackedModel::try_from_js_value(make_fixture_model(store)) else {
        panic!("stable-view fixture content model passes the duck test");
    };
    canvas.set_model(Rc::new(StableFixtureModel { content, view }));
    canvas.resize(width, height, dpr);
    canvas.set_frame_diagnostics_enabled(true);
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    (canvas, grid, overlay)
}

/// The B3 freeze-toggle observation: a freeze change is a `Fresh` rebuild
/// with `RebuildReason::Freeze`, and the snapshot must attribute the
/// addressed-cell count to exact before/after segments.
#[cfg(feature = "dev-tools")]
#[wasm_bindgen_test]
fn stage6_diag_freeze_toggle_explains_segments() {
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(1, 1);
    let (mut canvas, _grid, _overlay) = stable_diag_canvas_over(store, view.clone());

    // Baseline: one unfrozen BottomRight segment.
    let before = diag_snapshot(&canvas);
    assert_eq!(before.geometry.as_ref().unwrap().segments.len(), 1);

    // Activate a 2x1 freeze: geometry work forces Fresh with reason Freeze.
    view.frozen_rows.set(2);
    view.frozen_cols.set(1);
    canvas.request_repaint();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);

    let after = diag_snapshot(&canvas);
    assert_eq!(after.rebuild_reason.as_deref(), Some("freeze"));
    let geo = after.geometry.as_ref().unwrap();
    assert_eq!(geo.frozen_rows, 2);
    assert_eq!(geo.frozen_cols, 1);
    assert_eq!(geo.segments.len(), 4);
    // Every addressed cell the fetch charged is inside exactly one segment.
    let cells: usize = geo.segments.iter().map(|s| s.cells).sum();
    assert_eq!(cells, after.fetch.addressed_cells);
    // And the trace line's `fetched=` is exactly 4x the addressed cells.
    assert_eq!(after.fetch.logical_slots, 4 * cells);
    // A rebuild's Full verdict must NOT fabricate a fingerprint reason.
    assert_eq!(after.repaint.reason, None);

    // Deactivate: back to one segment, still Fresh/Freeze.
    view.frozen_rows.set(0);
    view.frozen_cols.set(0);
    canvas.request_repaint();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let off = diag_snapshot(&canvas);
    assert_eq!(off.rebuild_reason.as_deref(), Some("freeze"));
    assert_eq!(off.geometry.as_ref().unwrap().segments.len(), 1);
}

/// One edit per real segment, attributed by the probe address: the probe
/// must land in exactly the intended segment, an identical-value edit must
/// `Skip` with `fingerprintsEqual`, and a real change must repaint with
/// exact changed-cell evidence and a concrete repaint envelope.
#[cfg(feature = "dev-tools")]
#[wasm_bindgen_test]
fn stage6_diag_isolated_edits_attribute_segments_and_skips() {
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(1, 1).with_frozen(2, 1);
    let (mut canvas, _grid, _overlay) = stable_diag_canvas_over(store.clone(), view.clone());

    let snapshot = diag_snapshot(&canvas);
    let geo = snapshot.geometry.as_ref().unwrap();
    assert_eq!(geo.segments.len(), 4);
    let cells: usize = geo.segments.iter().map(|s| s.cells).sum();
    assert_eq!(cells, snapshot.fetch.addressed_cells);

    for (region, row, col) in [
        ("topLeft", 1, 1),
        ("topRight", 1, 4),
        ("bottomLeft", 5, 1),
        ("bottomRight", 5, 4),
    ] {
        // Identical-value edit: the fixture seeds `r{row}c{col}`, so
        // writing the same string back must compare equal and skip.
        canvas.set_frame_diagnostics_probe(row, col, row, col);
        stage6_set_value(&store, row, col, &format!("r{row}c{col}"));
        canvas.mark_content_dirty();
        assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
        let diag = diag_snapshot(&canvas);
        assert_eq!(
            diag.probe,
            Some(RcRangeScenario {
                r1: row,
                c1: col,
                r2: row,
                c2: col
            })
        );
        assert_eq!(
            diag.probe_segments,
            vec![region.to_string()],
            "the probe must belong to exactly the intended segment"
        );
        assert!(
            matches!(diag.repaint.verdict, Some(VerdictScenario::Skip)),
            "identical-value edit in {region} must skip; got {:?}",
            diag.repaint.verdict
        );
        assert_eq!(
            diag.repaint.reason.as_deref(),
            Some("fingerprintsEqual"),
            "a skip must name its reason"
        );

        // Real value change: repaint must report the exact changed cell and
        // applied envelope, while probe attribution stays exact.
        canvas.set_frame_diagnostics_probe(row, col, row, col);
        stage6_set_value(&store, row, col, &format!("{region}-changed"));
        canvas.mark_content_dirty();
        assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
        let diag = diag_snapshot(&canvas);
        assert_eq!(diag.probe_segments, vec![region.to_string()]);
        assert!(matches!(diag.repaint.verdict, Some(VerdictScenario::Cell)));
        assert_eq!(diag.repaint.reason.as_deref(), Some("changedCell"));
        assert_eq!(
            diag.repaint.changed_cells,
            vec![ChangedCellScenario { row, column: col }]
        );
        let clip = diag.repaint.clip.as_ref().expect("cell repaint has a clip");
        assert!(
            clip.width > 0.0 && clip.height > 0.0,
            "cell repaint clip has area"
        );
        assert!(
            diag.repaint
                .source_ranges
                .iter()
                .any(|source| source.region == region
                    && source.range.r1 <= row
                    && row <= source.range.r2
                    && source.range.c1 <= col
                    && col <= source.range.c2),
            "source ranges must contain the changed cell in {region}"
        );
    }
}

/// The supplied 30 by 18 live-debug case keeps its whole-grid bridge fetch but
/// paints only the bounded contributor envelope around the edited cell.
#[cfg(feature = "dev-tools")]
#[wasm_bindgen_test]
fn cell_repaint_diag_30_by_18_keeps_fetch_and_reduces_paint() {
    const ROWS: i32 = 30;
    const COLS: i32 = 18;
    const CANVAS_W: f64 = 1_390.0;
    const CANVAS_H: f64 = 608.0;
    const ADDRESSED: usize = (ROWS * COLS) as usize;

    let store = stage6_fixture_store();
    let view = StableViewFixture::new(15, 9);
    let (mut canvas, grid, overlay) =
        stable_diag_canvas_over_at(Rc::clone(&store), view.clone(), CANVAS_W, CANVAS_H, 1.0);
    let baseline = diag_snapshot(&canvas);
    assert_eq!(baseline.fetch.addressed_cells, ADDRESSED);
    assert_eq!(baseline.fetch.logical_slots, 4 * ADDRESSED);

    stage6_set_border(
        &store,
        15,
        9,
        FixtureSide::Right,
        Some(ic::BorderStyle::Medium),
    );
    stage6_set_value(&store, 15, 9, "edited");
    canvas.set_frame_diagnostics_probe(15, 9, 15, 9);
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);

    let diag = diag_snapshot(&canvas);
    assert!(matches!(diag.repaint.verdict, Some(VerdictScenario::Cell)));
    assert_eq!(diag.repaint.reason.as_deref(), Some("changedCell"));
    assert_eq!(
        diag.repaint.changed_cells,
        vec![ChangedCellScenario { row: 15, column: 9 }]
    );
    assert_eq!(diag.fetch.addressed_cells, ADDRESSED);
    assert_eq!(diag.fetch.logical_slots, 4 * ADDRESSED);
    assert!(diag.repaint.clip.is_some());
    assert!(!diag.repaint.source_ranges.is_empty());
    assert_eq!(diag.paint_counts.rows, 3);
    assert_eq!(diag.paint_counts.cells, 9);
    stage6_assert_verdict(&canvas.frame_trace(), "grid:cell", "30 by 18 edit");

    stable_assert_matches_forced_fresh_at(
        &grid,
        &overlay,
        &store,
        &view,
        "30 by 18 edit",
        CANVAS_W,
        CANVAS_H,
        1.0,
    );
}

/// Deep row and column scrolls must expose exact blit geometry: axis,
/// logical delta, effective clip, revealed strips, and a named result.
#[cfg(feature = "dev-tools")]
#[wasm_bindgen_test]
fn stage6_diag_deep_scrolls_expose_blit_clips() {
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(1, 1);
    let (mut canvas, _grid, _overlay) = stable_diag_canvas_over(store, view.clone());

    // Row scroll: origin 1 -> 12, a qualifying single-axis shift.
    stage6_scroll_to(&mut canvas, &view.top_row, 12);
    let row_blit = diag_snapshot(&canvas).blit.expect("row scroll blits");
    assert_eq!(row_blit.axis, "row");
    assert_eq!(row_blit.delta, 11);
    assert_eq!(row_blit.result, "shifted");
    assert!(row_blit.cold_cache.is_none());
    // The revealed address band is the repaint band the renderer actually
    // prepared: `revealed_strip` carries the boundary-overlap row and
    // `widen_to_pixel_clip` may add one partial row. It must therefore
    // cover AT LEAST the logical delta; the exact shift is pinned in
    // pixels below.
    let revealed_rows: i32 = row_blit
        .revealed
        .iter()
        .map(|s| s.range.r2 - s.range.r1 + 1)
        .sum();
    assert!(
        revealed_rows >= row_blit.delta,
        "revealed band must cover at least the logical delta"
    );
    // Exact shift in pixels: strip height == delta rows x fixed row height.
    assert_eq!(
        row_blit.strip.height,
        f64::from(row_blit.delta) * STAGE6_ROW_H
    );
    // Effective clip equals the repaint band (finalized blit work hands
    // plan.pixel_strip to push_clip): the shifted arm reached push_clip,
    // so the wire must carry the exact rectangle, not null.
    let row_clip = row_blit.clip.as_ref().expect("shifted blit applies a clip");
    assert_eq!(row_clip.width, row_blit.strip.width);
    assert_eq!(row_clip.height, row_blit.strip.height);
    assert!(row_clip.width > 0.0 && row_clip.height > 0.0);
    assert_ne!(row_blit.src.width, 0.0);

    // Column scroll: origin 1 -> 8.
    view.left_column.set(8);
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let col_blit = diag_snapshot(&canvas).blit.expect("column scroll blits");
    assert_eq!(col_blit.axis, "column");
    assert_eq!(col_blit.delta, 7);
    assert_eq!(col_blit.result, "shifted");
    let revealed_cols: i32 = col_blit
        .revealed
        .iter()
        .map(|s| s.range.c2 - s.range.c1 + 1)
        .sum();
    assert!(
        revealed_cols >= col_blit.delta,
        "revealed band must cover at least the logical delta"
    );
    assert_eq!(
        col_blit.strip.width,
        f64::from(col_blit.delta) * STAGE6_COL_W
    );
}
