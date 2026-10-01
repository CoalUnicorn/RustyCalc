use super::*;
use crate::coord::CellAddress;
use crate::events::{EventBus, NavigationEvent, SpreadsheetEvent, StructureEvent};
use crate::model::{EvaluationMode, mutate};
use crate::state::ModelStore;
use iron_canvas_web::{IronCanvas, RenderResult};
use ironcalc_base::UserModel;
use ironcalc_base::expressions::types::Area;
use ironcalc_base::types::Link;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test;

use super::super::adapter::WorksheetModelAdapter;

fn canvas(
    model: ModelStore,
    state: WorkbookState,
    dpr: f64,
) -> (
    IronCanvas,
    web_sys::HtmlCanvasElement,
    web_sys::HtmlCanvasElement,
) {
    let element = || {
        document()
            .create_element("canvas")
            .expect("create canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("canvas element")
    };
    let (grid, overlay) = (element(), element());
    let mut canvas = IronCanvas::create(grid.clone(), overlay.clone()).expect("create renderer");
    canvas.resize(400.0, 300.0, dpr).expect("valid metrics");
    canvas.set_model(Rc::new(WorksheetModelAdapter {
        store: model,
        show_headers: state.show_headers,
    }));
    (canvas, grid, overlay)
}

fn pixels(canvas: &web_sys::HtmlCanvasElement) -> Vec<u8> {
    canvas
        .get_context("2d")
        .expect("canvas context")
        .expect("2d context")
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .expect("Canvas2D context")
        .get_image_data(0.0, 0.0, canvas.width() as f64, canvas.height() as f64)
        .expect("read pixels")
        .data()
        .0
}

async fn flush() {
    for _ in 0..4 {
        leptos::task::tick().await;
    }
}

#[wasm_bindgen_test]
async fn metadata_events_refresh_cached_links_and_merges_like_fresh() {
    let _runtime = leptos::mount::mount_to(
        document()
            .create_element("div")
            .expect("executor host")
            .dyn_into::<web_sys::HtmlElement>()
            .expect("HTML element"),
        || (),
    );
    for dpr in [1.0, 1.25] {
        let owner = Owner::new();
        let (model, state, handle, grid, overlay) = owner.with(|| {
            let state = WorkbookState::new(EventBus::new());
            let model = StoredValue::new_local(
                UserModel::new_empty("Sheet1", "en", "UTC", "en").expect("empty workbook"),
            );
            mutate(model, EvaluationMode::Immediate, |m| {
                m.set_user_input(0, 2, 2, "wrapped\nlink")
                    .expect("cell label");
                m.set_frozen_rows_count(0, 2).expect("freeze rows");
                m.set_frozen_columns_count(0, 2).expect("freeze columns");
                m.set_cell_link(
                    0,
                    2,
                    2,
                    Link::Internal {
                        location: "A10".into(),
                        tooltip: None,
                    },
                    None,
                )
                .expect("initial link");
            });
            let (mut canvas, grid, overlay) = canvas(model, state, dpr);
            canvas.set_metadata_epoch(Some(state.events.metadata_seq.get_value()));
            assert_eq!(canvas.render_pending(), RenderResult::Rendered);
            let handle = StoredValue::new_local(Some(canvas));
            install_subscribe_effect(
                state,
                handle,
                StoredValue::new(false),
                Memo::new(|_| OverlayTuple {
                    extend_to: None,
                    point_range: None,
                    formula_refs: vec![],
                }),
                StoredValue::new_local(None),
                || {},
            );
            (model, state, handle, grid, overlay)
        });
        flush().await;

        mutate(model, EvaluationMode::Immediate, |m| {
            m.set_cell_link(
                0,
                2,
                2,
                Link::Internal {
                    location: "A20".into(),
                    tooltip: None,
                },
                None,
            )
            .expect("changed link");
            m.merge_cells(&Area {
                sheet: 0,
                row: 2,
                column: 2,
                width: 2,
                height: 2,
            })
            .expect("merge across frozen boundaries");
        });
        state.emit_events([
            SpreadsheetEvent::Content(ContentEvent::NamedRangesChanged),
            SpreadsheetEvent::Structure(StructureEvent::MergedCellsChanged { sheet: 0 }),
        ]);
        flush().await;
        handle.update_value(|slot| {
            let canvas = slot.as_mut().expect("live canvas");
            assert_eq!(canvas.render_pending(), RenderResult::Rendered);
            assert_eq!(
                canvas
                    .link_at(2, 2)
                    .expect("changed link")
                    .target()
                    .as_str(),
                "A20"
            );
            let point = canvas.cell_rect(3, 3).expect("covered cell").center();
            assert_eq!(
                canvas
                    .display_cell_at(f64::from(point.x), f64::from(point.y))
                    .expect("committed merge")
                    .merged,
                RCRange {
                    r1: 2,
                    c1: 2,
                    r2: 3,
                    c2: 3
                }
            );
        });

        model.update_value(|m| m.set_selected_cell(4, 4).expect("move selection"));
        state.emit_event(SpreadsheetEvent::Navigation(
            NavigationEvent::SelectionChanged {
                address: CellAddress {
                    sheet: 0,
                    row: 4,
                    column: 4,
                },
            },
        ));
        flush().await;
        handle.update_value(|slot| {
            assert_eq!(
                slot.as_mut().expect("live canvas").render_pending(),
                RenderResult::Rendered
            );
        });
        let (mut fresh, fresh_grid, fresh_overlay) = owner.with(|| canvas(model, state, dpr));
        assert_eq!(fresh.render_pending(), RenderResult::Rendered);
        assert!(pixels(&grid) == pixels(&fresh_grid), "grid at DPR {dpr}");
        assert!(
            pixels(&overlay) == pixels(&fresh_overlay),
            "overlay at DPR {dpr}"
        );
    }
}
