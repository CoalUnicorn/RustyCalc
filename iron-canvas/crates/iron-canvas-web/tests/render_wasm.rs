#![cfg(target_arch = "wasm32")]
//! Browser-only proof that fractional DPR reaches the real `<canvas>`
//! backing store through the `IronCanvas` facade. Regression test for a
//! bug where `resize()` rounded `dpr` before forwarding it to
//! `WebSurface::resize`, silently mapping e.g. 1.25 -> 1 and 1.5 -> 2.

use iron_canvas_web::{CanvasSize, IronCanvas, JsPaintResult};
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

use iron_canvas_web::CanvasModel;
use iron_canvas_web::wasm::JsBackedModel;
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
    let Ok(()) = canvas.setModel(make_fixture_model(store)) else {
        panic!("fixture model passes the duck test");
    };
    canvas.resize(FIXTURE_CANVAS_W, FIXTURE_CANVAS_H, FIXTURE_DPR);
    (canvas, grid)
}

/// Raw RGBA backing-store bytes for `canvas`'s current pixels, read
/// through a `CanvasRenderingContext2d` on the same element `IronCanvas`
/// painted into (`getContext("2d")` is idempotent — this doesn't disturb
/// the live painter's own handle on the same canvas).
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
    let Ok(()) = canvas.setModel(model) else {
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
    let Ok(()) = fresh_canvas.setModel(fresh_model) else {
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
/// own bulk per-pane prepare directly — not `FrameInputs::capture` (already
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
    let Ok(()) = canvas.setModel(model) else {
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
    let Ok(()) = fresh_canvas.setModel(make_fixture_model(plain_fixture_store())) else {
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
            cell.border_bottom = true;
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
        cell.border_bottom = true;
    }
    let (mut canvas, grid) = canvas_over(Rc::clone(&store));
    canvas.paint_if_dirty(); // baseline (row 10 has a bottom border)

    {
        let mut cells = store.borrow_mut();
        let Some(cell) = cells.get_mut(&(10, 1)) else {
            panic!("seeded");
        };
        cell.border_bottom = false;
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
    let Ok(()) = canvas.setModel(make_fixture_model(plain_fixture_store())) else {
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
    let Ok(()) = fresh_canvas.setModel(make_fixture_model(plain_fixture_store())) else {
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
    let Ok(()) = canvas.setModel(make_fixture_model(plain_fixture_store())) else {
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
    let Ok(()) = fresh_canvas.setModel(make_fixture_model(plain_fixture_store())) else {
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
    let Ok(()) = canvas.setModel(make_active_sheet_fixture_model(Rc::clone(&active_sheet))) else {
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
    let Ok(()) = canvas.setModel(make_sheet_throws_once_fixture_model(plain_fixture_store()))
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
