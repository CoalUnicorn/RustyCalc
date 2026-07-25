# Demand-Driven Worksheet and Camera Scheduling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Worksheet's and Camera's permanent, unconditional per-frame `requestAnimationFrame` loops with a shared, demand-driven scheduler that runs a paint only when something changed, at most once per frame.

**Architecture:** A new `use_one_shot_raf` primitive wraps `leptos_use::use_raf_fn` so the loop starts paused and self-pauses after each frame unless told to keep going; callers get back a `poke()` handle. Worksheet's `raf_loop.rs` and Camera's `mod.rs` are each restructured around this, replacing the `render_needed: RwSignal<bool>` gate (Worksheet) and the "loop runs forever, relies on `paint_if_dirty`'s internal no-op" pattern (Camera) with explicit `poke()` calls at every mutation site.

**Tech Stack:** Rust, Leptos 0.7 (`leptos::prelude::*`), `leptos_use` 0.19 (`use_raf_fn_with_options`, `UseRafFnOptions`, `Pausable`), `wasm-bindgen-test` + `wasm-bindgen-futures` for the one automated test.

## Global Constraints

- Zero changes to `iron-canvas-core`, `iron-canvas-canvas2d`, `iron-canvas-web`, or `iron-canvas/docs/designs/2026-07-21-managed-web-grid-api.md`. Every file this plan touches is under RustyCalc's own `src/`.
- No new Cargo dependencies. `wasm-bindgen-test`, `wasm-bindgen-futures`, `js-sys`, and `leptos-use` (with its `immediate` option) are already present in `Cargo.toml` — confirmed directly, no `cargo add` needed anywhere in this plan.
- `theme_dirty: StoredValue<bool>` must be preserved exactly, unchanged in behavior — it is a real timing fence (deferring a DOM CSS-variable read until after `<html data-theme>` settles), not shadow state to remove. Only `render_needed` is deleted.
- Playback's continuous per-frame tick (while `app.playback_loaded && app.playback_playing`) must be preserved exactly, independent of `poke()`.
- `use_one_shot_raf` must never reference `Orchestrator`, `IronCanvas`, `CameraCanvas`, or any `iron-canvas` type — it is a fully generic, paint-operation-agnostic scheduling primitive. This is the exact property whose absence sank the prior rejected design (`iron-canvas/docs/designs/2026-07-21-shared-paint-scheduler.md`, see `iron-canvas/docs/reviews/2026-07-21-shared-paint-scheduler-review.md`).
- **Never run `git commit`.** Every task's last step stages changes with `git add` and stops — the user commits manually.

---

### Task 1: `use_one_shot_raf` scheduling primitive

**Files:**
- Create: `src/components/workbook/one_shot_raf.rs`
- Create: `src/test/one_shot_raf.rs`
- Modify: `src/test/mod.rs` (register the new test module)
- Modify: `src/components/workbook/mod.rs` (register the new source module)

**Interfaces:**
- Produces: `pub(crate) fn use_one_shot_raf(paint: impl Fn() -> bool + 'static) -> impl Fn() + Clone`. Tasks 2 and 3 both call this and use the returned closure (referred to as `poke` at call sites) to wake the scheduler.

- [ ] **Step 1: Write the failing test**

Create `src/test/one_shot_raf.rs`:

```rust
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
```

Register it in `src/test/mod.rs` (module list is alphabetical — `one_shot_raf` sorts between `mouse` and `state`) — find:

```rust
mod mouse;
mod state;
```

Replace with:

```rust
mod mouse;
mod one_shot_raf;
mod state;
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd /home/mmm/01_Dev/RustyCalc && cargo check --target wasm32-unknown-unknown --tests`
Expected: FAIL — `unresolved import` or `failed to resolve: could not find one_shot_raf in workbook` (the source module doesn't exist yet).

- [ ] **Step 3: Implement `use_one_shot_raf`**

Create `src/components/workbook/one_shot_raf.rs`:

```rust
//! Demand-driven wrapper over `leptos_use::use_raf_fn`, shared by
//! `worksheet::raf_loop` and `camera`. Replaces a permanent, unconditional
//! per-frame loop with one that runs only while there is real work to do.
//!
//! See `docs/designs/2026-07-21-demand-driven-worksheet-camera-scheduling.md`.

use leptos_use::utils::Pausable;
use leptos_use::{UseRafFnOptions, use_raf_fn_with_options};
use std::cell::RefCell;
use std::rc::Rc;

/// `paint` runs on every animation frame while it returns `true` (e.g.
/// still waiting for canvas refs to mount, or a recording is actively
/// playing back); once it returns `false` the loop self-pauses and goes
/// idle until the returned closure (`poke`) is called again.
///
/// `paint` must never reference `Orchestrator`, `IronCanvas`, or
/// `CameraCanvas` types here -- this primitive is generic scheduling only;
/// the caller's closure owns the actual paint operation.
pub(crate) fn use_one_shot_raf(paint: impl Fn() -> bool + 'static) -> impl Fn() + Clone {
    let pause_slot: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let slot_for_cb = Rc::clone(&pause_slot);
    let Pausable { pause, resume, .. } = use_raf_fn_with_options(
        move |_| {
            if !paint() {
                if let Some(p) = slot_for_cb.borrow().as_ref() {
                    p();
                }
            }
        },
        UseRafFnOptions::default().immediate(false),
    );
    *pause_slot.borrow_mut() = Some(Box::new(pause));
    resume(); // kick off the initial frame (e.g. construction polling)
    resume
}
```

Register the module in `src/components/workbook/mod.rs` — find:

```rust
pub mod camera;
pub mod editing;
pub mod worksheet;
```

Replace with:

```rust
pub mod camera;
pub mod editing;
mod one_shot_raf;
pub mod worksheet;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd /home/mmm/01_Dev/RustyCalc && wasm-pack test --headless --chrome -- --test one_shot_raf` (or whatever invocation this project's existing `src/test/` suite uses — check `docs/` or an existing CI workflow for the exact headless-Chrome/`wasm-bindgen-cli` setup already documented for running this test module, since global `wasm-bindgen-cli` version skew has required a local pinned install before).
Expected: PASS — `poke_coalesces_and_self_pauses ... ok`.

- [ ] **Step 5: Run the full crate check**

Run: `cd /home/mmm/01_Dev/RustyCalc && cargo check --target wasm32-unknown-unknown && cargo clippy --target wasm32-unknown-unknown -- -D warnings`
Expected: both PASS, no warnings.

- [ ] **Step 6: Stage the changes**

```bash
git -C /home/mmm/01_Dev/RustyCalc add src/components/workbook/one_shot_raf.rs src/components/workbook/mod.rs src/test/one_shot_raf.rs src/test/mod.rs
```

Do not commit — user commits manually.

---

### Task 2: Worksheet migration

**Files:**
- Modify: `src/components/workbook/worksheet/raf_loop.rs`
- Modify: `src/components/workbook/worksheet/mod.rs`
- Modify: `src/components/workbook/worksheet/subscribe.rs`
- Modify: `src/components/workbook/worksheet/dev_tools_effects.rs`

**Interfaces:**
- Consumes: `use_one_shot_raf` from Task 1 (`crate::components::workbook::one_shot_raf::use_one_shot_raf`).
- Produces: `raf_loop::install_raf_loop(...) -> impl Fn() + Clone` (was `-> ()`). `subscribe::install_subscribe_effect`'s and `dev_tools_effects::install_playback_effect`'s last parameter changes from `render_needed: RwSignal<bool>` to `poke: impl Fn() + Clone + 'static`.

No existing automated test covers this component-mount/RAF-lifecycle glue (confirmed: nothing in `src/test/` mounts `Worksheet` or drives its RAF loop). Verification for this task is `cargo check`/`cargo clippy` plus the manual browser checklist in Step 6 — matching how this class of code is already verified in this project.

- [ ] **Step 1: Restructure `raf_loop.rs` around the `paint -> bool` contract**

Replace the full contents of `src/components/workbook/worksheet/raf_loop.rs` with:

```rust
//! Per-frame render loop — fires on demand via `use_one_shot_raf`'s
//! `poke`, coalesced to at most one paint per animation frame.
//!
//! Owns the lazy IronCanvas construction: runs every frame until both
//! `<canvas>` refs are mounted AND container dims > 0, then becomes a
//! no-op via the `slot.is_some()` short-circuit. Handles the
//! zero-size-container edge case (refs resolved but layout pass hasn't
//! measured yet) without extra ResizeObserver plumbing.
//!
//! `ic.paint_if_dirty()` is cheap and safe to call unconditionally (it
//! no-ops internally when nothing is dirty), so there is no separate
//! "should I paint" gate here beyond "did something poke() me" -- that
//! is exactly what `use_one_shot_raf` already answers.

use leptos::html;
use leptos::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use web_sys::HtmlCanvasElement;

use crate::app_state::AppState;
use crate::components::workbook::one_shot_raf::use_one_shot_raf;
use crate::coord::SheetRange;
use crate::input::mouse::CanvasHandle;
use crate::state::{ModelStore, Split};
use iron_canvas_core::*;
use iron_canvas_web::IronCanvas;

use super::ClipboardDraw;
use super::adapter::WorksheetModelAdapter;
use super::overlay_memo::OverlayTuple;

#[allow(clippy::too_many_arguments)]
pub(super) fn install_raf_loop(
    grid_ref: NodeRef<html::Canvas>,
    overlay_ref: NodeRef<html::Canvas>,
    canvas_handle: CanvasHandle,
    model: ModelStore,
    reactive_overlay: Memo<OverlayTuple>,
    clipboard_draw: ClipboardDraw,
    theme_dirty: StoredValue<bool>,
    app: Option<AppState>,
    show_headers: Split<bool>,
) -> impl Fn() + Clone {
    let last_canvas_w = Cell::new(0.0f64);
    let last_canvas_h = Cell::new(0.0f64);

    let paint = move || -> bool {
        canvas_handle.update_value(|slot| {
            if slot.is_some() {
                return;
            }
            let Some(grid_el) = grid_ref.get_untracked() else {
                return;
            };
            let Some(overlay_el) = overlay_ref.get_untracked() else {
                return;
            };
            let w = grid_el.client_width() as f64;
            let h = grid_el.client_height() as f64;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let dpr = window().device_pixel_ratio();
            match IronCanvas::create(grid_el, overlay_el) {
                Ok(mut ic) => {
                    ic.resize(w, h, dpr);
                    // Initial state push: sync current Worksheet state to the
                    // freshly-constructed orchestrator so the first frame is
                    // correct. Subsequent pushes are driven by the reactive
                    // subscribe Effect and the workbook-switch Effect.
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = window().document().and_then(|d| d.document_element()) {
                        ic.set_theme_from_element(&el);
                    }
                    ic.set_model(Rc::new(WorksheetModelAdapter {
                        store: model,
                        show_headers,
                    }));
                    let OverlayTuple {
                        extend_to,
                        point_range,
                        formula_refs,
                    } = reactive_overlay.get_untracked();
                    let clipboard = clipboard_draw.with_value(|opt| {
                        opt.as_ref().map(|acb| SheetRange {
                            sheet: acb.sheet,
                            area: acb.range,
                        })
                    });
                    ic.set_overlays(RenderOverlays {
                        extend_to,
                        clipboard: clipboard.map(Into::into),
                        point_range: point_range.map(Into::into),
                        formula_refs: formula_refs.into_iter().map(Into::into).collect(),
                    });
                    *slot = Some(ic);
                }
                Err(e) => web_sys::console::error_1(&e),
            }
        });

        let constructed = canvas_handle.with_value(|slot| slot.is_some());
        if !constructed {
            return true; // keep polling every frame until constructed
        }

        // Playback tick. Runs unconditionally so play cadence is independent
        // of ordinary paint work; keeps the loop alive on its own (`playing`)
        // rather than needing a poke() per frame while playing.
        let mut playing = false;
        #[cfg(feature = "dev-tools")]
        if let Some(app) = &app
            && app.playback_loaded.get_untracked()
            && app.playback_playing.get_untracked()
        {
            playing = true;
            canvas_handle.update_value(|slot| {
                if let Some(ic) = slot.as_mut() {
                    if ic.tick_playback(crate::perf::now()) {
                        app.playback_frame.set(ic.recording_current_frame());
                    }
                    // Engine auto-pauses at end-of-recording — mirror it.
                    if !ic.is_playing() {
                        app.playback_playing.set(false);
                    }
                }
            });
        }

        let Some(canvas) = grid_ref.get_untracked() else {
            return playing;
        };
        let canvas_el: HtmlCanvasElement = canvas;
        // Sync canvas dimensions into the model so scroll/autofill knows the
        // visible viewport size.
        let canvas_w = canvas_el.client_width() as f64;
        let canvas_h = canvas_el.client_height() as f64;
        if canvas_w != last_canvas_w.get() || canvas_h != last_canvas_h.get() {
            model.update_value(|m| {
                m.set_window_width(canvas_w);
                m.set_window_height(canvas_h);
            });
            last_canvas_w.set(canvas_w);
            last_canvas_h.set(canvas_h);
        }
        #[cfg(feature = "dev-tools")]
        web_sys::console::time_with_label("render");
        let paint_t0 = crate::perf::now();
        canvas_handle.update_value(|slot| {
            if let Some(ic) = slot.as_mut() {
                if theme_dirty.get_value() {
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = window().document().and_then(|d| d.document_element()) {
                        ic.set_theme_from_element(&el);
                    }
                    theme_dirty.set_value(false);
                }
                ic.paint_if_dirty();
            }
        });
        #[cfg(feature = "dev-tools")]
        web_sys::console::time_end_with_label("render");

        // Record paint duration for the PerfPanel. Skipped until the first
        // cell commit has happened so the panel stays on its placeholder
        // ("commit a cell to measure") and we don't spam the signal on
        // every scroll / resize / overlay tick.
        if let Some(app) = &app
            && app.perf.commit_start.get_untracked().is_some()
        {
            app.perf.render_ms.set(Some(crate::perf::now() - paint_t0));
        }

        playing
    };

    let poke = use_one_shot_raf(paint);

    // Webfont finished loading: clear the engine's text-measure memos and
    // poke the scheduler — engine-side cache clears alone never reach a
    // repaint without waking a currently-idle (self-paused) loop.
    let poke_for_fonts = poke.clone();
    let _ = leptos_use::use_event_listener(
        web_sys::EventTarget::from(document().fonts()),
        leptos::ev::Custom::<web_sys::Event>::new("loadingdone"),
        move |_| {
            canvas_handle.update_value(|slot| {
                if let Some(ic) = slot.as_mut() {
                    ic.fonts_changed();
                }
            });
            poke_for_fonts();
        },
    );

    poke
}
```

- [ ] **Step 2: Delete `render_needed`, reorder `install_raf_loop`, and thread `poke` through in `mod.rs`**

In `src/components/workbook/worksheet/mod.rs`, find:

```rust
    let clipboard_draw = expect_context::<ClipboardDraw>();
    let reactive_overlay = reactive_overlay(state, model);

    // Flag: set by the reactive subscription Effect, cleared by the rAF
    // render loop. Starts true so the first animation frame draws the
    // initial state without waiting for an event.
    let render_needed = RwSignal::new(true);

    subscribe::install_subscribe_effect(
        state,
        canvas_handle,
        theme_dirty,
        reactive_overlay,
        clipboard_draw,
        render_needed,
    );

    // Grow rows to fit multi-line / wrapped content on commit. Lives here
    // because it needs the `CanvasHandle` to measure glyphs; watches content
    // events only, so it can't loop on its own `SetRowHeight` (Format) emits.
    autofit::install_autofit_effect(state, canvas_handle, model);

    // Workbook-switch Effect — watching `current_uuid` gives us a deterministic
    // signal that fires once per workbook switch. Without a set_model call,
    // the orchestrator keeps last_frame from the old workbook (stale pane
    // geometry, stale sheet ID), and paint_if_dirty never drops it for a
    // Fresh rebuild. `set_model` is idempotent-safe — re-pushing the same
    // adapter triggers a full repaint.
    {
        let current_uuid = state.current_uuid.read();
        Effect::new(move |_| {
            let _uuid = current_uuid.get();
            canvas_handle.update_value(|slot| {
                if let Some(ic) = slot.as_mut() {
                    ic.set_model(Rc::new(WorksheetModelAdapter {
                        store: model,
                        show_headers: state.show_headers,
                    }));
                }
            });
        });
    }

    #[cfg(feature = "dev-tools")]
    {
        dev_tools_effects::install_recording_effect(state, app, canvas_handle);
        dev_tools_effects::install_playback_effect(state, app, canvas_handle, render_needed);
        dev_tools_effects::install_export_effect(state, app, canvas_handle);
    }

    raf_loop::install_raf_loop(
        grid_ref,
        overlay_ref,
        canvas_handle,
        model,
        reactive_overlay,
        clipboard_draw,
        theme_dirty,
        render_needed,
        Some(app),
        state.show_headers,
    );
```

Replace with:

```rust
    let clipboard_draw = expect_context::<ClipboardDraw>();
    let reactive_overlay = reactive_overlay(state, model);

    // `install_raf_loop` runs first so `poke` exists before anything below
    // needs to wake the (now demand-driven, self-pausing) render loop.
    let poke = raf_loop::install_raf_loop(
        grid_ref,
        overlay_ref,
        canvas_handle,
        model,
        reactive_overlay,
        clipboard_draw,
        theme_dirty,
        Some(app),
        state.show_headers,
    );

    subscribe::install_subscribe_effect(
        state,
        canvas_handle,
        theme_dirty,
        reactive_overlay,
        clipboard_draw,
        poke.clone(),
    );

    // Grow rows to fit multi-line / wrapped content on commit. Lives here
    // because it needs the `CanvasHandle` to measure glyphs; watches content
    // events only, so it can't loop on its own `SetRowHeight` (Format) emits.
    autofit::install_autofit_effect(state, canvas_handle, model);

    // Workbook-switch Effect — watching `current_uuid` gives us a deterministic
    // signal that fires once per workbook switch. Without a set_model call,
    // the orchestrator keeps last_frame from the old workbook (stale pane
    // geometry, stale sheet ID), and paint_if_dirty never drops it for a
    // Fresh rebuild. `set_model` is idempotent-safe — re-pushing the same
    // adapter triggers a full repaint. `poke()` after `set_model` closes a
    // real gap: nothing previously woke the render loop for a workbook
    // switch specifically (it only painted if `render_needed` happened to
    // already be true for an unrelated reason).
    {
        let current_uuid = state.current_uuid.read();
        let poke = poke.clone();
        Effect::new(move |_| {
            let _uuid = current_uuid.get();
            canvas_handle.update_value(|slot| {
                if let Some(ic) = slot.as_mut() {
                    ic.set_model(Rc::new(WorksheetModelAdapter {
                        store: model,
                        show_headers: state.show_headers,
                    }));
                }
            });
            poke();
        });
    }

    #[cfg(feature = "dev-tools")]
    {
        dev_tools_effects::install_recording_effect(state, app, canvas_handle);
        dev_tools_effects::install_playback_effect(state, app, canvas_handle, poke.clone());
        dev_tools_effects::install_export_effect(state, app, canvas_handle);
    }
```

- [ ] **Step 3: Update `subscribe.rs`'s signature and wake call**

In `src/components/workbook/worksheet/subscribe.rs`, find:

```rust
pub(super) fn install_subscribe_effect(
    state: WorkbookState,
    canvas_handle: CanvasHandle,
    theme_dirty: StoredValue<bool>,
    reactive_overlay: Memo<OverlayTuple>,
    clipboard_draw: ClipboardDraw,
    render_needed: RwSignal<bool>,
) {
```

Replace with:

```rust
pub(super) fn install_subscribe_effect(
    state: WorkbookState,
    canvas_handle: CanvasHandle,
    theme_dirty: StoredValue<bool>,
    reactive_overlay: Memo<OverlayTuple>,
    clipboard_draw: ClipboardDraw,
    poke: impl Fn() + Clone + 'static,
) {
```

Find:

```rust
        render_needed.set(true);
```

Replace with:

```rust
        poke();
```

- [ ] **Step 4: Update `dev_tools_effects.rs`'s `install_playback_effect` signature and both wake calls**

In `src/components/workbook/worksheet/dev_tools_effects.rs`, find:

```rust
pub(super) fn install_playback_effect(
    state: WorkbookState,
    app: AppState,
    canvas_handle: CanvasHandle,
    render_needed: RwSignal<bool>,
) {
```

Replace with:

```rust
pub(super) fn install_playback_effect(
    state: WorkbookState,
    app: AppState,
    canvas_handle: CanvasHandle,
    poke: impl Fn() + Clone + 'static,
) {
```

Find:

```rust
                PlaybackCmd::Play => match ic.play_recording(crate::perf::now()) {
                    Ok(()) => {
                        app.playback_playing.set(true);
                        // Wake the rAF: tickPlayback runs on every frame
                        // unconditionally, but a render_needed bump ensures
                        // we don't stall on a sleeping rAF after a long pause.
                        render_needed.set(true);
                    }
```

Replace with:

```rust
                PlaybackCmd::Play => match ic.play_recording(crate::perf::now()) {
                    Ok(()) => {
                        app.playback_playing.set(true);
                        // Wake the (self-pausing) render loop: raf_loop.rs's
                        // playback-tick block keeps itself going every frame
                        // once playing, but the loop may currently be paused
                        // if nothing else woke it since the last idle frame.
                        poke();
                    }
```

Find:

```rust
                PlaybackCmd::Exit => {
                    ic.exit_playback();
                    app.playback_loaded.set(false);
                    app.playback_playing.set(false);
                    app.playback_frame.set(0);
                    app.playback_frame_count.set(0);
                    // exitPlayback called request_repaint on the engine; we
                    // also need a rAF wake so paintIfDirty fires next tick.
                    render_needed.set(true);
                }
```

Replace with:

```rust
                PlaybackCmd::Exit => {
                    ic.exit_playback();
                    app.playback_loaded.set(false);
                    app.playback_playing.set(false);
                    app.playback_frame.set(0);
                    app.playback_frame_count.set(0);
                    // exitPlayback called request_repaint on the engine; poke
                    // so paintIfDirty actually fires on the next frame.
                    poke();
                }
```

- [ ] **Step 5: Run the full crate check**

Run: `cd /home/mmm/01_Dev/RustyCalc && cargo check --target wasm32-unknown-unknown && cargo clippy --target wasm32-unknown-unknown -- -D warnings`
Expected: both PASS, no warnings, no unused `RwSignal` import left behind in any of the four files (remove `RwSignal` from an import list only if the compiler flags it unused — `mod.rs` likely still uses `RwSignal` elsewhere for `settings_open`-style state, so check before removing any import wholesale).

- [ ] **Step 6: Manual browser verification**

Build and run the app (`trunk serve` or whatever this project's existing dev-server command is). Check:

- [ ] Cell edit, scroll, resize, and theme toggle each still repaint correctly.
- [ ] Opening devtools Performance/Rendering tools (or a quick counter inside `paint_if_dirty`'s call site) confirms the grid does **not** repaint every animation frame while idle — only in response to an actual interaction.
- [ ] A slow-loading custom font (throttle network in devtools, reload) doesn't leave text mismeasured once it finishes loading.
- [ ] Switch workbooks (open a different file / new workbook) — the new sheet paints correctly without needing an unrelated interaction to nudge it (this exercises the gap fixed in Step 2).
- [ ] With `dev-tools` feature enabled: load a recording, play it, pause, seek, exit — playback still ticks smoothly and ordinary painting resumes correctly after exit.

- [ ] **Step 7: Stage the changes**

```bash
git -C /home/mmm/01_Dev/RustyCalc add src/components/workbook/worksheet/raf_loop.rs src/components/workbook/worksheet/mod.rs src/components/workbook/worksheet/subscribe.rs src/components/workbook/worksheet/dev_tools_effects.rs
```

Do not commit — user commits manually.

---

### Task 3: Camera migration

**Files:**
- Modify: `src/components/workbook/camera/mod.rs`

**Interfaces:**
- Consumes: `use_one_shot_raf` from Task 1.

Same testing story as Task 2: no existing automated coverage for this component's RAF lifecycle (`src/test/camera.rs` covers only the pure `extract_grid`/`events_touch_source` functions, unaffected by this task). Verification is `cargo check`/`cargo clippy` plus a manual checklist.

- [ ] **Step 1: Replace the RAF loop construction**

In `src/components/workbook/camera/mod.rs`, find:

```rust
use canvas::CameraCanvas;
use leptos::html;
use leptos::prelude::*;
use leptos_use::use_raf_fn;
use leptos_use::utils::Pausable;
use wasm_bindgen::JsCast;
```

Replace with:

```rust
use canvas::CameraCanvas;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::workbook::one_shot_raf::use_one_shot_raf;
```

Find:

```rust
    // Unlike the worksheet's app-lifetime loop, cameras come and go with the
    // <For> list — pause the rAF loop on disposal or it outlives the widget.
    let Pausable { pause, .. } = use_raf_fn(move |_| {
        cam.update_value(|slot| {
            if slot.is_some() {
                if let Some(c) = slot.as_mut() {
                    c.paint_if_dirty();
                }
                return;
            }
            // Wait until both canvas elements are in the DOM.
            let Some(grid_el) = grid_ref.get_untracked() else {
                return;
            };
            let Some(overlay_el) = overlay_ref.get_untracked() else {
                return;
            };
            // Guard against the zero-size-container edge case: refs resolved
            // but the layout pass hasn't measured yet (mirrors raf_loop.rs).
            let w = grid_el.client_width() as f64;
            let h = grid_el.client_height() as f64;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let Some(spec) = my_spec.get_untracked() else {
                return;
            };
            let dpr = window().device_pixel_ratio();
            match CameraCanvas::create(grid_el, overlay_el) {
                Ok(mut c) => {
                    c.resize(w, h, dpr);
                    c.set_grid(model.with_value(|m| extract_grid(m, spec.source)));
                    c.set_scroll(spec.scroll.0, spec.scroll.1);
                    if spec.autosize {
                        apply_autosize(&mut c);
                    }
                    *slot = Some(c);
                }
                Err(e) => leptos::logging::error!("camera canvas init failed: {e:?}"),
            }
        });
    });
    on_cleanup(pause);

    // Webfont finished loading: cached text widths may be fallback-font
    // stale. Listener is scope-bound — use_event_listener detaches it when
    // this camera unmounts with the <For> list.
    let _ = leptos_use::use_event_listener(
        web_sys::EventTarget::from(document().fonts()),
        leptos::ev::Custom::<web_sys::Event>::new("loadingdone"),
        move |_| {
            cam.update_value(|slot| {
                if let Some(c) = slot.as_mut() {
                    c.fonts_changed();
                }
            });
        },
    );
```

Replace with:

```rust
    // Cameras come and go with the <For> list. `use_one_shot_raf`'s
    // internal `on_cleanup` (inside `leptos_use::use_raf_fn`) already
    // pauses on widget disposal, so no explicit pause hookup is needed here
    // (unlike the previous `use_raf_fn` + manual `on_cleanup(pause)` pair).
    let paint = move || -> bool {
        let constructed = cam.with_value(|slot| slot.is_some());
        if constructed {
            cam.update_value(|slot| {
                if let Some(c) = slot.as_mut() {
                    c.paint_if_dirty();
                }
            });
            return false;
        }
        // Wait until both canvas elements are in the DOM.
        let Some(grid_el) = grid_ref.get_untracked() else {
            return true;
        };
        let Some(overlay_el) = overlay_ref.get_untracked() else {
            return true;
        };
        // Guard against the zero-size-container edge case: refs resolved
        // but the layout pass hasn't measured yet (mirrors raf_loop.rs).
        let w = grid_el.client_width() as f64;
        let h = grid_el.client_height() as f64;
        if w <= 0.0 || h <= 0.0 {
            return true;
        }
        let Some(spec) = my_spec.get_untracked() else {
            return true;
        };
        let dpr = window().device_pixel_ratio();
        match CameraCanvas::create(grid_el, overlay_el) {
            Ok(mut c) => {
                c.resize(w, h, dpr);
                c.set_grid(model.with_value(|m| extract_grid(m, spec.source)));
                c.set_scroll(spec.scroll.0, spec.scroll.1);
                if spec.autosize {
                    apply_autosize(&mut c);
                }
                cam.update_value(|slot| *slot = Some(c));
            }
            Err(e) => leptos::logging::error!("camera canvas init failed: {e:?}"),
        }
        true
    };
    let poke = use_one_shot_raf(paint);

    // Webfont finished loading: cached text widths may be fallback-font
    // stale. Listener is scope-bound — use_event_listener detaches it when
    // this camera unmounts with the <For> list.
    let poke_for_fonts = poke.clone();
    let _ = leptos_use::use_event_listener(
        web_sys::EventTarget::from(document().fonts()),
        leptos::ev::Custom::<web_sys::Event>::new("loadingdone"),
        move |_| {
            cam.update_value(|slot| {
                if let Some(c) = slot.as_mut() {
                    c.fonts_changed();
                }
            });
            poke_for_fonts();
        },
    );
```

- [ ] **Step 2: Add `poke()` at every remaining mutation site**

Unlike Worksheet, Camera's loop previously ran unconditionally forever, so every one of these sites relied on "the loop is always running anyway" and needs an explicit new `poke()` call now.

In `src/components/workbook/camera/mod.rs`, find:

```rust
    let on_handle_move = move |ev: web_sys::PointerEvent| {
        let Some((sx, sy, sw, sh)) = resize_grab.get_value() else {
            return;
        };
        if ev.buttons() == 0 {
            resize_grab.set_value(None);
            return;
        }
        let w = (sw + ev.client_x() as f64 - sx).max(MIN_W);
        let h = (sh + ev.client_y() as f64 - sy).max(MIN_H);
        state.cameras.update(|cams| {
            if let Some(c) = cams.iter_mut().find(|c| c.id == id) {
                c.size = (w, h);
            }
        });
        // spec.size is the outer widget box; the canvas area excludes the
        // grip strip and the 1px borders (matching what client_width/height
        // measure on the init path).
        cam.update_value(|slot| {
            if let Some(c) = slot.as_mut() {
                c.resize(
                    w - 2.0 * BORDER_PX,
                    h - GRIP_H - 2.0 * BORDER_PX,
                    window().device_pixel_ratio(),
                );
            }
        });
    };
```

Replace with:

```rust
    let poke_for_resize = poke.clone();
    let on_handle_move = move |ev: web_sys::PointerEvent| {
        let Some((sx, sy, sw, sh)) = resize_grab.get_value() else {
            return;
        };
        if ev.buttons() == 0 {
            resize_grab.set_value(None);
            return;
        }
        let w = (sw + ev.client_x() as f64 - sx).max(MIN_W);
        let h = (sh + ev.client_y() as f64 - sy).max(MIN_H);
        state.cameras.update(|cams| {
            if let Some(c) = cams.iter_mut().find(|c| c.id == id) {
                c.size = (w, h);
            }
        });
        // spec.size is the outer widget box; the canvas area excludes the
        // grip strip and the 1px borders (matching what client_width/height
        // measure on the init path).
        cam.update_value(|slot| {
            if let Some(c) = slot.as_mut() {
                c.resize(
                    w - 2.0 * BORDER_PX,
                    h - GRIP_H - 2.0 * BORDER_PX,
                    window().device_pixel_ratio(),
                );
            }
        });
        poke_for_resize();
    };
```

Find:

```rust
    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        let d_rows = match ev.delta_y().partial_cmp(&0.0) {
            Some(std::cmp::Ordering::Greater) => 1,
            Some(std::cmp::Ordering::Less) => -1,
            _ => 0,
        };
        let d_cols = match ev.delta_x().partial_cmp(&0.0) {
            Some(std::cmp::Ordering::Greater) => 1,
            Some(std::cmp::Ordering::Less) => -1,
            _ => 0,
        };
        if d_rows == 0 && d_cols == 0 {
            return;
        }
        cam.update_value(|slot| {
            let Some(c) = slot.as_mut() else {
                return;
            };
            let (top, left) = c.scroll_by(d_rows, d_cols);
            state.cameras.update(|cams| {
                if let Some(s) = cams.iter_mut().find(|s| s.id == id) {
                    s.scroll = (top, left);
                }
            });
        });
    };
```

Replace with:

```rust
    let poke_for_wheel = poke.clone();
    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        let d_rows = match ev.delta_y().partial_cmp(&0.0) {
            Some(std::cmp::Ordering::Greater) => 1,
            Some(std::cmp::Ordering::Less) => -1,
            _ => 0,
        };
        let d_cols = match ev.delta_x().partial_cmp(&0.0) {
            Some(std::cmp::Ordering::Greater) => 1,
            Some(std::cmp::Ordering::Less) => -1,
            _ => 0,
        };
        if d_rows == 0 && d_cols == 0 {
            return;
        }
        cam.update_value(|slot| {
            let Some(c) = slot.as_mut() else {
                return;
            };
            let (top, left) = c.scroll_by(d_rows, d_cols);
            state.cameras.update(|cams| {
                if let Some(s) = cams.iter_mut().find(|s| s.id == id) {
                    s.scroll = (top, left);
                }
            });
        });
        poke_for_wheel();
    };
```

Find:

```rust
    Effect::new(move |_| {
        let content = state.events.content.get();
        let format = state.events.format.get();
        let structure_hit = !state.events.structure.get().is_empty();
        let theme_hit = !state.events.theme.get().is_empty();

        let Some(spec) = my_spec.get_untracked() else {
            return;
        };
        let local_hit = events_touch_source(spec.source, &content, &format);
        if !(structure_hit || theme_hit || local_hit) {
            return;
        }
        cam.update_value(|slot| {
            let Some(c) = slot.as_mut() else {
                return;
            };
            if theme_hit {
                c.sync_theme_from_document();
            }
            // `set_grid` swaps in a fresh DataGrid whose scroll resets to the
            // top, so preserve where the user is actually looking: snapshot the
            // live anchors first and restore them. Re-imposing spec.scroll would
            // yank a scrolled camera back on every recalc.
            let (top, left) = c.scroll_anchors();
            c.set_grid(model.with_value(|m| extract_grid(m, spec.source)));
            c.set_scroll(top, left);
        });
    });
```

Replace with:

```rust
    let poke_for_events = poke.clone();
    Effect::new(move |_| {
        let content = state.events.content.get();
        let format = state.events.format.get();
        let structure_hit = !state.events.structure.get().is_empty();
        let theme_hit = !state.events.theme.get().is_empty();

        let Some(spec) = my_spec.get_untracked() else {
            return;
        };
        let local_hit = events_touch_source(spec.source, &content, &format);
        if !(structure_hit || theme_hit || local_hit) {
            return;
        }
        cam.update_value(|slot| {
            let Some(c) = slot.as_mut() else {
                return;
            };
            if theme_hit {
                c.sync_theme_from_document();
            }
            // `set_grid` swaps in a fresh DataGrid whose scroll resets to the
            // top, so preserve where the user is actually looking: snapshot the
            // live anchors first and restore them. Re-imposing spec.scroll would
            // yank a scrolled camera back on every recalc.
            let (top, left) = c.scroll_anchors();
            c.set_grid(model.with_value(|m| extract_grid(m, spec.source)));
            c.set_scroll(top, left);
        });
        poke_for_events();
    });
```

Find:

```rust
    let on_apply = move |_: web_sys::MouseEvent| {
        if let Some(source) = pending_source.get_value() {
            state.cameras.update(|cams| {
                if let Some(c) = cams.iter_mut().find(|c| c.id == id) {
                    c.source = source;
                    c.scroll = (1, 1);
                }
            });
            cam.update_value(|slot| {
                if let Some(c) = slot.as_mut() {
                    c.set_grid(model.with_value(|m| extract_grid(m, source)));
                    c.set_scroll(1, 1);
                    // Re-point shrink-wraps to the new range; without this the
                    // widget keeps the previous range's (often wider) size.
                    apply_autosize(c);
                }
            });
        }
        state.range_capture.set(None);
        settings_open.set(false);
    };
```

Replace with:

```rust
    let poke_for_apply = poke.clone();
    let on_apply = move |_: web_sys::MouseEvent| {
        if let Some(source) = pending_source.get_value() {
            state.cameras.update(|cams| {
                if let Some(c) = cams.iter_mut().find(|c| c.id == id) {
                    c.source = source;
                    c.scroll = (1, 1);
                }
            });
            cam.update_value(|slot| {
                if let Some(c) = slot.as_mut() {
                    c.set_grid(model.with_value(|m| extract_grid(m, source)));
                    c.set_scroll(1, 1);
                    // Re-point shrink-wraps to the new range; without this the
                    // widget keeps the previous range's (often wider) size.
                    apply_autosize(c);
                }
            });
            poke_for_apply();
        }
        state.range_capture.set(None);
        settings_open.set(false);
    };
```

Find:

```rust
                on:dblclick=move |_| cam.update_value(|slot| {
                    if let Some(c) = slot.as_mut() {
                        apply_autosize(c);
                    }
                })
```

Replace with:

```rust
                on:dblclick={
                    let poke = poke.clone();
                    move |_| {
                        cam.update_value(|slot| {
                            if let Some(c) = slot.as_mut() {
                                apply_autosize(c);
                            }
                        });
                        poke();
                    }
                }
```

- [ ] **Step 3: Run the full crate check**

Run: `cd /home/mmm/01_Dev/RustyCalc && cargo check --target wasm32-unknown-unknown && cargo clippy --target wasm32-unknown-unknown -- -D warnings`
Expected: both PASS, no warnings.

- [ ] **Step 4: Manual browser verification**

- [ ] Add a camera widget; confirm it constructs, paints, and repaints on drag (position), resize (SE handle), wheel-scroll, and double-click autosize.
- [ ] Trigger a content/structure/theme event that should re-extract the camera's `DataGrid` (edit a cell inside its source range, insert a row/column that shifts it, toggle the app theme) — confirm the camera repaints with the new data.
- [ ] Remove a camera widget (✕ button) — confirm no console errors and no lingering animation-frame activity afterward (open devtools Performance/Rendering tools, confirm no repaint activity tied to the removed widget).
- [ ] Add and remove several cameras in quick succession — confirm no accumulating console errors (exercises the dispose path under Leptos's `<For>` list churn).

- [ ] **Step 5: Stage the changes**

```bash
git -C /home/mmm/01_Dev/RustyCalc add src/components/workbook/camera/mod.rs
```

Do not commit — user commits manually.
