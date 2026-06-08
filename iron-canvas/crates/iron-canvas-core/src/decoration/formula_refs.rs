//! Per-reference formula highlights for the in-edit cell. One color per
//! `color_idx` (mod the palette), off-sheet refs silently skipped.

use std::borrow::Cow;

use crate::chrome::Chrome;
use crate::decoration::Layer;
use crate::geometry::constants::{DASHED_BORDER_WIDTH, REF_HANDLE_HIT_PAD_PX};
use crate::geometry::pixel_rect::PixelRect;
use crate::painter::{GroupClass, PaintColor, Painter};
use crate::theme::{FORMULA_REF_COLORS, FORMULA_REF_TINTS};
use crate::types::coord::{FormulaRef, FormulaRefKind, RCRange};
use crate::types::ui::{HitTest, RectCorner, RefZone, Side};

#[derive(Default)]
pub struct FormulaRefsLayer {
    pub refs: Vec<FormulaRef>,
}

impl Layer for FormulaRefsLayer {
    fn group(&self) -> GroupClass {
        GroupClass::FormulaRefs
    }

    fn paint(&self, frame: &Chrome, painter: &dyn Painter) {
        if self.refs.is_empty() {
            return;
        }
        for fr in &self.refs {
            if fr.sheet_area.sheet != frame.sheet {
                continue;
            }
            let Some(b) = frame.range_rect(fr.sheet_area.range.normalized()) else {
                continue;
            };
            let idx = fr.color_idx % FORMULA_REF_COLORS.len();
            painter.rect_fill(
                b,
                PaintColor::from_theme_str(&Cow::Borrowed(FORMULA_REF_TINTS[idx])),
            );
            painter.rect_dashed(
                b,
                PaintColor::Static(FORMULA_REF_COLORS[idx]),
                f64::from(DASHED_BORDER_WIDTH),
            );
        }
    }

    fn hit_test(
        &self,
        frame: &Chrome,
        _selection_range: RCRange,
        x: i32,
        y: i32,
    ) -> Option<HitTest> {
        // Reverse paint order: the last-painted ref claims overlapping
        // hits. Filters mirror `paint`: non-Direct kinds and off-sheet
        // refs never claim a hit.
        for (ref_idx, fr) in self.refs.iter().enumerate().rev() {
            if !matches!(fr.kind, FormulaRefKind::Direct) {
                continue;
            }
            if fr.sheet_area.sheet != frame.sheet {
                continue;
            }
            let Some(rect) = frame.range_rect(fr.sheet_area.range.normalized()) else {
                continue;
            };
            if let Some(zone) = classify_ref_zone(rect, x, y, REF_HANDLE_HIT_PAD_PX) {
                // grab_row/grab_column are the cell the pointer is over right
                // now. `None` means the pointer sits over chrome or off-
                // grid; treat that as no hit even if the zone classified.
                let Some(grab_row) = frame.pane_set.rows.pixel_to_id(y) else {
                    continue;
                };
                let Some(grab_column) = frame.pane_set.cols.pixel_to_id(x) else {
                    continue;
                };
                return Some(HitTest::FormulaRef {
                    ref_idx,
                    zone,
                    grab_row,
                    grab_column,
                });
            }
        }
        None
    }
}

/// Classify a pointer against a formula-ref rectangle. Precedence:
/// `Corner` (within `pad` of two intersecting edges) > `Edge` (within
/// `pad` of one edge) > `Body` (inside, away from edges) > `None`
/// (outside the padded rect). `rect` is assumed normalized.
fn classify_ref_zone(rect: PixelRect, x: i32, y: i32, pad: i32) -> Option<RefZone> {
    let l = rect.top_left.x;
    let t = rect.top_left.y;
    let r = rect.right();
    let b = rect.bottom();

    if x < l - pad || x > r + pad || y < t - pad || y > b + pad {
        return None;
    }

    let near_left = (x - l).abs() <= pad;
    let near_right = (x - r).abs() <= pad;
    let near_top = (y - t).abs() <= pad;
    let near_bottom = (y - b).abs() <= pad;

    let corner = match (near_top, near_right, near_bottom, near_left) {
        (true, _, _, true) => Some(RectCorner::TopLeft),
        (true, true, _, _) => Some(RectCorner::TopRight),
        (_, _, true, true) => Some(RectCorner::BottomLeft),
        (_, true, true, _) => Some(RectCorner::BottomRight),
        _ => None,
    };
    if let Some(c) = corner {
        return Some(RefZone::Corner(c));
    }

    if near_top {
        return Some(RefZone::Edge(Side::Top));
    }
    if near_bottom {
        return Some(RefZone::Edge(Side::Bottom));
    }
    if near_left {
        return Some(RefZone::Edge(Side::Left));
    }
    if near_right {
        return Some(RefZone::Edge(Side::Right));
    }

    Some(RefZone::Body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::prim::Point;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> PixelRect {
        PixelRect {
            top_left: Point { x, y },
            width: w,
            height: h,
        }
    }

    #[test]
    fn classify_body_inside_rect() {
        assert_eq!(
            classify_ref_zone(rect(100, 100, 200, 80), 200, 140, 8),
            Some(RefZone::Body)
        );
    }

    #[test]
    fn classify_corner_top_left() {
        assert_eq!(
            classify_ref_zone(rect(100, 100, 200, 80), 102, 103, 8),
            Some(RefZone::Corner(RectCorner::TopLeft))
        );
    }

    #[test]
    fn classify_corner_bottom_right() {
        // rect: x=100, y=100, w=200, h=80 — right=300, bottom=180.
        assert_eq!(
            classify_ref_zone(rect(100, 100, 200, 80), 297, 178, 8),
            Some(RefZone::Corner(RectCorner::BottomRight))
        );
    }

    #[test]
    fn classify_edge_right() {
        // x near right=300, y=140 interior (far from top=100 / bottom=180).
        assert_eq!(
            classify_ref_zone(rect(100, 100, 200, 80), 298, 140, 8),
            Some(RefZone::Edge(Side::Right))
        );
    }

    #[test]
    fn classify_outside_rect_returns_none() {
        let r = rect(100, 100, 200, 80);
        assert_eq!(classify_ref_zone(r, 50, 50, 8), None);
        assert_eq!(classify_ref_zone(r, 400, 200, 8), None);
    }
}
