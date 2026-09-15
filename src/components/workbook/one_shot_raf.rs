//! Demand-driven wrapper over `leptos_use::use_raf_fn`, shared by
//! `worksheet::raf_loop` and `camera`. Replaces a permanent, unconditional
//! per-frame loop with one that runs only while there is real work to do.
//!
//! See `docs/designs/2026-07-21-demand-driven-worksheet-camera-scheduling.md`.

use leptos_use::utils::Pausable;
use leptos_use::{UseRafFnOptions, use_raf_fn_with_options};
use std::cell::RefCell;
use std::rc::Rc;

type PauseFn = Box<dyn Fn()>;

/// `paint` runs on every animation frame while it returns `true` (e.g.
/// still waiting for canvas refs to mount, or a recording is actively
/// playing back); once it returns `false` the loop self-pauses and goes
/// idle until the returned closure (`poke`) is called again.
///
/// `paint` must never reference `Orchestrator`, `IronCanvas`, or
/// `CameraCanvas` types here -- this primitive is generic scheduling only;
/// the caller's closure owns the actual paint operation.
pub(crate) fn use_one_shot_raf(paint: impl Fn() -> bool + 'static) -> impl Fn() + Clone {
    let pause_slot: Rc<RefCell<Option<PauseFn>>> = Rc::new(RefCell::new(None));
    let slot_for_cb = Rc::clone(&pause_slot);
    let Pausable { pause, resume, .. } = use_raf_fn_with_options(
        move |_| {
            if !paint()
                && let Some(p) = slot_for_cb.borrow().as_ref()
            {
                p();
            }
        },
        UseRafFnOptions::default().immediate(false),
    );
    *pause_slot.borrow_mut() = Some(Box::new(pause));
    resume(); // kick off the initial frame (e.g. construction polling)
    resume
}
