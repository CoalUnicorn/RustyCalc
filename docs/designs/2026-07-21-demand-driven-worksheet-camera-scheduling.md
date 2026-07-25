# Demand-Driven Paint Scheduling for Worksheet and Camera — Design Spec

**Date:** 2026-07-21
**Status:** Draft
**Scope:** RustyCalc's two Leptos canvas consumers (`Worksheet`, `Camera`) only. No `iron-canvas` changes.

## Relationship to prior documents

This revises `iron-canvas/docs/designs/2026-07-21-shared-paint-scheduler.md`, which proposed centralizing both dirty-signal classification (`PaintReason`) and RAF scheduling (`Scheduler<S>`) inside `iron-canvas-canvas2d`, shared by JS, Worksheet, and Camera. `iron-canvas/docs/reviews/2026-07-21-shared-paint-scheduler-review.md` found that design unsound; direct re-verification against the actual code confirmed all five of its blockers:

1. A scheduler living below `IronCanvas` cannot correctly invoke "the complete paint operation." `IronCanvas::paint_if_dirty` (`iron-canvas-web/src/orchestrator.rs:226-245`) no-ops entirely during playback and brackets recording capture around the core call — a scheduler reaching into the bare core `Orchestrator` bypasses both. During an active recording, edits would paint but silently vanish from the captured `.icr` file.
2. `PaintReason::Structural` is not equivalent to `requestRepaint` + `markContentDirty`. `paint_fresh_regime` (`iron-canvas-core/src/orchestrator.rs:693-696`) only invalidates the pane cache when the `CONTENT` bit is present; `request_repaint` never raises it. The original design's "the second call is inert" claim verified regime *selection*, not what the selected regime does with the signal payload.
3. Classification is entangled with real per-host data work — Camera's event effect (`camera/mod.rs:262-292`) does range-intersection checks, theme resolution, and `DataGrid` re-extraction alongside deciding what changed; a generic core enum cannot absorb that.
4. Only 4 of the ~14 state-changing methods were wired to schedule anything; `set_overlays`/`set_theme`/`set_model`/`resize`/`add_decoration`/etc. all raise `GridSignals` independently and would have stalled.
5. The self-referential `Rc<RefCell<Orchestrator<S>>>` + stored `Closure` made a leaked scheduler the default outcome of any missed teardown, with Camera's cleanup hookup left unresolved.

`managed-web-grid-api.md` is unaffected by any of this — its JS-side `wake()` calling `raw.paintIfDirty()` was always correct (it calls the *complete* facade method, not a bypass). This document applies the same underlying idea — schedule a paint on demand, coalesced, only when something changed — to Worksheet and Camera, using Leptos's own tools rather than inventing shared Rust ownership machinery. `2026-07-20-row-fingerprint-repaint.md` is unrelated to any of this.

## Problem

`Worksheet` (`src/components/workbook/worksheet/raf_loop.rs`) and `Camera` (`src/components/workbook/camera/mod.rs`) each run a **permanent** `leptos_use::use_raf_fn` loop for the component's entire lifetime — a real ~60fps JS/wasm boundary crossing even when nothing has changed for minutes.

- Worksheet gates the actual paint behind `render_needed: RwSignal<bool>`. This gate is correctly set from most mutation paths (`subscribe.rs:53`, the font-load listener, both dev-tools playback commands) but **not from `mod.rs:138`'s workbook-switch `set_model` call** — confirmed by direct grep, no `render_needed` site exists near it. That call raises `STRUCTURAL | OVERLAY` internally and, today, relies entirely on `render_needed` happening to already be `true` for an unrelated reason to ever actually paint.
- Camera has no gate at all — `camera/mod.rs`'s loop calls `paint_if_dirty()` unconditionally on every frame, relying entirely on the engine's own cheap internal no-op check (confirmed: the font handler's own comment already notes "no render gate to poke").

Neither loop's cost is really about the *scheduling* mechanism being wrong in isolation — `leptos_use::use_raf_fn` is a fine primitive. It's that nothing currently uses it in **demand-driven** mode, and Worksheet's manual gate has at least one confirmed gap.

## Goals

- Worksheet and Camera each schedule a paint only when something changed, coalesced to at most one per animation frame, sharing one small piece of Leptos-level plumbing.
- Delete `render_needed` as a signal, replaced by explicit `poke()` calls at every site that currently sets it, plus the one confirmed-missing site.
- Preserve every other responsibility currently bundled into these loops, unchanged: lazy construction and the zero-size-container guard, playback's independent continuous tick, viewport-size mirroring into the model, paint-duration recording for the perf panel, `theme_dirty`'s deferred-DOM-read timing fence, and font-cache clearing.
- Zero changes to `iron-canvas-core`, `iron-canvas-canvas2d`, or `iron-canvas-web`. `managed-web-grid-api.md` stands as originally written.

## Non-goals

- A scheduler or classification type shared between Rust and JS hosts — rejected, see Relationship above.
- Changing the construction-polling strategy itself (still polls every frame until refs mount and have nonzero size; a `ResizeObserver`-based alternative is explicitly out of scope, matching `raf_loop.rs`'s own documented rationale for avoiding it).
- Changing playback's architecture — it keeps its own unconditional per-frame tick while active.
- Removing `theme_dirty` — it is a real timing fence (waiting for a DOM mutation to settle before reading CSS custom properties), not shadow state. Only `render_needed` is removed.
- Adding recorder golden tests for Worksheet/Camera — none exist today (confirmed: no `recorder`/`.icr`/`Recording` reference anywhere under `worksheet/` or `camera/`); not introduced here.
- Row-fingerprinting — unrelated.

## Design

### 1. `use_one_shot_raf` (new, RustyCalc-only)

A small wrapper over `leptos_use::use_raf_fn`, verified directly against its `0.19.0` source (`~/.cargo/registry/.../leptos-use-0.19.0/src/use_raf_fn.rs`): `resume()` is idempotent (no-ops if already active), `pause()` immediately cancels any in-flight `requestAnimationFrame` handle, and `use_raf_fn_with_options` already registers its own `on_cleanup(pause)` internally — disposal on scope exit is free, not something this design needs to solve.

```rust
/// Demand-driven wrapper over `leptos_use::use_raf_fn`. `paint` runs on
/// every animation frame while it returns `true` (still constructing, or a
/// recording is actively playing back); once it returns `false` the loop
/// self-pauses and goes idle until the returned `poke` is called again.
pub fn use_one_shot_raf(paint: impl Fn() -> bool + 'static) -> impl Fn() + Clone {
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
    resume();  // kick off the initial frame (construction polling)
    resume
}
```

The `pause_slot` indirection exists because the callback passed to `use_raf_fn_with_options` must be defined *before* the `Pausable` (and its `pause` handle) is returned — a plain ordering constraint, not a reference cycle: nothing here is self-referential the way the rejected design's `Rc<RefCell<Orchestrator>>` was, since this closure never touches an `Orchestrator`, `IronCanvas`, or `CameraCanvas` at all. Self-pausing from inside the callback costs one extra, harmless animation-frame tick: `loop_fn` unconditionally re-requests a frame after every callback invocation regardless of what the callback just did; that next frame checks `is_active`, finds it false, and returns without calling `paint` again.

Proposed location: `src/components/workbook/one_shot_raf.rs`, sibling to both `worksheet/` and `camera/` (confirm against whatever module convention `src/components/workbook/mod.rs` already establishes during implementation).

### 2. Worksheet migration

`raf_loop.rs`'s callback is restructured around the `paint -> bool` contract; its internal logic is preserved, not rewritten:

```rust
let paint = move || -> bool {
    if construct_if_needed(...) {           // unchanged: same polling logic
        return true;                        // keep polling until constructed
    }
    #[cfg(feature = "dev-tools")]
    let playing = tick_playback_if_active(...);  // unchanged: still unconditional while playing
    #[cfg(not(feature = "dev-tools"))]
    let playing = false;

    do_ordinary_paint(...);                 // unchanged: viewport-size mirror,
                                             // theme_dirty-gated deferred read,
                                             // paint_if_dirty, perf timing

    playing   // only reason to keep looping without a fresh poke: active playback
};
let poke = use_one_shot_raf(paint);
```

`render_needed` is deleted. Every current `render_needed.set(true)` site becomes a `poke()` call; `subscribe.rs`'s existing dispatch logic (which of the four raw dirty-signal methods to call for a given event) is untouched — it was never part of what the review found broken.

| Site | Today | Revised |
| --- | --- | --- |
| `subscribe.rs:53` | `render_needed.set(true)` | `poke()` |
| `raf_loop.rs:183` (font load, after `ic.fonts_changed()`) | `render_needed.set(true)` | `poke()` |
| `dev_tools_effects.rs:124` (`PlaybackCmd::Play`) | `render_needed.set(true)` | `poke()` |
| `dev_tools_effects.rs:142` (`PlaybackCmd::Exit`) | `render_needed.set(true)` | `poke()` |
| `mod.rs:138` (workbook-switch, after `ic.set_model(...)`) | *(none — confirmed gap today)* | `poke()`, closing a real pre-existing staleness gap |

### 3. Camera migration

Camera has no `render_needed`-equivalent today, so every mutation site that currently relies on "the loop is always running anyway" needs an explicit new `poke()` call:

| Site | Change |
| --- | --- |
| `camera/mod.rs:57,100,219` (resize ×3) | add `poke()` after each |
| `camera/mod.rs:265-293` (EventBus effect — `sync_theme_from_document`, `set_grid`) | add `poke()` after the existing calls; the data work itself (range-intersection check, scroll-anchor preservation) is unchanged |
| Font-load listener (`camera/canvas.rs:114-122` equivalent call site) | add `poke()` after `fonts_changed()` |

`camera/mod.rs:112`'s existing `on_cleanup(pause)` becomes redundant once built on `use_one_shot_raf` (see §1's disposal note) — safe to remove, not required to.

## File Impact

| File | Change |
| --- | --- |
| `src/components/workbook/one_shot_raf.rs` | New: `use_one_shot_raf` |
| `src/components/workbook/worksheet/raf_loop.rs` | Restructured around `paint -> bool`; construction/playback/ordinary-paint logic preserved |
| `src/components/workbook/worksheet/mod.rs` | `render_needed` deleted; `set_model` call site (~line 138) gains a `poke()` |
| `src/components/workbook/worksheet/subscribe.rs` | `render_needed.set(true)` → `poke()`; dispatch logic unchanged |
| `src/components/workbook/worksheet/dev_tools_effects.rs` | Same swap at both `render_needed.set(true)` sites |
| `src/components/workbook/camera/mod.rs` | `use_raf_fn` → `use_one_shot_raf`; `poke()` added at every resize/theme/grid/font mutation site; redundant `on_cleanup(pause)` removable |

No change to `iron-canvas-core`, `iron-canvas-canvas2d`, `iron-canvas-web`, or `managed-web-grid-api.md`.

## Trade-offs

### Benefits

- Kills two permanent 60fps loops, matching the demand-driven approach `managed-web-grid-api.md` already specifies for the JS host — without unifying the three hosts' implementations, which is exactly what the rejected design got wrong.
- Deletes `render_needed` and closes a real, confirmed staleness gap (the workbook-switch `set_model` site) as a byproduct of the same audit.
- Built entirely on `leptos_use::use_raf_fn`'s existing, already-in-production primitives. No new `wasm_bindgen::Closure`, no `Rc<RefCell<Orchestrator>>`, no new sharp edge for this codebase.
- Disposal is free — already handled by `use_raf_fn`'s own `on_cleanup`.
- Each host's "complete paint operation" stays exactly where it already correctly lives — no risk of the recording/playback bypass that sank the prior design.

### Costs

- Camera's migration touches more call sites than Worksheet's — it had no gate at all, so every mutation site is new plumbing rather than a one-line swap.
- One harmless extra animation-frame tick per self-pause (documented above).
- `theme_dirty` remains as a third piece of state alongside the engine's own `GridSignals`; this design removes `render_needed` only. Removing `theme_dirty` too would require restructuring when the DOM theme read happens, which is out of scope here.

## Tests

- `use_one_shot_raf`: N synchronous `poke()` calls within one task run `paint` once (coalescing); a `paint` that returns `true` keeps the loop alive across frames; `false` stops it after one frame. Dropping the owning scope stops the loop — this exercises `leptos_use`'s own `on_cleanup` wiring, not new logic, so the test confirms integration rather than the primitive itself.
- Manual verification — Worksheet: construction on mount, edit, scroll, resize, theme toggle, font load, and dev-tools playback (load/play/pause/seek/exit) all still repaint correctly with `raf_loop.rs` restructured.
- Manual verification — Camera: same checklist per widget, plus add/remove through the `<For>` list (dispose path), plus the workbook-switch gap specifically — switch workbooks and confirm the new sheet paints without needing an unrelated interaction to nudge it.

## Review Checklist

- [ ] `render_needed` is fully deleted; every site that set it now calls `poke()`, including the previously-missing `set_model` site.
- [ ] `theme_dirty` is preserved unchanged, not folded into a generic reason.
- [ ] Playback's continuous tick is preserved exactly, independent of `poke()`.
- [ ] No `iron-canvas-core`, `iron-canvas-canvas2d`, or `iron-canvas-web` file changes.
- [ ] `use_one_shot_raf` never touches `Orchestrator`, `IronCanvas`, or `CameraCanvas` — it is paint-operation-agnostic.
- [ ] Camera's every resize/theme/grid/font mutation site gained a `poke()`.
