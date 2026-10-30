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
use crate::scene::{SceneHandle, clamp_viewport_origin, clipboard_range_on_sheet, request_for};
use crate::state::{ModelStore, WorkbookState};
use iron_canvas::RevisionToken;
use iron_canvas_core::{CanvasSize, CanvasTheme, CellCoord};

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

        // 2. Clamp the live model origin to its current sheet's frozen bands.
        //    Never copy the previous frame's origin here: the model can hold a
        //    newer scroll request, or belong to another sheet or workbook.
        let (sheet, requested_origin) = model.with_value(|m| {
            let view = m.get_selected_view();
            let frozen_rows = m.get_frozen_rows_count(view.sheet).unwrap_or(0);
            let frozen_columns = m.get_frozen_columns_count(view.sheet).unwrap_or(0);
            (
                view.sheet,
                clamp_viewport_origin(
                    CellCoord {
                        row: view.top_row,
                        col: view.left_column,
                    },
                    frozen_rows,
                    frozen_columns,
                ),
            )
        });
        let (mut top_row, mut left_col) = (requested_origin.row, requested_origin.col);
        let current_origin = model.with_value(|m| {
            let view = m.get_selected_view();
            CellCoord {
                row: view.top_row,
                col: view.left_column,
            }
        });
        if requested_origin != current_origin {
            model.update_value(|m| {
                if let Err(e) =
                    m.set_top_left_visible_cell(requested_origin.row, requested_origin.col)
                {
                    web_sys::console::warn_1(&format!("[rustycalc nav] origin clamp: {e}").into());
                }
            });
        }
        let workbook_id = state.workbook_generation.get_value();

        // A navigation asked for the active cell to be brought into view. Only
        // the renderer can say whether it already fits — it alone knows the
        // pane extent, the frozen bands and the partial trailing row.
        if state.scroll_into_view.get_value() {
            let (row, column) = model.with_value(|m| {
                let view = m.get_selected_view();
                (view.row, view.column)
            });
            let target = canvas_handle.with_value(|slot| {
                slot.as_ref()
                    .filter(|h| h.matches_context(sheet, workbook_id))
                    .map(|h| {
                        model.with_value(|m| h.scroll_to_show(m, sheet, workbook_id, row, column))
                    })
            });
            if let Some(target) = target {
                state.scroll_into_view.set_value(false);
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
            opt.as_ref().and_then(|clipboard| {
                clipboard_range_on_sheet(clipboard.sheet, clipboard.range, sheet)
            })
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
                workbook_id,
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
        let pane = canvas_handle.with_value(|slot| {
            slot.as_ref()
                .filter(|h| h.matches_context(sheet, workbook_id))
                .and_then(|h| h.scroll_pane_rect())
        });
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

        // Keep polling when a scroll request arrived before this sheet's
        // geometry committed. The next frame can then use matching geometry.
        state.scroll_into_view.get_value()
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
