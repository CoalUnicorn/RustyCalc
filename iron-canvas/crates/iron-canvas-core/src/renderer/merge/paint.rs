//! Merge paint: fill, perimeter, decoration, then text per fragment.

use crate::chrome::Chrome;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Point, Side};
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::renderer::cell::cf::CfDecorationPaint;
use crate::renderer::cell::text::TextPaint;

use super::geometry::translate;
use super::{BorderRun, MergeFragment, PreparedMerge};

impl<P: Painter> RendererCore<P> {
    /// Paint one prepared merge: fill, perimeter, decoration, then text laid out
    /// once and translated into each fragment's transform.
    pub(crate) fn paint_prepared_merge(&self, frame: &Chrome, merge: &PreparedMerge) {
        let theme = &frame.theme;
        let decoration = merge
            .decoration
            .clone()
            .and_then(|deco| CfDecorationPaint::resolve(deco, &self.color_intern));
        for fragment in &merge.fragments {
            self.paint_merge_fragment(frame, merge, fragment, decoration.as_ref());
        }
        if merge.fragments.is_empty() {
            return;
        }
        // Same text policy as the per-cell pass: a decoration on the anchor
        // hides the value or reserves a left band for its icon.
        let (hide_value, reserved_left) = match decoration.as_ref() {
            Some(deco) if deco.hides_value() => (true, 0),
            Some(deco) => (false, deco.reserved_left(merge.logical_rect)),
            None => (false, 0),
        };
        let mut text_lines = self.frame_cache.text_lines.take();
        if !hide_value
            && let Some(text) = TextPaint::resolve_into(
                self,
                merge.logical_rect,
                merge.logical_rect,
                reserved_left,
                &merge.style,
                merge.value.clone(),
                merge.cell_type,
                merge.link.as_deref(),
                &mut text_lines,
            )
        {
            let origin = merge.logical_rect;
            for fragment in &merge.fragments {
                // Start/End alignment uses `anchor`. Translate the anchor,
                // clip, and line centres into the same fragment coordinates.
                let mut fragment_text = text.clone();
                fragment_text.clip = translate(origin, fragment.logical_rect, text.clip);
                fragment_text.anchor = translate(origin, fragment.logical_rect, text.anchor);
                // Lay the text out once in merge-local coordinates, then
                // translate it into this fragment's own transform. Without
                // the translation a merge crossing the freeze boundary (or
                // the address gap after it) paints the frozen anchor's
                // absolute position under the scrolled fragment's clip —
                // right-aligned text lands past the clip and disappears.
                let dx = fragment.logical_rect.left() - origin.left();
                let dy = fragment.logical_rect.top() - origin.top();
                for line in text_lines.iter_mut() {
                    line.center_x += f64::from(dx);
                    line.center_y += f64::from(dy);
                }
                self.painter.push_clip(fragment.rect);
                self.paint_text(&fragment_text, theme, &text_lines);
                self.painter.pop_clip();
                for line in text_lines.iter_mut() {
                    line.center_x -= f64::from(dx);
                    line.center_y -= f64::from(dy);
                }
            }
        }
        self.frame_cache.text_lines.set(text_lines);
    }

    /// Paint every prepared merge. The last step of the grid paint, inside
    /// `GroupClass::Cells`, after every segment's cells.
    pub(crate) fn paint_merges(&self, frame: &Chrome, merges: &[PreparedMerge]) {
        for merge in merges {
            self.paint_prepared_merge(frame, merge);
        }
    }

    fn paint_merge_fragment(
        &self,
        frame: &Chrome,
        merge: &PreparedMerge,
        fragment: &MergeFragment,
        decoration: Option<&CfDecorationPaint>,
    ) {
        let theme = &frame.theme;
        // Same fill branch as `paint_bg`: a per-cell override or the theme
        // background.
        let color = match merge.style.fill_color.as_deref() {
            Some(c) => PaintColor::Borrowed(c),
            None => PaintColor::from_theme_str(&theme.cell_bg),
        };
        self.painter.rect_fill(fragment.rect, color);

        // The perimeter runs, gated on `fragment.sides` so a viewport-clipped
        // or freeze-clipped edge paints no stroke.
        self.paint_merge_side(frame, fragment, Side::Top, 1, &merge.borders.top);
        self.paint_merge_side(frame, fragment, Side::Bottom, 3, &merge.borders.bottom);
        self.paint_merge_side(frame, fragment, Side::Left, 0, &merge.borders.left);
        self.paint_merge_side(frame, fragment, Side::Right, 2, &merge.borders.right);

        if let Some(decoration) = decoration {
            decoration.paint(&*self.painter, fragment.rect);
        }
    }

    /// Paint the fragment's intersecting part of every run on one side. A run
    /// shorter than the fragment paints only its own span, and a run reaching
    /// beyond the fragment is clamped to the fragment's own rect — so two
    /// fragments of one merge never paint the same pixels twice.
    fn paint_merge_side(
        &self,
        frame: &Chrome,
        fragment: &MergeFragment,
        side: Side,
        side_index: usize,
        runs: &[BorderRun],
    ) {
        if !fragment.sides.get(side_index).copied().unwrap_or(false) {
            return;
        }
        let rect = fragment.rect;
        let horizontal = matches!(side, Side::Top | Side::Bottom);
        for run in runs {
            let subrect = if horizontal {
                let start = frame.pane_set.col_to_x(run.start);
                let end = frame.pane_set.col_to_x(run.end) + frame.pane_set.col_extent_at(run.end);
                let left = start.max(rect.left());
                let right = end.min(rect.right());
                if left >= right {
                    continue;
                }
                PixelRect {
                    top_left: Point {
                        x: left,
                        y: rect.top(),
                    },
                    width: right - left,
                    height: rect.height,
                }
            } else {
                let start = frame.pane_set.row_to_y(run.start);
                let end = frame.pane_set.row_to_y(run.end) + frame.pane_set.row_extent_at(run.end);
                let top = start.max(rect.top());
                let bottom = end.min(rect.bottom());
                if top >= bottom {
                    continue;
                }
                PixelRect {
                    top_left: Point {
                        x: rect.left(),
                        y: top,
                    },
                    width: rect.width,
                    height: bottom - top,
                }
            };
            self.paint_border(side, subrect, &run.paint);
        }
    }
}
