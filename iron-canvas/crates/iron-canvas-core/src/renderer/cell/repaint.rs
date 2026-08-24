use crate::chrome::Chrome;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::geometry::slot::AxisSlot;
use crate::types::coord::RCRange;

pub(crate) const CELL_REPAINT_PAD_PX: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CellRepaintEnvelope {
    NoPixels,
    UnalignedDpr,
    Visible {
        clip: PixelRect,
        sources: [Option<RCRange>; 4],
    },
}

pub(crate) fn build_cell_repaint_envelope(
    frame: &Chrome,
    changed_cells: &[RCRange],
) -> CellRepaintEnvelope {
    let (width, height) = frame.canvas_size.to_logical_extent();
    let canvas = PixelRect {
        top_left: Point { x: 0, y: 0 },
        width,
        height,
    };
    let changed_bounds = changed_cells
        .iter()
        .filter_map(|changed| {
            let changed = changed.normalized();
            frame
                .cell_rect(changed.r1, changed.c1)
                .filter(positive_area)
        })
        .map(grow_pixel_rect)
        .reduce(PixelRect::bounding_union);
    let Some(bounds) = changed_bounds else {
        return CellRepaintEnvelope::NoPixels;
    };
    let Some(aligned) = align_outward_to_backing(bounds, frame.dpr) else {
        return CellRepaintEnvelope::UnalignedDpr;
    };
    let Some(clip) = aligned.intersection(canvas) else {
        return CellRepaintEnvelope::NoPixels;
    };

    let mut sources = [None; 4];
    for segment in frame.grid_layout().segments() {
        let region = segment.region();
        sources[region.index()] = contributor_source(region.rows(frame), region.cols(frame), clip);
    }

    CellRepaintEnvelope::Visible { clip, sources }
}

fn contributor_source<R: AxisSlot, C: AxisSlot>(
    rows: &[R],
    cols: &[C],
    clip: PixelRect,
) -> Option<RCRange> {
    let (r1, r2) = contributing_slot_ids(rows, clip.top(), clip.bottom())?;
    let (c1, c2) = contributing_slot_ids(cols, clip.left(), clip.right())?;
    Some(RCRange { r1, c1, r2, c2 })
}

fn contributing_slot_ids<S: AxisSlot>(
    slots: &[S],
    clip_start: i32,
    clip_end: i32,
) -> Option<(i32, i32)> {
    let first =
        slots.partition_point(|slot| slot.end().saturating_add(CELL_REPAINT_PAD_PX) <= clip_start);
    let end =
        slots.partition_point(|slot| slot.start().saturating_sub(CELL_REPAINT_PAD_PX) < clip_end);
    let candidates = slots.get(first..end)?;
    let first_id = candidates.iter().find(|slot| slot.extent() > 0)?.id();
    let last_id = candidates.iter().rfind(|slot| slot.extent() > 0)?.id();
    Some((first_id, last_id))
}

fn positive_area(rect: &PixelRect) -> bool {
    rect.width > 0 && rect.height > 0
}

fn grow_pixel_rect(rect: PixelRect) -> PixelRect {
    rect.inset(-CELL_REPAINT_PAD_PX, -CELL_REPAINT_PAD_PX)
}

fn align_outward_to_backing(rect: PixelRect, dpr: f64) -> Option<PixelRect> {
    if !dpr.is_finite() || dpr <= 0.0 {
        return None;
    }
    let left = aligned_edge(rect.left(), dpr, -1)?;
    let top = aligned_edge(rect.top(), dpr, -1)?;
    let right = aligned_edge(rect.right(), dpr, 1)?;
    let bottom = aligned_edge(rect.bottom(), dpr, 1)?;
    Some(PixelRect {
        top_left: Point { x: left, y: top },
        width: right - left,
        height: bottom - top,
    })
}

fn aligned_edge(start: i32, dpr: f64, direction: i32) -> Option<i32> {
    const MAX_OUTSET_CSS_PX: i32 = 32;
    (0..=MAX_OUTSET_CSS_PX)
        .map(|offset| start.saturating_add(direction.saturating_mul(offset)))
        .find(|&edge| {
            let backing = f64::from(edge) * dpr;
            (backing - backing.round()).abs() <= 1.0e-9
        })
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::FrameInputs;
    use crate::chrome::{FramePath, PaneRegion};
    use crate::geometry::CanvasSize;
    use crate::geometry::slot::RowSlot;
    use crate::model_adapter::{CanvasModel, CanvasView, CellContentQuery};
    use crate::style::{CellKind, CellStyle};
    use crate::theme::CanvasTheme;
    use crate::types::fetched::Fetched;

    struct Model;

    impl CellContentQuery for Model {
        fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Fetched<CellStyle> {
            Fetched::Absent
        }

        fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Fetched<CellKind> {
            Fetched::Absent
        }

        fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Fetched<String> {
            Fetched::Absent
        }
    }

    impl CanvasModel for Model {
        fn get_selected_sheet(&self) -> Option<u32> {
            Some(0)
        }

        fn get_selected_view(&self) -> Option<CanvasView> {
            Some(CanvasView {
                sheet: 0,
                row: 1,
                column: 1,
                selection: RCRange::from_cell(1, 1),
                top_row: 1,
                left_column: 1,
            })
        }

        fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
            Some(0)
        }

        fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
            Some(0)
        }

        fn get_row_height(&self, _: u32, row: i32) -> Option<f64> {
            Some(if row == 2 { 0.0 } else { 20.0 })
        }

        fn get_column_width(&self, _: u32, column: i32) -> Option<f64> {
            Some(if column == 2 { 0.0 } else { 60.0 })
        }

        fn get_show_grid_lines(&self, _: u32) -> Option<bool> {
            Some(true)
        }
    }

    fn frame() -> Chrome {
        frame_at_dpr(1.0)
    }

    fn frame_at_dpr(dpr: f64) -> Chrome {
        let model = Model;
        let inputs = FrameInputs::capture(
            &model,
            CanvasSize { w: 320.0, h: 180.0 },
            dpr,
            Rc::new(CanvasTheme::light()),
            0,
        )
        .expect("test model provides complete frame inputs");
        Chrome::next(None, &model, &inputs, FramePath::Fresh)
    }

    #[test]
    fn interior_cell_builds_one_two_pixel_clip_and_pixel_adjacent_source() {
        let frame = frame();
        let rect = frame
            .cell_rect(3, 3)
            .expect("the test cell is visible and addressable");
        let CellRepaintEnvelope::Visible { clip, sources } =
            build_cell_repaint_envelope(&frame, &[RCRange::from_cell(3, 3)])
        else {
            panic!("a visible cell must produce an envelope");
        };
        assert_eq!(clip, rect.inset(-2, -2));
        assert_eq!(
            sources[PaneRegion::BottomRight.index()],
            Some(RCRange {
                r1: 1,
                c1: 1,
                r2: 4,
                c2: 4,
            })
        );
    }

    #[test]
    fn hidden_changed_cell_produces_no_pixels() {
        assert_eq!(
            build_cell_repaint_envelope(&frame(), &[RCRange::from_cell(2, 2)]),
            CellRepaintEnvelope::NoPixels
        );
    }

    #[test]
    fn pixel_bounds_cross_hidden_address_gap_to_adjacent_visible_slot() {
        let mut rows = vec![RowSlot {
            row: 1,
            top: 0,
            height: 20,
        }];
        rows.extend((2..=100).map(|row| RowSlot {
            row,
            top: 20,
            height: 0,
        }));
        rows.extend([
            RowSlot {
                row: 101,
                top: 20,
                height: 20,
            },
            RowSlot {
                row: 102,
                top: 40,
                height: 20,
            },
        ]);

        assert_eq!(contributing_slot_ids(&rows, 18, 42), Some((1, 102)));
    }

    #[test]
    fn pixel_bounded_slot_ids_match_per_slot_intersections() {
        let rows = [
            RowSlot {
                row: 1,
                top: 0,
                height: 20,
            },
            RowSlot {
                row: 2,
                top: 20,
                height: 0,
            },
            RowSlot {
                row: 3,
                top: 20,
                height: 15,
            },
            RowSlot {
                row: 4,
                top: 35,
                height: 25,
            },
        ];

        for clip_start in -3..=63 {
            for clip_end in (clip_start + 1)..=(clip_start + 8) {
                let mut contributors = rows.iter().filter(|slot| {
                    slot.extent() > 0
                        && slot.end().saturating_add(CELL_REPAINT_PAD_PX) > clip_start
                        && slot.start().saturating_sub(CELL_REPAINT_PAD_PX) < clip_end
                });
                let expected = contributors
                    .clone()
                    .next()
                    .zip(contributors.next_back())
                    .map(|(first, last)| (first.id(), last.id()));

                assert_eq!(
                    contributing_slot_ids(&rows, clip_start, clip_end),
                    expected,
                    "clip {clip_start}..{clip_end}"
                );
            }
        }
    }

    #[test]
    fn fractional_dpr_clip_edges_align_outward_to_backing_pixels() {
        let frame = frame_at_dpr(1.25);
        let CellRepaintEnvelope::Visible { clip, .. } =
            build_cell_repaint_envelope(&frame, &[RCRange::from_cell(3, 3)])
        else {
            panic!("a common fractional DPR must produce an aligned envelope");
        };
        for edge in [clip.left(), clip.top(), clip.right(), clip.bottom()] {
            let backing = f64::from(edge) * frame.dpr;
            assert!((backing - backing.round()).abs() <= 1.0e-9);
        }
    }

    #[test]
    fn unrepresentable_fractional_dpr_requests_conservative_fallback() {
        assert_eq!(
            build_cell_repaint_envelope(
                &frame_at_dpr(std::f64::consts::SQRT_2),
                &[RCRange::from_cell(3, 3)]
            ),
            CellRepaintEnvelope::UnalignedDpr
        );
    }
}
