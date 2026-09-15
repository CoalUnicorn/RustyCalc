//! Wire schema: one [`DrawOp`] per painter call, the [`RecordingFilter`] that
//! decides what a capture omits, and [`replay`], which drives any painter from
//! a log.

use std::collections::HashSet;

use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Point, Span};
use iron_canvas_core::painter::{BlitPainter, GroupClass, PaintColor, TextAlign, TextBaseline};
use serde::{Deserialize, Serialize};

/// Which layer surfaces a recording captures. Single enum (rather than two
/// bools) makes "record neither" unrepresentable — disabling both layers
/// is the same as not recording at all, which `startRecording` rejects by
/// not being called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayerScope {
    #[default]
    Both,
    GridOnly,
    OverlayOnly,
}

impl LayerScope {
    pub fn includes_grid(self) -> bool {
        matches!(self, Self::Both | Self::GridOnly)
    }
    pub fn includes_overlay(self) -> bool {
        matches!(self, Self::Both | Self::OverlayOnly)
    }
}

/// What a recording omits. `layers` skips entire surfaces (grid or
/// overlay); `skip_groups` drops named `begin_group`/`end_group` brackets
/// (and their contents, recursively) within recorded surfaces.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RecordingFilter {
    pub layers: LayerScope,
    pub skip_groups: HashSet<GroupClass>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DrawOp {
    RectFill {
        rect: PixelRect,
        color: String,
    },
    FillPath {
        points: Vec<Point>,
        color: String,
    },
    ClearRect {
        rect: PixelRect,
    },
    RectStroke {
        rect: PixelRect,
        color: String,
        width: f64,
    },
    RectDashed {
        rect: PixelRect,
        color: String,
        width: f64,
    },
    StrokeLine {
        line: Line,
        color: String,
        width: f64,
    },
    StrokeHLine {
        span: Span,
        y: f64,
        color: String,
        width: f64,
    },
    StrokeVLine {
        x: f64,
        span: Span,
        color: String,
        width: f64,
    },
    StrokeTextHLine {
        x1: f64,
        x2: f64,
        y: f64,
        color: String,
        width: f64,
    },
    PushClip {
        rect: PixelRect,
    },
    PopClip,
    FillText {
        text: String,
        x: f64,
        y: f64,
        font_css: String,
        color: String,
        align: TextAlign,
        baseline: TextBaseline,
    },
    InvalidateCache,
    ResetTextDefaults,
    ApplyDprTransform {
        dpr: f64,
    },
    BeginGroup {
        class: GroupClass,
    },
    EndGroup,
    Blit {
        src: PixelRect,
        dst: PixelRect,
    },
}

/// Replay a captured op log onto any `BlitPainter`. Debug-visualizer
/// path — e.g. record once via `RecorderPainter`, then replay onto
/// `SvgPainter` for a golden artifact, or onto a second `RecorderPainter`
/// to round-trip the log. Calls `target.invalidate_cache()` once before
/// dispatch so the target's ctx-state cache (`last_fill` / `last_stroke` /
/// `last_font` / `last_line_width`) is not desync'd against the replayed
/// stream.
///
/// Recorded color / font strings are owned `String`s — replay routes them
/// through `PaintColor::Borrowed`, which falls back to the content-eq
/// cache on the target (no ptr-eq fast path). Built-in-theme ptr-eq is
/// only available on the original render pass.
pub fn replay<P: BlitPainter>(target: &P, ops: &[DrawOp]) {
    target.invalidate_cache();
    for op in ops {
        match op {
            DrawOp::RectFill { rect, color } => {
                target.rect_fill(*rect, PaintColor::Borrowed(color));
            }
            DrawOp::FillPath { points, color } => {
                target.fill_path(points, PaintColor::Borrowed(color));
            }
            DrawOp::ClearRect { rect } => target.clear_rect(*rect),
            DrawOp::RectStroke { rect, color, width } => {
                target.rect_stroke(*rect, PaintColor::Borrowed(color), *width);
            }
            DrawOp::RectDashed { rect, color, width } => {
                target.rect_dashed(*rect, PaintColor::Borrowed(color), *width);
            }
            DrawOp::StrokeLine { line, color, width } => {
                target.stroke_line(*line, PaintColor::Borrowed(color), *width);
            }
            DrawOp::StrokeHLine {
                span,
                y,
                color,
                width,
            } => {
                target.stroke_hline(*span, *y, PaintColor::Borrowed(color), *width);
            }
            DrawOp::StrokeVLine {
                x,
                span,
                color,
                width,
            } => {
                target.stroke_vline(*x, *span, PaintColor::Borrowed(color), *width);
            }
            DrawOp::StrokeTextHLine {
                x1,
                x2,
                y,
                color,
                width,
            } => {
                target.stroke_text_hline(*x1, *x2, *y, PaintColor::Borrowed(color), *width);
            }
            DrawOp::PushClip { rect } => target.push_clip(*rect),
            DrawOp::PopClip => target.pop_clip(),
            DrawOp::FillText {
                text,
                x,
                y,
                font_css,
                color,
                align,
                baseline,
            } => target.fill_text(
                text,
                *x,
                *y,
                PaintColor::Borrowed(font_css),
                PaintColor::Borrowed(color),
                *align,
                *baseline,
            ),
            DrawOp::InvalidateCache => target.invalidate_cache(),
            DrawOp::ResetTextDefaults => target.reset_text_defaults(),
            DrawOp::ApplyDprTransform { dpr } => target.apply_dpr_transform(*dpr),
            DrawOp::BeginGroup { class } => target.begin_group(*class),
            DrawOp::EndGroup => target.end_group(),
            DrawOp::Blit { src, dst } => target.blit(*src, *dst),
        }
    }
}
