use crate::Owner;
use crate::components::workbook::one_shot_raf::use_one_shot_raf;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let win = web_sys::window().expect("window");
        let _ = win.request_animation_frame(&resolve);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

#[wasm_bindgen_test]
async fn poke_coalesces_and_self_pauses() {
    let owner = Owner::new();
    let (calls, poke) = owner.with(|| {
        let calls = Rc::new(Cell::new(0));
        let calls_for_paint = Rc::clone(&calls);
        let poke = use_one_shot_raf(move || {
            calls_for_paint.set(calls_for_paint.get() + 1);
            false
        });
        (calls, poke)
    });

    // use_one_shot_raf kicks off one frame immediately on creation.
    next_frame().await;
    assert_eq!(calls.get(), 1, "runs paint once on creation");

    // Idle: the loop self-paused after that one frame.
    next_frame().await;
    assert_eq!(calls.get(), 1, "idle loop must not repaint without a poke");

    // Ten synchronous pokes in one task coalesce into a single next frame.
    for _ in 0..10 {
        poke();
    }
    next_frame().await;
    assert_eq!(calls.get(), 2, "N synchronous pokes run paint exactly once");

    // Drain the harmless trailing tick from the self-pause above before
    // `owner` drops -- self-pausing mid-callback still lets one more frame
    // get scheduled (loop_fn requests the next frame unconditionally after
    // every callback), so one more no-op frame is already in flight.
    next_frame().await;
}
