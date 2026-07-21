//! Event-category -> IronCanvas dispatch decision.
//!
//! Reactive subscription Effect that tracks event signals and overlay
//! changes. Does NOT render — only sets `render_needed` so the rAF loop
//! can do the draw on the next animation frame.
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

use crate::coord::SheetRange;
use crate::events::ContentEvent;
use crate::input::mouse::CanvasHandle;
use crate::state::WorkbookState;
use iron_canvas_core::*;

use super::ClipboardDraw;
use super::overlay_memo::OverlayTuple;

pub(super) fn install_subscribe_effect(
    state: WorkbookState,
    canvas_handle: CanvasHandle,
    theme_dirty: StoredValue<bool>,
    reactive_overlay: Memo<OverlayTuple>,
    clipboard_draw: ClipboardDraw,
    render_needed: RwSignal<bool>,
) {
    Effect::new(move |prev: Option<OverlayTuple>| {
        let content_events = state.events.content.get();
        let has_content = !content_events.is_empty();
        let has_structure = !state.events.structure.get().is_empty();
        let has_format = !state.events.format.get().is_empty();
        let has_nav = !state.events.navigation.get().is_empty();
        let has_theme = !state.events.theme.get().is_empty();
        let overlay = reactive_overlay.get();
        let overlay_changed = prev.is_some_and(|p| p != overlay);

        if !(has_content || has_structure || has_format || has_nav || has_theme || overlay_changed)
        {
            return overlay;
        }
        render_needed.set(true);

        // Push the same state into the IronCanvas orchestrator. Each setter
        // value-compares, so redundant pushes (e.g. format-only events not
        // touching theme) flip dirty only on the layers that actually need it.
        // Dirty routing is then explicit per event class (below): structure and
        // format request a full repaint (they can move slot geometry); content
        // takes the row-damage fast path where the event names its rows,
        // raising the overlay bit only when a nav event co-fires
        // (commit+Enter); nav-only repaints just the overlay.
        let OverlayTuple {
            extend_to,
            point_range,
            formula_refs,
        } = overlay.clone();
        let clipboard = clipboard_draw.with_value(|opt| {
            opt.as_ref().map(|acb| SheetRange {
                sheet: acb.sheet,
                area: acb.range,
            })
        });
        let overlays = RenderOverlays {
            extend_to,
            clipboard: clipboard.map(Into::into),
            point_range: point_range.map(Into::into),
            formula_refs: formula_refs.into_iter().map(Into::into).collect(),
        };
        if has_theme {
            theme_dirty.set_value(true);
        }
        canvas_handle.update_value(|slot| {
            if let Some(ic) = slot.as_mut() {
                ic.set_overlays(overlays);
                // Format events include row/col resize (LayoutChanged) — those
                // mutate slot pixel geometry and must drop last_frame, so they
                // route through requestRepaint with structure. Row-addressed
                // content edits feed markRowsDamaged (Damage regime: one row
                // band fetched + repainted); un-rowed events fall back to
                // markContentDirty, which poisons the batch to the pane-mask
                // path inside the engine — conservative, never wrong. Nav
                // co-firing (commit-Enter) needs an explicit overlay raise
                // because neither content raise touches the overlay bit.
                if has_structure || has_format {
                    ic.request_repaint();
                } else if has_content {
                    for event in &content_events {
                        match event {
                            ContentEvent::CellChanged { address, .. } => {
                                ic.mark_rows_damaged(address.sheet, address.row, address.row);
                            }
                            ContentEvent::RangeChanged { sheet_area } => {
                                ic.mark_rows_damaged(
                                    sheet_area.sheet,
                                    sheet_area.area.r1,
                                    sheet_area.area.r2,
                                );
                            }
                            ContentEvent::FormulaChanged { .. }
                            | ContentEvent::CalculationUpdated { .. }
                            | ContentEvent::NamedRangesChanged => ic.mark_content_dirty(),
                        }
                    }
                    if has_nav {
                        ic.request_overlay_repaint();
                    }
                } else if has_nav {
                    ic.request_overlay_repaint();
                }
            }
        });

        overlay
    });
}
