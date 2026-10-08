use std::cell::RefCell;
use std::rc::Rc;

use crate::components::workbook::worksheet::Worksheet;
use crate::coord::CellAddress;
use crate::events::{EventBus, FormatEvent, NavigationEvent, SpreadsheetEvent, StructureEvent};
use crate::input::mouse::{CanvasHandle, with_canvas};
use crate::model::AppClipboard;
use crate::scene::SceneHandle;
use crate::state::{ModelStore, WorkbookState};
use ironcalc_base::UserModel;
use leptos::mount::{UnmountHandle, mount_to};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

struct MountedWorksheet {
    model: ModelStore,
    state: WorkbookState,
    canvas: CanvasHandle,
    _unmount: Box<dyn std::any::Any>,
}

fn empty_model() -> UserModel<'static> {
    UserModel::new_empty("Sheet1", "en", "UTC", "en").expect("create test workbook")
}

fn mount_worksheet(model: UserModel<'static>) -> MountedWorksheet {
    let document = window().document().expect("document");
    let host = document
        .create_element("div")
        .expect("host element")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("host div");
    document
        .body()
        .expect("document body")
        .append_child(&host)
        .expect("attach host");

    let captured = Rc::new(RefCell::new(None));
    let captured_for_mount = Rc::clone(&captured);
    let unmount: UnmountHandle<_> = mount_to(host, move || {
        let state = WorkbookState::new(EventBus::new());
        let model = StoredValue::new_local(model);
        let canvas: CanvasHandle = StoredValue::new_local(None::<SceneHandle>);
        let clipboard: StoredValue<Option<AppClipboard>, LocalStorage> =
            StoredValue::new_local(None);

        provide_context(state);
        provide_context(model);
        provide_context(canvas);
        provide_context(clipboard);
        *captured_for_mount.borrow_mut() = Some((model, state, canvas));

        view! { <Worksheet /> }
    });
    let (model, state, canvas) = captured.borrow_mut().take().expect("worksheet context");

    MountedWorksheet {
        model,
        state,
        canvas,
        _unmount: Box::new(unmount),
    }
}

async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let win = window();
        let _ = win.request_animation_frame(&resolve);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

async fn wait_for_frame(fixture: &MountedWorksheet, sheet: u32, workbook_id: u64) {
    for _ in 0..6 {
        next_frame().await;
        let ready = fixture.canvas.with_value(|slot| {
            slot.as_ref()
                .is_some_and(|scene| scene.matches_context(sheet, workbook_id))
        });
        if ready {
            return;
        }
    }
    panic!("worksheet frame did not commit for sheet {sheet}, workbook {workbook_id}");
}

async fn wait_for_commit(fixture: &MountedWorksheet, previous: u64) {
    for _ in 0..6 {
        next_frame().await;
        if fixture.state.committed_frame.get_untracked() != previous {
            return;
        }
    }
    panic!("worksheet did not commit after the model event");
}

#[wasm_bindgen_test]
async fn worksheet_keeps_the_live_origin_after_edge_scroll() {
    let fixture = mount_worksheet(empty_model());
    wait_for_frame(&fixture, 0, 0).await;

    let previous_commit = fixture.state.committed_frame.get_untracked();
    fixture.model.update_value(|model| {
        model
            .set_top_left_visible_cell(25, 2)
            .expect("set edge-scroll origin");
    });
    fixture.state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::ViewportScrolled {
            sheet: 0,
            top_row: 25,
            left_col: 2,
        },
    ));
    wait_for_commit(&fixture, previous_commit).await;

    let origin = fixture.model.with_value(|model| {
        let view = model.get_selected_view();
        (view.top_row, view.left_column)
    });
    assert_eq!(origin, (25, 2));
}

#[wasm_bindgen_test]
async fn worksheet_ignores_geometry_from_the_previous_sheet() {
    let fixture = mount_worksheet(empty_model());
    wait_for_frame(&fixture, 0, 0).await;
    assert!(with_canvas(fixture.canvas, fixture.model, fixture.state, |_| ()).is_some());

    fixture.model.update_value(|model| {
        model.new_sheet().expect("add second sheet");
    });
    fixture.state.emit_event(SpreadsheetEvent::Structure(
        StructureEvent::WorksheetAdded {
            sheet: 1,
            name: "Sheet2".to_string(),
        },
    ));

    assert!(with_canvas(fixture.canvas, fixture.model, fixture.state, |_| ()).is_none());
    wait_for_frame(&fixture, 1, 0).await;
    assert!(with_canvas(fixture.canvas, fixture.model, fixture.state, |_| ()).is_some());
}

#[wasm_bindgen_test]
async fn worksheet_keeps_scroll_request_until_new_sheet_geometry_commits() {
    let mut model = empty_model();
    model.new_sheet().expect("add second sheet");
    model.set_selected_sheet(0).expect("select first sheet");
    let fixture = mount_worksheet(model);
    wait_for_frame(&fixture, 0, 0).await;

    fixture.model.update_value(|model| {
        model.set_selected_sheet(1).expect("select second sheet");
        model
            .set_selected_cell(100, 1)
            .expect("select a cell outside the current viewport");
        model
            .set_top_left_visible_cell(1, 1)
            .expect("keep the second sheet at its initial origin");
    });
    fixture.state.scroll_into_view.set_value(true);
    fixture.state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::ActiveSheetChanged {
            from_sheet: 0,
            to_sheet: 1,
        },
    ));

    let mut cell_is_visible = false;
    for _ in 0..8 {
        next_frame().await;
        cell_is_visible = with_canvas(fixture.canvas, fixture.model, fixture.state, |scene| {
            scene.cell_rect(100, 1).is_some()
        })
        .unwrap_or(false);
        if cell_is_visible && !fixture.state.scroll_into_view.get_value() {
            break;
        }
    }

    assert!(
        cell_is_visible,
        "the active cell must enter the committed viewport"
    );
    assert!(
        !fixture.state.scroll_into_view.get_value(),
        "the scroll request completes after matching geometry commits"
    );
    let origin = fixture.model.with_value(|model| {
        let view = model.get_selected_view();
        (view.top_row, view.left_column)
    });
    assert!(
        origin.0 > 1,
        "the viewport must scroll down to the active cell"
    );
}

#[wasm_bindgen_test]
async fn worksheet_ignores_geometry_from_the_previous_workbook() {
    let fixture = mount_worksheet(empty_model());
    wait_for_frame(&fixture, 0, 0).await;

    let previous_commit = fixture.state.committed_frame.get_untracked();
    fixture.model.update_value(|model| *model = empty_model());
    fixture.state.advance_workbook_generation();
    fixture
        .state
        .emit_event(SpreadsheetEvent::Structure(StructureEvent::DocumentReset));

    assert!(with_canvas(fixture.canvas, fixture.model, fixture.state, |_| ()).is_none());
    wait_for_commit(&fixture, previous_commit).await;
    wait_for_frame(&fixture, 0, 1).await;
    assert!(with_canvas(fixture.canvas, fixture.model, fixture.state, |_| ()).is_some());
}

#[wasm_bindgen_test]
async fn worksheet_refreshes_selection_after_hiding_a_sheet() {
    let mut model = empty_model();
    model
        .set_selected_cell(5, 2)
        .expect("select B5 on the first sheet");
    model.new_sheet().expect("add second sheet");
    model
        .set_selected_cell(1, 1)
        .expect("select A1 on the second sheet");

    let fixture = mount_worksheet(model);
    wait_for_frame(&fixture, 1, 0).await;

    let previous_commit = fixture.state.committed_frame.get_untracked();
    fixture
        .state
        .emit_event(SpreadsheetEvent::Format(FormatEvent::CellStyleChanged {
            address: CellAddress {
                sheet: 1,
                row: 1,
                column: 1,
            },
        }));
    wait_for_commit(&fixture, previous_commit).await;
    assert!(
        fixture
            .state
            .events
            .navigation
            .with_untracked(Vec::is_empty),
        "format change leaves navigation events empty"
    );

    let previous_commit = fixture.state.committed_frame.get_untracked();
    fixture.model.update_value(|model| {
        model.hide_sheet(1).expect("hide selected sheet");
    });
    fixture.state.emit_event(SpreadsheetEvent::Structure(
        StructureEvent::WorksheetHidden { sheet: 1 },
    ));
    wait_for_commit(&fixture, previous_commit).await;

    assert_eq!(
        fixture.state.overlays.get_untracked().active_cell,
        Some(iron_canvas_core::CellCoord { row: 5, col: 2 })
    );
}
