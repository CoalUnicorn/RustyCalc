use crate::{
    geometry::{constants::HEADER_OFFSET, pixel_rect::PixelRect},
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
    ///
    /// # Examples
    ///
    /// ```
    /// use iron_canvas_core::{Line, Span};
    /// let h = Line::H { span: Span { from: 0, to: 100 }, y: 10 };
    /// // H runs horizontally, so cross-axis is y.
    /// assert_eq!(
    ///     h.offset_cross(5),
    ///     Line::H { span: Span { from: 0, to: 100 }, y: 15 },
    /// );
    /// ```
    pub fn offset_cross(self, d: i32) -> Self {
        match self {
            Line::H { span, y } => Line::H { span, y: y + d },
            Line::V { span, x } => Line::V { span, x: x + d },
        }
    }
}

/// Endpoints of an axis-aligned line. The line covers `from` through `to`;
/// `to - from` is its pixel length.
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
pub enum Axis {
    Row,
    Column,
}

impl Axis {
    /// Rect that pins a header cell to the corresponding header strip.
    ///
    /// `along` is the position along the axis (top_y for rows, left_x for
    /// cols). `parallel_size` is the cell's extent along the axis (row
    /// height / col width). `header_thickness` is the perpendicular size
    /// of the header strip — `chrome.row_header_thickness` for rows (dynamic),
    /// `HEADER_ROW_HEIGHT` for cols (currently static).
    pub fn header_rect(
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

    /// Inclusive `(start, end)` of the user's selection along this axis,
    /// read from ironcalc's `SelectedView.range`
    pub fn selection_range(self, view_range: RCRange) -> (i32, i32) {
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
pub enum BorderEdge {
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
