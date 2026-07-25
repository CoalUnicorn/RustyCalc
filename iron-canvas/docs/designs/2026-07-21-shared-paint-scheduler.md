# Shared Paint-Reason Classification and Scheduler — Design Spec

**Date:** 2026-07-21
**Status:** Superseded — rejected by `docs/reviews/2026-07-21-shared-paint-scheduler-review.md` (confirmed on direct re-verification: the scheduler bypassed `IronCanvas::paint_if_dirty`'s recording/playback guards, `PaintReason::Structural` was not actually equivalent to `requestRepaint`+`markContentDirty`, and classification can't be centralized without losing host-specific data work). Revised, narrower design at `../../docs/designs/2026-07-21-demand-driven-worksheet-camera-scheduling.md` (RustyCalc-scoped, no iron-canvas changes). Kept here as a record of the rejected approach.
**Scope:** Cross-consumer dirty-signal classification and paint scheduling for iron-canvas

## Problem

Telling iron-canvas "something changed, please repaint" is currently two separate jobs, and every consumer has to do both itself:

1. **Classify** the event into one of four calls — `requestRepaint`/`request_repaint`, `markContentDirty`/`mark_content_dirty`, `markRowsDamaged`/`mark_rows_damaged`, `requestOverlayRepaint`/`request_overlay_repaint_js` — picking the wrong (too broad) one forecloses a cheaper `PaintRegime` (`iron-canvas-core/src/orchestrator.rs:471-533`).
2. **Drive a paint loop** that actually calls `paintIfDirty()`/`paint_if_dirty()` often enough to show the result. Nothing on the Rust side schedules a frame — `paint_if_dirty` is cheap to call when idle (`iron-canvas-core/src/orchestrator.rs:433-436`, two `Cell` swaps) but something external still has to call it.

Both jobs are duplicated independently, right now, in this codebase:

- `Worksheet` (RustyCalc `src/components/workbook/worksheet/subscribe.rs:83-122`) centralizes classification for one consumer, but pairs it with three *more* pieces of shadow state that exist only to work around there being no cheap, shared way to ask "should I schedule a frame": `render_needed: RwSignal<bool>` (`worksheet/mod.rs:110`), `theme_dirty: StoredValue<bool>` (`worksheet/mod.rs:38,50`), and `raf_loop.rs`'s permanent `use_raf_fn` loop, which discards its own `Pausable` handle (`raf_loop.rs:45`) and so never stops once mounted. One confirmed correctness gap already lives here: the playback-tick path never re-arms `render_needed` after its first bump (`dev_tools_effects.rs:118-125`, `raf_loop.rs:100-124`).
- `Camera` (`src/components/workbook/camera/`) bypasses `IronCanvas` entirely and drives `iron_canvas_core::Orchestrator<WebSurface>` directly, with its **own independent** copy of classification (`camera/mod.rs:265-293`) and its own RAF loop (also `leptos_use::use_raf_fn`, `camera/mod.rs:70-90`).
- The paused `docs/superpowers/plans/2026-07-20-ironcalc-webapp-iron-canvas-swap.md` plan would have added a **third**, JS-side copy (`dirtySignal.ts`'s `DirtyReason`/`notifyDirty`) plus a hand-rolled permanent `requestAnimationFrame` loop in `WorksheetIron.tsx` — see `2026-07-21-managed-web-grid-api.md`, which independently proposes a demand-driven `wake()` for that same JS host.

Three independently-maintained copies of the same classification logic, two redundant shadow-state signals, and one already-confirmed bug in one of the three, are the actual problem. The fix is to have iron-canvas itself own both jobs once, rather than have each host reinvent them.

## Goals

- One implementation of "which paint call does this event imply," shared by Worksheet, Camera, and any future host (including the planned JS webapp).
- One implementation of "schedule at most one paint frame," driven by iron-canvas itself rather than pushed onto every caller.
- Delete `render_needed` and `theme_dirty` once the shared scheduler subsumes their job.
- Keep every existing public `IronCanvas` JS method name and signature unchanged — no migration required for hosts already calling them.
- Shape the new classification type so `2026-07-20-row-fingerprint-repaint.md` (independent, separately-timed effort) can make the coarse "content changed" case cheaper later with zero API change here.

## Non-goals

- Implementing row-fingerprinting itself. This spec is forward-compatible with it, not a prerequisite for it or dependent on it.
- Changing `Orchestrator::decide()`'s regime selection, `PaintRegime`, or any painter code. This spec only changes who classifies and who schedules — not how a scheduled paint is executed.
- Removing `paintIfDirty()`/`paint_if_dirty()` or the four existing raw dirty-signal methods from the public API. They stay, and become optional rather than mandatory for a host to drive itself.
- Solving Camera's exact `on_cleanup`/unmount hookup in this document — named as an open item for the implementation plan, not resolved here.
- Changing anything in `managed-web-grid-api.md`'s JS-facade design (canvas creation, `ResizeObserver`/DPR, font-load wiring, TypeScript types, the `iron-canvas`/`iron-canvas/raw` package split). This spec narrows that document's RAF section only.

## Design

### 1. `PaintReason` (`iron-canvas-core`)

A new enum, placed alongside `GridSignals`/`CellDamage` in `iron-canvas-core/src/signal.rs`, reusing the existing `RowSpan` type rather than inventing a new coordinate shape:

```rust
pub enum PaintReason {
    /// Cell content maybe changed. `rows: Some(span)` is today's
    /// `mark_rows_damaged` precision (Damage fast path); `rows: None` is
    /// today's `mark_content_dirty` (SlotsReuse, whole pane or masked —
    /// row-banded for free once row-fingerprinting lands, with no change
    /// to this type).
    Content { sheet: u32, rows: Option<RowSpan> },
    /// Selection, active cell, formula-ref/clipboard/point-range overlay,
    /// scroll position. Cheap layer, no model re-fetch.
    Overlay,
    /// Geometry, theme, freeze-panes, sheet switch, or anything
    /// unclassified. Forces a full repaint; drops the frame cache.
    Structural,
}
```

This is a generalization, not a new concept: it collapses today's four caller-facing methods into three variants because `Content`'s `rows` field already carries the distinction the fourth (`mark_rows_damaged`) exists for.

### 2. `Orchestrator::notify` (`iron-canvas-core`)

A new inherent method on `Orchestrator<S>`:

```rust
impl<S: Surface> Orchestrator<S> where S::P: BlitPainter {
    pub fn notify(&mut self, reason: PaintReason) {
        match reason {
            PaintReason::Structural => self.request_repaint(),
            PaintReason::Overlay => self.request_overlay_repaint(),
            PaintReason::Content { sheet, rows } => match rows {
                Some(span) => self.mark_rows_damaged(sheet, span),
                None => self.mark_content_dirty(),
            },
        }
    }
}
```

Purely additive: the existing `request_repaint`/`mark_content_dirty`/`mark_rows_damaged`/overlay-raise methods and their internal logic (`GridSignals` bits, `pending_damage`, `pending_content`) are reused verbatim, not reimplemented. This also retires the redundant two-call `refresh()` pattern the JS design doc proposed: `request_repaint()` already sets `self.last_frame = None` (`iron-canvas-core/src/orchestrator.rs:170-177`), which alone forces the next `decide()` to `PaintRegime::Fresh` regardless of any `CONTENT` bit — a follow-up `mark_content_dirty()` call adds nothing. `PaintReason::Structural` replaces both calls with one.

### 3. `Scheduler<S>` (`iron-canvas-canvas2d`)

`iron-canvas-core` has zero wasm/browser dependencies by design (it's shared with the SVG/PDF/recorder backends) — nothing touching `requestAnimationFrame` can live there. `iron-canvas-canvas2d` is the only crate both `IronCanvas` and Camera already depend on directly (`iron-canvas-web/Cargo.toml:24`, RustyCalc's workspace `Cargo.toml:40`, `camera/canvas.rs:7`), already carries the needed `web-sys` features (`Window` is already enabled), and has no existing `Closure`/RAF code to conflict with — new territory for the crate, but the right crate.

```rust
struct SchedulerInner<S: Surface> {
    orch: RefCell<Orchestrator<S>>,
    pending_frame: Cell<Option<i32>>,               // requestAnimationFrame's handle, for cancel
    disposed: Cell<bool>,
    raf_closure: RefCell<Option<Closure<dyn FnMut(f64)>>>,  // stored once, reused every frame
}

pub struct Scheduler<S: Surface>(Rc<SchedulerInner<S>>);

impl<S: Surface + 'static> Scheduler<S> {
    pub fn notify(&self, reason: PaintReason) {
        if self.0.disposed.get() { return; }
        self.0.orch.borrow_mut().notify(reason);
        if self.0.pending_frame.get().is_some() { return; }   // already scheduled this task
        let handle = window().request_animation_frame(/* stored closure, created lazily on first use */);
        self.0.pending_frame.set(Some(handle));
    }

    pub fn dispose(&self) {
        self.0.disposed.set(true);
        if let Some(h) = self.0.pending_frame.take() {
            window().cancel_animation_frame(h);
        }
        self.0.raf_closure.borrow_mut().take();   // breaks the Rc self-reference cycle
    }
}
```

The stored closure captures a clone of the same `Rc<SchedulerInner<S>>`, clears `pending_frame`, and calls `orch.borrow_mut().paint_if_dirty()` — it does **not** call `request_animation_frame` again itself; only a fresh `.notify()` schedules another frame. This is deliberately not the classic "forever self-rearming" RAF pattern; it fires once, paints if dirty, and stops. The closure-holding-a-reference-to-its-owner is still an intentional `Rc` cycle (`SchedulerInner` → its own `raf_closure` slot → a clone of `Rc<SchedulerInner>`), so `Drop` alone can never free it — `dispose()` dropping the closure slot is the only thing that breaks the cycle, and must be called explicitly from every teardown path.

`Surface` (`iron-canvas-core/src/layer/mod.rs:35-60`) has no `'static` bound written in its definition, but every concrete impl (`WebSurface`, `RecordingSurface<S>`) is lifetime-free today, so `S: 'static` holds without friction. `Orchestrator<S>` itself (`iron-canvas-core/src/orchestrator.rs:89-93`) has no lifetimes or `PhantomData` — nothing resists being placed inside `Rc<RefCell<_>>`.

### 4. `IronCanvas` integration (`iron-canvas-web`)

`IronCanvas.orch` changes from a plain `Orchestrator<FacadeSurface>` field to `Rc<RefCell<Orchestrator<FacadeSurface>>>`, with a new `scheduler: Scheduler<FacadeSurface>` field holding a clone of the same `Rc`. Every existing method that touches `self.orch` (hit-testing, resize, overlays, export, recording — the ~23-36 methods outside the dirty-signal cluster) keeps its exact public signature; only its body changes from `self.orch.foo()` to `self.orch.borrow_mut().foo()`.

The four dirty-signal methods become thin wrappers, **with no change to their JS-visible names or signatures**:

```rust
#[wasm_bindgen(js_name = "requestRepaint")]
pub fn request_repaint(&self) { self.scheduler.notify(PaintReason::Structural); }

#[wasm_bindgen(js_name = "markContentDirty")]
pub fn mark_content_dirty(&self) {
    let sheet = self.model.as_ref().map_or(0, |m| m.get_selected_sheet());
    self.scheduler.notify(PaintReason::Content { sheet, rows: None });
}

#[wasm_bindgen(js_name = "markRowsDamaged")]
pub fn mark_rows_damaged(&self, sheet: u32, row_start: i32, row_end: i32) {
    self.scheduler.notify(PaintReason::Content { sheet, rows: Some(RowSpan { r1: row_start, r2: row_end }) });
}

#[wasm_bindgen(js_name = "requestOverlayRepaint")]
pub fn request_overlay_repaint_js(&self) { self.scheduler.notify(PaintReason::Overlay); }
```

No new JS-facing type or method is needed for `PaintReason` itself — it never crosses the wasm boundary. `paintIfDirty()` stays exactly as it is today: an unscheduled, manual check-and-paint, still needed by dev-tools/recording playback (which explicitly requires a continuous tick independent of the dirty scheduler) and by any host that wants to drive its own loop instead. `IronCanvas::dispose(self)` (`orchestrator.rs:340`) additionally calls `self.scheduler.dispose()`.

**Consequence for the paused JS webapp swap plan:** `dirtySignal.ts`'s `DirtyReason` routing to these same four method names stays valid as written — it needs no change. Only `WorksheetIron.tsx`'s hand-rolled `requestAnimationFrame(loop)` block becomes unnecessary, because `IronCanvas` now schedules its own paint. Every other part of `managed-web-grid-api.md` (canvas creation and stacking, `ResizeObserver`/DPR, font-load wiring, typed `.d.ts`, the package split) is unaffected.

### 5. RustyCalc consumer changes

**Worksheet:** `raf_loop.rs` is deleted entirely. `render_needed` and `theme_dirty` are deleted. `subscribe.rs`'s current four-way dispatch (`subscribe.rs:83-122`) shrinks to a pure mapping function from `SpreadsheetEvent` to `PaintReason`, followed by one `.notify()` call — same event taxonomy, same routing decisions, now expressed as data instead of four different method calls picked by a match. The ad hoc `ic.request_repaint()` call in the `ResizeObserver` callback (`mod.rs:99`) and the font-load listener's direct calls (`raf_loop.rs:174-185`) both become `.notify(PaintReason::Structural)` calls on the same shared scheduler — no separate wake-up needed since `.notify()` already schedules.

**Camera:** `CameraCanvas` (`camera/canvas.rs:17-24`) gets the same `Rc<RefCell<Orchestrator<WebSurface>>>` + `Scheduler<WebSurface>` treatment as `IronCanvas`. Its own classification copy (`camera/mod.rs:265-293`) is replaced by the same event-to-`PaintReason` mapping pattern Worksheet uses. Its `use_raf_fn` loop (`camera/mod.rs:70-90`) is deleted. **Open item:** today, `Pausable::pause()` is called when a camera widget unmounts from its `<For>` list (`camera/mod.rs:70-72`); the equivalent `scheduler.dispose()` call needs an unmount hook, almost certainly Leptos's `on_cleanup`, but the exact site needs to be confirmed during implementation rather than asserted here.

## File Impact

| File | Change |
| --- | --- |
| `iron-canvas-core/src/signal.rs` | New `PaintReason` enum |
| `iron-canvas-core/src/orchestrator.rs` | New `Orchestrator::notify(&mut self, reason: PaintReason)` |
| `iron-canvas-canvas2d/src/scheduler.rs` | New `Scheduler<S>` (create/notify/dispose). No new dependency. |
| `iron-canvas-web/src/orchestrator.rs` | `IronCanvas.orch` becomes `Rc<RefCell<Orchestrator<FacadeSurface>>>`; new `scheduler` field; every `self.orch.` call site gets `.borrow()`/`.borrow_mut()`; the 4 dirty-signal methods become thin wrappers over `scheduler.notify()`; `dispose()` also disposes the scheduler. No public method name/signature changes. |
| `RustyCalc src/components/workbook/worksheet/raf_loop.rs` | Deleted |
| `RustyCalc src/components/workbook/worksheet/mod.rs` | `render_needed`, `theme_dirty` deleted |
| `RustyCalc src/components/workbook/worksheet/subscribe.rs` | Dispatch shrinks to `SpreadsheetEvent -> PaintReason` mapping + one `.notify()` |
| `RustyCalc src/components/workbook/camera/canvas.rs` | Same `Rc<RefCell>` + `Scheduler<WebSurface>` treatment |
| `RustyCalc src/components/workbook/camera/mod.rs` | `use_raf_fn` loop deleted; classification replaced by shared mapping; unmount hook needs `scheduler.dispose()` |

No change to `iron-canvas-core`'s `PaintRegime`, `decide()`, or any painter. No change to `docs/designs/2026-07-21-managed-web-grid-api.md`'s non-RAF sections.

## Trade-offs

### Benefits

- Collapses three independently-maintained classification copies (Worksheet, Camera, and what would have been a third in the JS webapp) into one.
- Deletes two shadow-state signals (`render_needed`, `theme_dirty`) that exist only to route around the problem this fixes at the root, along with the one confirmed re-arm gap in the playback path.
- Shrinks the paused swap plan's remaining work rather than growing it — `dirtySignal.ts` and the named-method vocabulary survive unchanged; only the JS-side RAF loop is removed.
- Makes row-fingerprinting a pure, API-silent win whenever it ships: `PaintReason::Content { rows: None }` gets cheaper without this spec changing.
- `refresh()`-style callers get a single correct call (`Structural`) instead of a two-call pattern where the second call is inert.

### Costs

- Wide mechanical diff across `iron-canvas-web/src/orchestrator.rs` (993 lines) and `camera/canvas.rs` — every existing `self.orch` access needs a borrow. Low risk per line (compiler-enforced), large review surface.
- A genuinely new pattern for this codebase: self-referential `Rc<RefCell<_>>` + a stored `Closure`, with an explicit dispose-breaks-the-cycle requirement. `iron-canvas-canvas2d` has no existing precedent for this (confirmed: zero `Closure`/`Rc<RefCell` usage in that crate today). Needs its own leak test, not just behavioral tests.
- `RefCell` introduces a new panic category (double-borrow) that plain field access could not have. Should be structurally safe given wasm's single-threaded execution and the closure only firing between synchronous execution windows, but is a standing constraint future edits to `orchestrator.rs`/`camera/canvas.rs` must respect.
- Camera's exact dispose/unmount hookup is unresolved pending implementation-time verification.
- Touches four crates plus two RustyCalc consumer modules in one coordinated change — a bigger review surface than either source design document proposed alone.

### Rejected alternatives

**Thin per-host RAF wrapper, classify-only sharing.** Centralize `PaintReason`/`notify` in `iron-canvas-core` but leave RAF scheduling as three small, independent per-host wrappers (JS, Worksheet, Camera). Lower risk (no self-referential `Rc<RefCell<_>>` anywhere), and captures most of the classification win. Rejected in favor of full centralization: RAF-loop code is not the part that has actually gone wrong (Worksheet's real bug was in the classification/re-arm interaction, not the loop mechanics), but three still-separate scheduling implementations means three places to keep the "coalesce to one frame per task" behavior correct.

**JS-only facade, exactly as `managed-web-grid-api.md` proposes, no Rust changes.** Smallest, fastest-to-ship option. Rejected because it leaves Worksheet's and Camera's independently-duplicated classification logic — including the confirmed playback re-arm gap — untouched, and would have added a third duplicate in JS rather than removing the existing two.

**Sequence row-fingerprinting first.** Would let `PaintReason` drop its `rows` field entirely once the renderer no longer needs callers to supply row precision. Rejected to avoid gating this fix on a separate, independently-timed effort; `PaintReason::Content { rows: Option<RowSpan> }` is already shaped to absorb that change silently whenever it lands.

## Tests

### Unit tests (`iron-canvas-core`, no wasm)

- Each `PaintReason` variant raises the same `GridSignals` bits / populates `pending_damage`/`pending_content` as today's equivalent method call.
- `PaintReason::Structural` alone reproduces what the two-call `refresh()` pattern produced.
- `Content { rows: Some(span) }` merges into `CellDamage` identically to today's `mark_rows_damaged`.

### `Scheduler` tests (`iron-canvas-canvas2d`, `wasm_bindgen_test` + `run_in_browser`)

- N synchronous `.notify()` calls within one task schedule exactly one RAF callback.
- An idle scheduler (no `.notify()` since the last paint) schedules no further callbacks.
- `.dispose()` cancels a pending frame; `.notify()` after `.dispose()` is a no-op.
- Repeated create/dispose cycles do not grow retained memory (leak test for the closure/`Rc` cycle).

### Regression / integration

- Existing recorder goldens for Worksheet and Camera replay to identical frames through the new `notify()` path.
- `subscribe.rs`'s event-to-`PaintReason` mapping is a pure function, tested independent of DOM/canvas mounting.

### Manual verification

- Worksheet: edit, scroll, resize, theme toggle, font load, and dev-tools playback each still repaint correctly with `raf_loop.rs` deleted.
- Camera: same checklist per widget, plus widget add/remove through its `<For>` list (dispose path).

## Review Checklist

- [ ] `PaintReason` variants cover today's four dirty-signal methods with no behavior loss.
- [ ] `Orchestrator::notify` is additive — existing raise/mark logic is reused, not reimplemented.
- [ ] `Scheduler::dispose` is called from every teardown path (`IronCanvas::dispose`, Camera's unmount) before the owning `Rc` is dropped.
- [ ] No public `IronCanvas` JS method name or signature changes.
- [ ] `render_needed` and `theme_dirty` are fully deleted, not left dormant.
- [ ] `managed-web-grid-api.md`'s non-RAF sections remain valid and unmodified by this spec.
- [ ] No behavior change to `Orchestrator::decide()`'s regime selection or any painter.
- [ ] A leak test proves repeated create/dispose does not grow retained `Rc` count.

## Relationship to existing designs

- **`2026-07-21-managed-web-grid-api.md`**: this spec narrows that document's "Demand-driven RAF" section to nothing — `IronCanvas` now schedules itself. Every other section (canvas creation and stacking, `ResizeObserver`/DPR, font-load wiring, typed `.d.ts`, the `iron-canvas`/`iron-canvas/raw` package split, `dirtySignal.ts`'s named-method routing) is unaffected and still needed.
- **`2026-07-20-row-fingerprint-repaint.md`**: fully independent, own timeline. `PaintReason::Content { rows: None }` is designed to keep working unmodified once that lands — its `SlotsReuse` mismatch handling starts diffing rows internally, making the coarse case cheaper with no change to this spec's public shape.

## Implementation Order

1. `PaintReason` in `iron-canvas-core`, plus `Orchestrator::notify` and its unit tests.
2. `Scheduler<S>` in `iron-canvas-canvas2d`, plus its `wasm_bindgen_test` suite (coalescing, idle, dispose, leak).
3. `IronCanvas` integration in `iron-canvas-web`: `Rc<RefCell>` field change, wrapper methods, `dispose()` update. Confirm every existing wasm-pack consumer (web-test, recording/playback) still passes unmodified.
4. Worksheet migration: delete `raf_loop.rs`, `render_needed`, `theme_dirty`; shrink `subscribe.rs`. Manual verification checklist.
5. Camera migration: same pattern, plus resolving the `on_cleanup`/dispose hookup left open in this spec. Manual verification checklist.
