//! Camera tool: snapshot a cell range into a portable styled DataGrid.

mod canvas;
mod extract;
mod watch;

pub use extract::extract_grid;
pub use watch::events_touch_source;

use canvas::CameraCanvas;
use leptos::html;
use leptos::prelude::*;
use leptos_use::use_raf_fn;
use leptos_use::utils::Pausable;
use wasm_bindgen::JsCast;

use crate::components::ui::range_picker::{RangeFormat, RangePickerInput};
use crate::coord::SheetRange;
use crate::state::{CameraSpec, ModelStore, RangeCaptureTarget, WorkbookState};

/// Grip strip height in CSS px — keep in sync with the grip div's style.
pub(crate) const GRIP_H: f64 = 14.0;

/// Widget border width in CSS px — keep in sync with the widget div's style.
const BORDER_PX: f64 = 1.0;

const MIN_W: f64 = 80.0;
const MIN_H: f64 = 60.0;
pub(crate) const MAX_W: f64 = 480.0;
pub(crate) const MAX_H: f64 = 320.0;

/// One floating camera widget. Owns its CameraCanvas (built lazily once
/// both `<canvas>` refs are mounted and the element has non-zero dimensions),
/// then becomes a paint-only no-op each frame — exactly the raf_loop idiom.
#[component]
pub fn Camera(spec: CameraSpec) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let grid_ref = NodeRef::<html::Canvas>::new();
    let overlay_ref = NodeRef::<html::Canvas>::new();
    let cam: StoredValue<Option<CameraCanvas>, LocalStorage> = StoredValue::new_local(None);
    let id = spec.id;

    // Reactive view of *this* camera's spec — the source of truth lives in
    // state.cameras so drag/resize/persistence all see one copy.
    let my_spec = Memo::new(move |_| {
        state
            .cameras
            .with(|all| all.iter().find(|c| c.id == id).copied())
    });

    let apply_autosize = move |c: &mut CameraCanvas| {
        let (gw, gh) = c.autosize();
        let w = (gw + 2.0 * BORDER_PX).clamp(MIN_W, MAX_W);
        let h = (gh + GRIP_H + 2.0 * BORDER_PX).clamp(MIN_H, MAX_H);
        c.resize(
            w - 2.0 * BORDER_PX,
            h - GRIP_H - 2.0 * BORDER_PX,
            window().device_pixel_ratio(),
        );
        state.cameras.update(|cams| {
            if let Some(s) = cams.iter_mut().find(|s| s.id == id) {
                s.size = (w, h);
                s.autosize = false;
            }
        });
    };

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

    // Drag state: pointer offset from widget origin at grab time. Pointer
    // capture keeps moves flowing even when the cursor outruns the grip.
    // Both grab and move use client coordinates, so the container offset
    // cancels out — no per-move DOM reads required.
    let drag_offset: StoredValue<Option<(f64, f64)>, LocalStorage> = StoredValue::new_local(None);

    let on_grip_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let Some(spec) = my_spec.get_untracked() else {
            return;
        };
        if let Some(el) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        drag_offset.set_value(Some((
            ev.client_x() as f64 - spec.pos.0,
            ev.client_y() as f64 - spec.pos.1,
        )));
    };

    let on_grip_move = move |ev: web_sys::PointerEvent| {
        let Some((dx, dy)) = drag_offset.get_value() else {
            return;
        };
        // Capture can be lost without a pointerup (alt-tab, touch interrupt);
        // a buttonless move means the drag already ended.
        if ev.buttons() == 0 {
            drag_offset.set_value(None);
            return;
        }
        // pos is in workbook-container space; .max(0.0) prevents escaping top/left edge.
        let x = (ev.client_x() as f64 - dx).max(0.0);
        let y = (ev.client_y() as f64 - dy).max(0.0);
        state.cameras.update(|cams| {
            if let Some(c) = cams.iter_mut().find(|c| c.id == id) {
                c.pos = (x, y);
            }
        });
    };

    let on_grip_up = move |_: web_sys::PointerEvent| drag_offset.set_value(None);

    // Resize state: (start_x, start_y, start_w, start_h) at pointer-down.
    // Same pointer-capture pattern as drag; SE-corner handle only.
    let resize_grab: StoredValue<Option<(f64, f64, f64, f64)>, LocalStorage> =
        StoredValue::new_local(None);

    let on_handle_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        ev.stop_propagation();
        let Some(spec) = my_spec.get_untracked() else {
            return;
        };
        if let Some(el) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        resize_grab.set_value(Some((
            ev.client_x() as f64,
            ev.client_y() as f64,
            spec.size.0,
            spec.size.1,
        )));
    };

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

    let on_handle_up = move |_: web_sys::PointerEvent| resize_grab.set_value(None);

    // Wheel: scroll_by clamps internally; persisting the returned anchors
    // keeps CameraSpec.scroll truthful. The nested cameras.update inside
    // cam.update_value is safe: signal writes only schedule a reactive flush
    // after this closure returns — nothing re-enters cam synchronously.
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

    // EventBus → re-extract. Structure events (row/col insert/delete shift
    // ranges) and theme events (colors resolved at extraction) always trigger
    // a re-extract; content/format only when they intersect the source range.
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
            c.set_grid(model.with_value(|m| extract_grid(m, spec.source)));
            c.set_scroll(spec.scroll.0, spec.scroll.1);
        });
    });

    // --- Settings popover: re-point the source range ---

    let settings_open = RwSignal::new(false);
    // Display text for the range picker — updated by RangePickerInput while armed.
    let picker_text = RwSignal::new(String::new());
    // Structural capture: on each selection event while this camera is armed,
    // snapshot the real SheetRange so Apply can write it without an A1 parser.
    let pending_source: StoredValue<Option<SheetRange>, LocalStorage> =
        StoredValue::new_local(None);

    Effect::new(move |_| {
        let _ = state.events.navigation.get();
        if state.range_capture.get_untracked() == Some(RangeCaptureTarget::Camera(id)) {
            pending_source.set_value(Some(model.with_value(|m| SheetRange::from_view(m))));
        }
    });

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

    let on_cancel = move |_: web_sys::MouseEvent| {
        state.range_capture.set(None);
        settings_open.set(false);
    };

    let on_grip_button_down = move |ev: web_sys::PointerEvent| {
        ev.stop_propagation();
    };

    let on_gear_click = move |_: web_sys::MouseEvent| {
        let opening = !settings_open.get_untracked();
        if opening {
            // No A1 formatter accepts an arbitrary SheetRange (only the current
            // selection); leave the field empty so the ⊞ button seeds it on
            // first arm. The placeholder communicates intent.
            picker_text.set(String::new());
            pending_source.set_value(None);
        } else {
            state.range_capture.set(None);
        }
        settings_open.set(opening);
    };

    let on_close_click = move |_: web_sys::MouseEvent| {
        if state.range_capture.get_untracked() == Some(RangeCaptureTarget::Camera(id)) {
            state.range_capture.set(None);
        }
        state.cameras.update(|cams| cams.retain(|c| c.id != id));
    };

    view! {
        <div
            class="camera-widget"
            style=move || {
                let Some(s) = my_spec.get() else {
                    return String::new();
                };
                format!(
                    "position:absolute; left:{}px; top:{}px; width:{}px; height:{}px; \
                     pointer-events:auto; border:1px solid var(--border-color); \
                     box-shadow: 0 2px 8px rgba(0,0,0,0.25); background: var(--bg-primary); \
                     display:flex; flex-direction:column;",
                    s.pos.0, s.pos.1, s.size.0, s.size.1
                )
            }
        >
            <div
                class="camera-grip"
                on:pointerdown=on_grip_down
                on:pointermove=on_grip_move
                on:pointerup=on_grip_up
                on:pointercancel=on_grip_up
                style="position:relative; height:14px; cursor:grab; flex:none; \
                       background: var(--border-color); opacity:0.35; \
                       display:flex; align-items:center; justify-content:flex-end;"
            >
                <button
                    on:pointerdown=on_grip_button_down
                    on:click=on_gear_click
                    type="button"
                    title="Settings"
                    style="pointer-events:auto; padding:0 2px; font-size:10px; \
                           line-height:14px; background:transparent; border:none; \
                           cursor:pointer; opacity:0.8;"
                >"⚙"</button>
                <button
                    on:pointerdown=on_grip_button_down
                    on:click=on_close_click
                    type="button"
                    title="Delete camera"
                    style="pointer-events:auto; padding:0 2px; font-size:10px; \
                           line-height:14px; background:transparent; border:none; \
                           cursor:pointer; opacity:0.8;"
                >"✕"</button>
            </div>
            <Show when=move || settings_open.get()>
                <div style="position:absolute; top:14px; left:4px; right:4px; z-index:2; \
                            background: var(--bg-primary); border:1px solid var(--border-color); \
                            padding:4px; display:flex; gap:4px; flex-wrap:wrap; align-items:center;">
                    <RangePickerInput
                        value=picker_text
                        target=RangeCaptureTarget::Camera(id)
                        format=RangeFormat::SheetRelative
                        placeholder="Source range"
                    />
                    <button type="button" class="cam-btn-apply" on:click=on_apply>"Apply"</button>
                    <button type="button" class="cam-btn-cancel" on:click=on_cancel>"Cancel"</button>
                </div>
            </Show>
            <div
                on:wheel=on_wheel
                style="position:relative; flex:1; min-height:0;"
            >
                <canvas node_ref=grid_ref style="position:absolute; inset:0; width:100%; height:100%;"></canvas>
                <canvas node_ref=overlay_ref style="position:absolute; inset:0; width:100%; height:100%;"></canvas>
            </div>
            <div
                on:pointerdown=on_handle_down
                on:pointermove=on_handle_move
                on:pointerup=on_handle_up
                on:pointercancel=on_handle_up
                on:dblclick=move |_| cam.update_value(|slot| {
                    if let Some(c) = slot.as_mut() {
                        apply_autosize(c);
                    }
                })
                style="position:absolute; right:0; bottom:0; width:12px; height:12px; \
                       cursor:nwse-resize;"
            ></div>
        </div>
    }
}

/// All cameras, floating above the worksheet. `pointer-events:none` on the
/// layer keeps the grid interactive between widgets.
#[component]
pub fn CameraLayer() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    view! {
        <div
            class="camera-layer"
            style="position:absolute; inset:0; pointer-events:none; z-index:5;"
        >
            <For
                each=move || state.cameras.get()
                key=|c| c.id
                children=move |spec| view! { <Camera spec /> }
            />
        </div>
    }
}
