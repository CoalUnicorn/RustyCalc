//! Browser regressions for the inspector and its generic window.

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

use crate::app_state::AppState;
use crate::components::panels::perf_panel::PerfPanel;
use crate::components::ui::floating_window::FloatingWindow;
use crate::events::EventBus;
use crate::perf::{AttemptKey, AttemptOrigin, AttemptRecord, CaptureState};
use iron_canvas_core::FrameDiagnostics;

wasm_bindgen_test_configure!(run_in_browser);

async fn flush() {
    for _ in 0..4 {
        leptos::task::tick().await;
    }
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        window().request_animation_frame(&resolve).unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    leptos::task::tick().await;
}

fn host() -> web_sys::HtmlElement {
    let host = document().create_element("div").unwrap();
    let style = document().create_element("style").unwrap();
    style.set_text_content(Some(include_str!("../../styles/ui/floating-window.css")));
    host.append_child(&style).unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host.unchecked_into()
}

fn element(host: &web_sys::HtmlElement, selector: &str) -> web_sys::HtmlElement {
    host.query_selector(selector)
        .unwrap()
        .expect(selector)
        .unchecked_into()
}

fn event(target: &web_sys::HtmlElement, kind: &str, properties: &str) {
    // Construct real DOM event classes without adding web-sys feature flags.
    js_sys::Function::new_with_args(
        "target",
        &format!(
            "target.dispatchEvent(new {}('{}', {{bubbles:true,cancelable:true,{properties}}}))",
            if kind.starts_with("pointer") {
                "PointerEvent"
            } else {
                "KeyboardEvent"
            },
            kind,
        ),
    )
    .call1(&wasm_bindgen::JsValue::NULL, target)
    .unwrap();
}

fn attempt(seq: u64) -> AttemptRecord {
    AttemptRecord {
        key: AttemptKey {
            generation: 0,
            attempt_seq: seq,
        },
        captured_at_ms: crate::perf::now(),
        render_call_ms: Some(1.0),
        origin: AttemptOrigin::Live,
        retry_of: None,
        batch_ids: vec![],
        sheet_name: Some("Sheet1".to_owned()),
        backing_size: (800, 600),
        diagnostics: FrameDiagnostics {
            attempt_seq: seq,
            ..Default::default()
        },
    }
}

#[wasm_bindgen_test]
async fn inspector_close_pauses_and_keeps_pinned_selection() {
    let host = host();
    host.set_id("workbook");
    host.set_tab_index(0);
    let _executor = leptos::mount::mount_to(
        document()
            .create_element("div")
            .unwrap()
            .unchecked_into::<web_sys::HtmlElement>(),
        || (),
    );
    let owner = Owner::new();
    let app = owner.with(|| AppState::new(EventBus::new()));
    let runtime = owner.with(|| leptos::mount::mount_to(host.clone(), move || {
        provide_context(app);
        view! {
            <button id="dev-tools-launcher" on:click=move |_| app.inspector_open.update(|open| *open = !*open)>"Perf"</button>
            <PerfPanel />
        }
    }));
    flush().await;
    app.perf_store.request_start(false, vec![]).unwrap();
    app.perf_store.confirm_start();
    app.perf_store.append_attempt(attempt(1));
    flush().await;
    element(&host, ".pp-tab:nth-child(2)").click();
    flush().await;
    let selected_button = element(&host, ".pp-table tbody button");
    selected_button.focus().unwrap();
    selected_button.click();
    app.perf_store.append_attempt(attempt(2));
    flush().await;
    assert_eq!(
        element(&host, ".pp-table tr.selected button")
            .text_content()
            .as_deref(),
        Some("#1")
    );
    assert_eq!(
        document().active_element().unwrap(),
        selected_button.unchecked_into::<web_sys::Element>()
    );
    element(&host, ".pp-tab:nth-child(3)").click();
    flush().await;
    assert_eq!(
        element(&host, ".pp-detail-attempt")
            .text_content()
            .as_deref(),
        Some("#1")
    );
    app.perf
        .frame_trace
        .set(Some("#99 live attempt".to_owned()));
    flush().await;
    assert!(
        element(&host, ".pp-advanced")
            .text_content()
            .unwrap()
            .contains("Latest live frame trace (not selected attempt)")
    );
    assert_eq!(
        element(&host, ".pp-detail-attempt")
            .text_content()
            .as_deref(),
        Some("#1")
    );
    element(&host, "#dev-tools-launcher").click();
    flush().await;
    assert_eq!(app.perf_store.state_untracked(), CaptureState::Paused(1));
    assert_eq!(
        app.perf_store
            .with_selected(|capture| capture.attempts.len()),
        Some(2)
    );
    assert_eq!(
        document().active_element().unwrap().id(),
        "dev-tools-launcher"
    );
    element(&host, "#dev-tools-launcher").click();
    flush().await;
    assert_eq!(app.perf_store.state_untracked(), CaptureState::Paused(1));
    assert!(
        element(&host, ".fw-shell").contains(
            document()
                .active_element()
                .as_ref()
                .map(|element| element.as_ref())
        )
    );
    event(&element(&host, ".fw-shell"), "keydown", "key:'Escape'");
    flush().await;
    assert!(!app.inspector_open.get_untracked());
    app.perf_store.finish(crate::perf::StopReason::Finished);
    element(&host, "#dev-tools-launcher").click();
    flush().await;
    element(
        &host,
        "button[title='Delete the selected finished capture']",
    )
    .click();
    flush().await;
    assert_eq!(app.perf_store.status_untracked().retained_captures, 0);
    // Other toolbar tabs do not mount the launcher.
    element(&host, "#dev-tools-launcher").set_id("inactive-launcher");
    event(&element(&host, ".fw-shell"), "keydown", "key:'Escape'");
    flush().await;
    assert_eq!(document().active_element().unwrap().id(), "workbook");
    drop(runtime);
    owner.cleanup();
    host.remove();
}

#[wasm_bindgen_test]
async fn floating_window_drag_starts_at_clamped_position_and_isolates_events() {
    let host = host();
    let owner = Owner::new();
    let (open, keys) = owner.with(|| (RwSignal::new(true), RwSignal::new(0)));
    let runtime = owner.with(|| {
        leptos::mount::mount_to(host.clone(), move || {
            view! {
                <div on:keydown=move |_| keys.update(|keys| *keys += 1)>
                    <input class="outside" />
                    <FloatingWindow open=open.read_only() set_open=open.write_only() title="Test"
                        default_pos=(10000.0, 10000.0) default_size=(10000.0, 10000.0)>
                        <input class="inside" on:keydown=|ev| {
                            if ev.key() == "Escape" { ev.prevent_default(); }
                        } />
                    </FloatingWindow>
                </div>
            }
        })
    });
    flush().await;
    flush().await;
    let shell = element(&host, ".fw-shell");
    let panel = element(&host, ".fw-panel");
    let before = panel.get_bounding_client_rect();
    assert!(before.right() <= window().inner_width().unwrap().as_f64().unwrap());
    assert!(before.bottom() <= window().inner_height().unwrap().as_f64().unwrap());
    let title = element(&host, ".fw-titlebar");
    let x = before.x() as i32 + 30;
    let y = before.y() as i32 + 10;
    event(
        &title,
        "pointerdown",
        &format!("pointerId:1,button:0,buttons:1,clientX:{x},clientY:{y}"),
    );
    event(
        &title,
        "pointermove",
        &format!("pointerId:1,buttons:1,clientX:{},clientY:{y}", x - 5),
    );
    flush().await;
    let after = panel.get_bounding_client_rect();
    assert!(
        (after.x() - (before.x() - 5.0).max(4.0)).abs() < 2.0,
        "drag jumped: {} -> {}",
        before.x(),
        after.x()
    );
    event(&title, "pointerup", "pointerId:1,buttons:0");
    let size_before = shell.get_bounding_client_rect();
    let handle = element(&host, ".fw-resize");
    event(
        &handle,
        "pointerdown",
        "pointerId:1,button:0,buttons:1,clientX:500,clientY:400",
    );
    event(
        &handle,
        "pointermove",
        "pointerId:1,buttons:1,clientX:470,clientY:370",
    );
    flush().await;
    let size_after = shell.get_bounding_client_rect();
    assert!(
        size_after.width() < size_before.width(),
        "resize must use the rendered size"
    );
    event(&handle, "pointerup", "pointerId:1,buttons:0");
    event(&element(&host, ".inside"), "keydown", "key:'Escape'");
    assert!(open.get_untracked(), "child handles Escape first");
    assert_eq!(keys.get_untracked(), 0);
    event(&element(&host, ".outside"), "keydown", "key:'Escape'");
    assert!(open.get_untracked(), "outside Escape belongs to the sheet");
    assert_eq!(keys.get_untracked(), 1);
    element(&host, ".outside").click();
    flush().await;
    assert!(
        open.get_untracked(),
        "outside click must keep the window open"
    );
    event(&shell, "keydown", "key:'Escape'");
    assert!(!open.get_untracked());
    drop(runtime);
    owner.cleanup();
    host.remove();
}
