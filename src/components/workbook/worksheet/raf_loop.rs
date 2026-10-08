//! Per-frame render loop — fires on demand via `use_one_shot_raf`'s
//! `poke`, coalesced to at most one paint per animation frame.
//!
//! Owns the lazy `SceneHandle` construction: runs every frame until the
//! `<canvas>` ref is mounted AND the container has nonzero dimensions, then
//! becomes demand-driven. On each tick it reconciles the model's viewport
//! against the committed frame (freeze clamp + scroll-into-view), builds the
//! `RenderRequest` from the current selection/overlays, and renders.
//!
//! The scene session owns frame reuse: an unchanged revision does no work, and
//! a changed one re-prepares against the previous frame, so there is no
//! per-category dirty routing here.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::html;
use leptos::prelude::*;

use crate::components::workbook::one_shot_raf::use_one_shot_raf;
use crate::input::mouse::CanvasHandle;
use crate::scene::{SceneHandle, request_for};
use crate::state::{ModelStore, WorkbookState};
use iron_canvas::RevisionToken;
use iron_canvas_core::{CanvasSize, CanvasTheme, CellCoord, scene_geometry::GridRange};

use super::ClipboardDraw;

/// Current size the request must be built at. `CanvasSize` is `f64`-exact:
/// `client_width`/`client_height` are integer logical pixels, so equality is
/// stable across frames.
fn default_size() -> CanvasSize {
    CanvasSize { w: 0.0, h: 0.0 }
}

pub(super) fn install_raf_loop(
    grid_ref: NodeRef<html::Canvas>,
    canvas_handle: CanvasHandle,
    model: ModelStore,
    state: WorkbookState,
    clipboard_draw: ClipboardDraw,
    theme_dirty: StoredValue<bool>,
) -> impl Fn() + Clone {
    let last_pane_w = Cell::new(0.0f64);
    let last_pane_h = Cell::new(0.0f64);
    // Workbook generation: the scene session must not reuse prepared content
    // across a workbook switch, even when the sheet index and theme match.
    let workbook_id = Cell::new(0u64);
    let last_uuid = Cell::new(None::<crate::storage::WorkbookId>);
    let theme_cache: Rc<RefCell<Option<CanvasTheme>>> = Rc::new(RefCell::new(None));

    let paint = move || -> bool {
        // 1. Ensure a session exists at the canvas's current size/scale. A
        //    DPR change (another monitor) does not fire the resize observer,
        //    so it is detected here too.
        let mut ready = false;
        canvas_handle.update_value(|slot| {
            let Some(grid_el) = grid_ref.get_untracked() else {
                return;
            };
            let w = grid_el.client_width() as f64;
            let h = grid_el.client_height() as f64;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let dpr = window().device_pixel_ratio();
            let size = CanvasSize { w, h };
            match slot {
                Some(handle) if handle.size() != size || handle.scale() != dpr => {
                    if let Err(e) = handle.resize(grid_el, size, dpr) {
                        web_sys::console::error_1(&format!("[rustycalc scene] resize: {e}").into());
                    }
                    ready = true;
                }
                Some(_) => ready = true,
                None => match SceneHandle::new(grid_el, size, dpr) {
                    Ok(handle) => {
                        *slot = Some(handle);
                        ready = true;
                    }
                    Err(e) => {
                        web_sys::console::error_1(&format!("[rustycalc scene] create: {e}").into());
                    }
                },
            }
        });
        if !ready {
            return true; // keep polling until refs mount and layout has measured
        }

        // 2. Bump the workbook generation whenever the model store is swapped.
        let uuid = state.current_uuid.get_untracked();
        if last_uuid.get() != uuid {
            last_uuid.set(uuid);
            workbook_id.set(workbook_id.get().wrapping_add(1));
        }

        // 3. View reconciliation — runs before the paint so a correction lands
        //    on this frame rather than flashing on the next one.

        // Freezing panes never moves `top_row`, so the model can hold an origin
        // inside the frozen run that no painted pixel agrees with. The renderer
        // clamps it silently; write the clamp back, because ironcalc's page
        // navigation derives its *new selection* from `top_row` and would
        // compute it from the stale value.
        let (sheet, mut top_row, mut left_col) = model.with_value(|m| {
            let view = m.get_selected_view();
            (view.sheet, view.top_row, view.left_column)
        });
        if let Some(origin) =
            canvas_handle.with_value(|slot| slot.as_ref().and_then(|h| h.scroll_origin()))
            && (origin.row, origin.col) != (top_row, left_col)
        {
            model.update_value(|m| {
                if let Err(e) = m.set_top_left_visible_cell(origin.row, origin.col) {
                    web_sys::console::warn_1(&format!("[rustycalc nav] origin sync: {e}").into());
                }
            });
            top_row = origin.row;
            left_col = origin.col;
        }

        // A navigation asked for the active cell to be brought into view. Only
        // the renderer can say whether it already fits — it alone knows the
        // pane extent, the frozen bands and the partial trailing row.
        if state.scroll_into_view.get_value() {
            state.scroll_into_view.set_value(false);
            let (row, column) = model.with_value(|m| {
                let view = m.get_selected_view();
                (view.row, view.column)
            });
            let target = model.with_value(|m| {
                canvas_handle
                    .with_value(|slot| slot.as_ref().and_then(|h| h.scroll_to_show(m, row, column)))
            });
            if let Some((top, left)) = target {
                // `scroll_to_show` already returns `None` when the target
                // matches the current origin, so `Some` names a real,
                // different origin.
                model.update_value(|m| match m.set_top_left_visible_cell(top, left) {
                    Ok(()) => {}
                    Err(e) => {
                        web_sys::console::warn_1(
                            &format!("[rustycalc nav] scroll into view: {e}").into(),
                        );
                    }
                });
                top_row = top;
                left_col = left;
            }
        }

        // 4. Resolve the theme from CSS custom properties. Cached because
        //    `getComputedStyle` is expensive and the palette changes only when
        //    the theme event fires.
        let theme = {
            let mut cache = theme_cache.borrow_mut();
            if cache.is_none() || theme_dirty.get_value() {
                theme_dirty.set_value(false);
                let computed = window()
                    .document()
                    .and_then(|d| d.document_element())
                    .map(|el| iron_canvas_canvas2d::theme_from_element::from_element(&el))
                    .unwrap_or_else(CanvasTheme::light);
                *cache = Some(computed);
            }
            cache.as_ref().expect("theme cache set above").clone()
        };

        // 5. Build the frame request from the live selection/overlay state.
        let mut overlays = state.overlays.get_untracked();
        overlays.clipboard = clipboard_draw.with_value(|opt| {
            opt.as_ref()
                .map(|clipboard| GridRange::from(clipboard.range))
        });
        let size = canvas_handle
            .with_value(|slot| slot.as_ref().map(|h| h.size()))
            .unwrap_or_else(default_size);
        let request = request_for(
            sheet,
            CellCoord {
                row: top_row,
                col: left_col,
            },
            size,
            theme,
            overlays,
            RevisionToken {
                workbook_id: workbook_id.get(),
                revision: state.render_revision.get_untracked(),
            },
        );

        let result = model.with_value(|m| {
            canvas_handle
                .try_update_value(|slot| match slot.as_mut() {
                    Some(handle) => handle.render(m, &request).map(|_| ()),
                    None => Err("scene handle disappeared mid-frame".to_string()),
                })
                .unwrap_or_else(|| Err("scene handle unavailable".to_string()))
        });
        match result {
            Ok(()) => {
                // A commit just changed what is on screen: publish it so hover
                // state derived from committed link data is revalidated.
                state.committed_frame.update(|n| *n = n.wrapping_add(1));
            }
            Err(e) => {
                web_sys::console::error_1(&format!("[rustycalc scene] render: {e}").into());
            }
        }

        // 6. Sync the *scrollable pane* extent into the model — the budget
        //    ironcalc's on_arrow_* / on_page_* compare accumulated row heights
        //    against when deciding to scroll the active cell into view. The
        //    canvas is the wrong number: the headers and any frozen bands eat
        //    into it.
        let pane =
            canvas_handle.with_value(|slot| slot.as_ref().and_then(|h| h.scroll_pane_rect()));
        if let Some(pane) = pane {
            let pane_w = f64::from(pane.width);
            let pane_h = f64::from(pane.height);
            if pane_w != last_pane_w.get() || pane_h != last_pane_h.get() {
                model.update_value(|m| {
                    m.set_window_width(pane_w);
                    m.set_window_height(pane_h);
                });
                last_pane_w.set(pane_w);
                last_pane_h.set(pane_h);
            }
        }

        false
    };

    let poke = use_one_shot_raf(paint);

    // Webfont finished loading: clear the backend's text-measure memos and
    // poke the scheduler — cache clears alone never reach a repaint without
    // waking a currently-idle (self-paused) loop.
    let poke_for_fonts = poke.clone();
    let _ = leptos_use::use_event_listener(
        web_sys::EventTarget::from(document().fonts()),
        leptos::ev::Custom::<web_sys::Event>::new("loadingdone"),
        move |_| {
            canvas_handle.update_value(|slot| {
                if let Some(handle) = slot.as_mut() {
                    handle.invalidate();
                }
            });
            poke_for_fonts();
        },
    );

    poke
}
