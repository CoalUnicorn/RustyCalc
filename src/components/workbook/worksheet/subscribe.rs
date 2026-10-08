//! Event-category -> scene render dispatch decision.
//!
//! Reactive subscription Effect that tracks event signals and overlay
//! changes. Does NOT render — it publishes the overlay payload and bumps the
//! render revision, then pokes the demand-driven render loop
//! (`use_one_shot_raf`) so it can draw on the next animation frame.
//!
//! Decoupling subscription from rendering is the key to smooth navigation:
//! holding an arrow key fires ~30 keydown events per second, each emitting
//! a NavigationEvent. Without rAF coalescing every event would trigger a
//! synchronous canvas render. With this split, all events in a single
//! 16 ms frame coalesce into one draw call.
//!
//! Per-category subscription: reads directly from EventBus signals.
//! Each category signal is replaced (not appended) on every emit, so
//! reading any non-empty signal means a new action just happened. The
//! Effect returns the current overlay state so the next run can detect
//! overlay-only changes (autofill preview, point-mode range) without a
//! synthetic content event to force the redraw.

use leptos::prelude::*;

use crate::state::WorkbookState;
use iron_canvas::OverlayState;

pub(super) fn install_subscribe_effect(
    state: WorkbookState,
    theme_dirty: StoredValue<bool>,
    reactive_overlay: Memo<OverlayState>,
    poke: impl Fn() + Clone + 'static,
) {
    Effect::new(move |prev: Option<OverlayState>| {
        let has_content = !state.events.content.get().is_empty();
        let has_structure = !state.events.structure.get().is_empty();
        let has_format = !state.events.format.get().is_empty();
        let has_nav = !state.events.navigation.get().is_empty();
        let has_theme = !state.events.theme.get().is_empty();
        let overlay = reactive_overlay.get();
        // The first run must publish the initial overlay (selection, active
        // cell) even though no event has fired yet, so the very first frame
        // shows the current selection rather than an empty one.
        let first = prev.is_none();
        let overlay_changed = prev.as_ref().is_some_and(|previous| previous != &overlay);

        if !(first
            || has_content
            || has_structure
            || has_format
            || has_nav
            || has_theme
            || overlay_changed)
        {
            return overlay;
        }
        if has_theme {
            theme_dirty.set_value(true);
        }
        // Publish the paint payload every time; the rAF loop stamps it into the
        // next request. Only *content-class* events advance the revision: the
        // session re-prepares when the revision moves, so a content edit must
        // move it, while a nav/overlay change is already distinguished by the
        // request's viewport/overlays and reuses prepared content (M3's
        // overlay-only path). A theme change is distinguished by the request
        // theme. Matches the legacy dirty routing (content = row damage,
        // structure/format = full repaint, nav = overlay-only).
        state.overlays.set(overlay.clone());
        if has_content || has_structure || has_format {
            state
                .render_revision
                .update(|revision| *revision = revision.wrapping_add(1));
        }
        poke();

        overlay
    });
}
