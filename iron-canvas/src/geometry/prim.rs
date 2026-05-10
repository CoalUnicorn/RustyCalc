use std::ops::RangeInclusive;

use crate::{
    chrome::Chrome,
    geometry::{
        constants::{HEADER_COL_WIDTH, HEADER_OFFSET, HEADER_ROW_HEIGHT},
        pixel_rect::PixelRect,
    },
    RCRange,
};

/// A point in logical (CSS) pixels on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// An axis-aligned line segment on the canvas.
///
/// Named fields per variant so callers can't transpose the scalars.
/// `offset_cross` shifts perpendicular to the line's direction - used by
/// `BorderStyle::Double`, which draws two parallel lines at ±1 on the
/// cross-axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Line {
    H { span: Span, y: i32 },
    V { x: i32, span: Span },
}

impl Line {
    /// Move the line by `d` perpendicular to its direction.
    pub fn offset_cross(self, d: i32) -> Self {
        match self {
            Line::H { span, y } => Line::H { span, y: y + d },
            Line::V { span, x } => Line::V { span, x: x + d },
        }
    }
}

/// Line length
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub from: i32,
    pub to: i32,
}

//  Shared axis - row-vs-column symmetry

/// Horizontal vs vertical axis.
///
/// Shared across viewport offset math (`cell_offset` dispatches on axis) and
/// header rect building (`Axis::header_rect`). Carries no payload - the
/// row/column index travels as a separate parameter so the same enum value
/// can be used across call sites that don't care about a specific index.
#[derive(Copy, Clone)]
pub(crate) enum Axis {
    Row,
    Column,
}

impl Axis {
    /// Rect that pins a header cell to the corresponding header strip.
    ///
    /// `along` is the position along the axis (top_y for rows, left_x for
    /// cols). `parallel_size` is the cell's extent along the axis (row
    /// height / col width). `header_thickness` is the perpendicular size
    /// of the header strip — `chrome.row_header_width` for rows (dynamic),
    /// `HEADER_ROW_HEIGHT` for cols (currently static).
    pub(crate) fn header_rect(
        self,
        along: i32,
        parallel_size: i32,
        header_thickness: i32,
    ) -> PixelRect {
        match self {
            Axis::Row => PixelRect {
                top_left: Point {
                    x: HEADER_OFFSET,
                    y: along,
                },
                width: header_thickness,
                height: parallel_size,
            },
            Axis::Column => PixelRect {
                top_left: Point {
                    x: along,
                    y: HEADER_OFFSET,
                },
                width: parallel_size,
                height: header_thickness,
            },
        }
    }

    /// Extent from the frame's prefix-sum snapshot — zero model access.
    pub(crate) fn frame_extent(self, frame: &Chrome, index: i32) -> i32 {
        match self {
            Axis::Row => frame.row_extent_at(index),
            Axis::Column => frame.col_extent_at(index),
        }
    }

    /// Pixel position where the header strip begins along this axis,
    /// offset by HEADER_OFFSET `0.5` for crisp integer-coordinate strokes.
    pub(crate) fn strip_start(self) -> i32 {
        match self {
            Axis::Row => HEADER_ROW_HEIGHT + HEADER_OFFSET,
            Axis::Column => HEADER_COL_WIDTH + HEADER_OFFSET,
        }
    }

    /// Visible scrollable band in this axis, derived from the frame's slot vecs.
    pub(crate) fn visible_band(self, frame: &Chrome) -> RangeInclusive<i32> {
        match self {
            Axis::Row => frame.top_row()..=frame.last_visible_row(),
            Axis::Column => frame.left_column()..=frame.last_visible_col(),
        }
    }

    /// Count of frozen cells along this axis (0 when nothing is frozen).
    pub(crate) fn frozen_count(self, frame: &Chrome) -> i32 {
        match self {
            Axis::Row => frame.frozen.rows,
            Axis::Column => frame.frozen.cols,
        }
    }

    /// Pixel origin where the scrollable strip for this axis begins.
    pub(crate) fn frozen_origin(self, frame: &Chrome) -> i32 {
        match self {
            Axis::Row => frame.frozen.offset.y,
            Axis::Column => frame.frozen.offset.x,
        }
    }

    /// Inclusive `(start, end)` of the user's selection along this axis,
    /// read from ironcalc's `SelectedView.range`
    pub(crate) fn selection_range(self, view_range: RCRange) -> (i32, i32) {
        let norm = view_range.normalized();
        match self {
            Axis::Row => (norm.r1, norm.r2),
            Axis::Column => (norm.c1, norm.c2),
        }
    }
}

/// Which edge of a cell rectangle is being stroked.
///
/// `line()` projects the edge onto a `PixelRect` to produce the
/// axis-aligned `Line` segment painted by `paint_border`.
#[derive(Copy, Clone)]
pub(crate) enum BorderEdge {
    Left,
    Top,
    Right,
    Bottom,
}

impl BorderEdge {
    /// The axis-aligned `Line` this edge would stroke on `rect`.
    pub fn line(self, rect: PixelRect) -> Line {
        let PixelRect {
            top_left: Point { x, y },
            width,
            height,
        } = rect;
        match self {
            BorderEdge::Left => Line::V {
                x,
                span: Span {
                    from: y,
                    to: y + height,
                },
            },
            BorderEdge::Top => Line::H {
                span: Span {
                    from: x,
                    to: x + width,
                },
                y,
            },
            BorderEdge::Right => Line::V {
                x: x + width,
                span: Span {
                    from: y,
                    to: y + height,
                },
            },
            BorderEdge::Bottom => Line::H {
                span: Span {
                    from: x,
                    to: x + width,
                },
                y: y + height,
            },
        }
    }
}
