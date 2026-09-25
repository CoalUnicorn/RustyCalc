#![cfg(target_arch = "wasm32")]
//! The grid canvas keeps its alpha channel, and `present()` replaces the
//! front canvas' pixels instead of blending over them.
//!
//! These tests exercise surface pixels directly. They do not establish
//! support for translucent sheet themes in the retained renderer.

use iron_canvas_canvas2d::WebSurface;
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::Point;
use iron_canvas_core::geometry::{CanvasMetrics, CanvasSize};
use iron_canvas_core::layer::Surface;
use iron_canvas_core::painter::{BlitPainter, PaintColor, Painter};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

wasm_bindgen_test_configure!(run_in_browser);

const SIDE: f64 = 8.0;
/// Half-alpha white: opaque compositing would flatten it to #808080.
const TRANSLUCENT: &str = "rgba(255, 255, 255, 0.5)";
const OPAQUE: &str = "#112233";

fn make_canvas() -> HtmlCanvasElement {
    let Some(window) = web_sys::window() else {
        panic!("browser window");
    };
    let Some(document) = window.document() else {
        panic!("document");
    };
    let Ok(element) = document.create_element("canvas") else {
        panic!("create canvas element");
    };
    let Ok(canvas) = element.dyn_into::<HtmlCanvasElement>() else {
        panic!("element is a canvas");
    };
    canvas
}

/// `getContext("2d")` is idempotent: this returns the context the surface's
/// painter writes through, so reads observe the live backing store.
fn ctx_of(canvas: &HtmlCanvasElement) -> CanvasRenderingContext2d {
    canvas
        .get_context("2d")
        .expect("getContext must not throw")
        .expect("2d context exists")
        .dyn_into::<CanvasRenderingContext2d>()
        .expect("context is CanvasRenderingContext2d")
}

fn pixel(canvas: &HtmlCanvasElement) -> [u8; 4] {
    let bytes = ctx_of(canvas)
        .get_image_data(0.0, 0.0, 1.0, 1.0)
        .expect("read painted pixel")
        .data()
        .0;
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

fn rect(side: f64) -> PixelRect {
    PixelRect {
        top_left: Point { x: 0, y: 0 },
        width: side as i32,
        height: side as i32,
    }
}

fn metrics(side: f64) -> CanvasMetrics {
    CanvasMetrics::new(CanvasSize { w: side, h: side }, 1.0)
        .expect("fixture canvas metrics are valid")
}

/// A double-buffered grid surface of `side` x `side` CSS pixels at DPR 1,
/// presenting into `front`. No DPR transform is applied, so backing pixels
/// and CSS pixels coincide and a read at (0, 0) is the rect's top-left.
fn grid(front: &HtmlCanvasElement, side: f64) -> WebSurface {
    let mut surface = WebSurface::grid(front.clone()).expect("create grid surface");
    surface.resize(metrics(side));
    surface
}

#[wasm_bindgen_test]
fn translucent_grid_paint_reaches_the_visible_canvas_intact() {
    let front = make_canvas();
    let surface = grid(&front, SIDE);
    surface
        .painter()
        .rect_fill(rect(SIDE), PaintColor::Static(TRANSLUCENT));
    surface.present();

    let [r, g, b, a] = pixel(&front);
    assert_eq!(
        (r, g, b),
        (255, 255, 255),
        "an alpha:false backing store would have blended the fill onto opaque black"
    );
    assert!(
        (i32::from(a) - 128).abs() <= 2,
        "a half-transparent fill must reach the visible canvas half-transparent, got alpha {a}"
    );
}

#[wasm_bindgen_test]
fn present_replaces_front_pixels_the_back_buffer_no_longer_covers() {
    let front = make_canvas();
    let surface = grid(&front, SIDE);
    surface
        .painter()
        .rect_fill(rect(SIDE), PaintColor::Static(OPAQUE));
    surface.present();
    assert_eq!(
        pixel(&front),
        [0x11, 0x22, 0x33, 255],
        "fixture: an opaque fill presents opaque"
    );

    // The next frame stops covering the pixel. A `source-over` copy would
    // leave the previous frame's pixel in place on the visible canvas.
    surface.painter().clear_rect(rect(SIDE));
    surface.present();
    assert_eq!(
        pixel(&front),
        [0, 0, 0, 0],
        "dropped coverage must leave the front canvas transparent, not stale"
    );
}

#[wasm_bindgen_test]
fn resize_re_pins_the_present_composite_op() {
    let front = make_canvas();
    let mut surface = grid(&front, SIDE);

    // A backing-store resize resets the drawing state of both grid contexts,
    // so the `copy` pin made at construction must be re-applied.
    surface.resize(metrics(SIDE * 2.0));
    surface
        .painter()
        .rect_fill(rect(SIDE * 2.0), PaintColor::Static(OPAQUE));
    surface.present();
    assert_eq!(pixel(&front), [0x11, 0x22, 0x33, 255]);

    surface.painter().clear_rect(rect(SIDE * 2.0));
    surface.present();
    assert_eq!(
        pixel(&front),
        [0, 0, 0, 0],
        "the post-resize copy must still replace, not blend"
    );
}

#[wasm_bindgen_test]
fn blit_replaces_alpha_and_preserves_pixels_outside_the_destination() {
    for dpr in [1.0, 1.25, 2.0] {
        // Exercise both the grid's front-to-back copy and an overlapping self-copy.
        for buffered in [true, false] {
            let front = make_canvas();
            let mut surface = if buffered {
                WebSurface::grid(front.clone())
            } else {
                WebSurface::overlay(front.clone())
            }
            .expect("create surface");
            surface.resize(CanvasMetrics::new(CanvasSize { w: 16.0, h: 8.0 }, dpr).unwrap());
            let painter = surface.painter();
            painter.apply_dpr_transform(dpr);
            let src = PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: 8,
                height: 8,
            };
            let dst = PixelRect {
                top_left: Point { x: 4, y: 0 },
                ..src
            };
            painter.rect_fill(PixelRect { width: 16, ..src }, PaintColor::Static(OPAQUE));
            painter.clear_rect(src);
            painter.rect_fill(
                PixelRect { width: 4, ..src },
                PaintColor::Static(TRANSLUCENT),
            );
            surface.present();
            let before = ctx_of(&front)
                .get_image_data(
                    0.0,
                    0.0,
                    f64::from(front.width()),
                    f64::from(front.height()),
                )
                .unwrap()
                .data()
                .0;
            painter.blit(src, dst);
            surface.present();
            let after = ctx_of(&front)
                .get_image_data(
                    0.0,
                    0.0,
                    f64::from(front.width()),
                    f64::from(front.height()),
                )
                .unwrap()
                .data()
                .0;
            let width = front.width() as usize;
            let shift = (4.0 * dpr) as usize;
            for y in 0..front.height() as usize {
                for x in 0..width {
                    let source_x = if (shift..3 * shift).contains(&x) {
                        x - shift
                    } else {
                        x
                    };
                    let actual = (y * width + x) * 4;
                    let expected = (y * width + source_x) * 4;
                    assert_eq!(
                        &after[actual..actual + 4],
                        &before[expected..expected + 4],
                        "DPR {dpr}, buffered {buffered}, pixel ({x}, {y})"
                    );
                }
            }
            // The temporary copy operation must not leak into later paint.
            painter.rect_fill(
                PixelRect {
                    top_left: Point { x: 12, y: 0 },
                    width: 4,
                    height: 8,
                },
                PaintColor::Static(TRANSLUCENT),
            );
            surface.present();
            assert_eq!(pixel(&front), [255, 255, 255, 128]);
        }
    }
}
