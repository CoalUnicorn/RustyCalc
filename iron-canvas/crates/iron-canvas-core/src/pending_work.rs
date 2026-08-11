//! The work algebra: one typed value describing everything queued for the
//! next paint attempt. `Orchestrator` owns exactly one `PendingWork`; every
//! setter marks intent on it and a paint attempt consumes it with one
//! `mem::take`, so successful consumption needs no clearing assignment and
//! layers hold no dirty state of their own.
//!
//! This replaced the side-by-side `pending_content` (`PaneRegionMask`) and
//! `pending_damage` (`CellDamage`) fields, the per-layer `PaintGate`, and
//! the `GridSignals` bitflags they were raised through.
//!
//! `PendingWork`, `ContentWork`, and `GeometryWork` stay crate-private —
//! their normalization and merge rules must not be bypassable by
//! constructing a value directly. `RowSpan` and the diagnostic `WorkFlags`
//! cross the existing core API boundary and are re-exported from the crate
//! root.

use crate::chrome::PaneRegionMask;

/// Inclusive row band in `RCRange` row coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowSpan {
    pub r1: i32,
    pub r2: i32,
}

impl RowSpan {
    /// Callers may hand spans in either order; a reversed span would
    /// survive merging and then silently intersect to nothing at paint
    /// time — stale pixels with content work already drained.
    fn normalized(self) -> Self {
        RowSpan {
            r1: self.r1.min(self.r2),
            r2: self.r1.max(self.r2),
        }
    }
}

/// Above this many disjoint bands a clipped repaint stops paying for
/// itself against one whole-pane walk.
// ponytail: fixed cap; tune only with profiler evidence.
pub(crate) const MAX_DAMAGE_SPANS: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum GeometryWork {
    #[default]
    Clean,
    Rebuild,
}

/// Row-band or pane-mask content work, carried inside `PendingWork`. Rows,
/// not cell rectangles, are the repaint unit: a row's shared top/bottom
/// border may be owned by (painted from) the row above or below it, and
/// cell text overflows horizontally into row neighbours unclipped, so a
/// rectangle clipped to just the changed cell risks leaving a stale
/// shared-edge stroke behind or clipping fresh overflow text either way
/// (`fingerprint::plan_pane_repaint`'s border-safety check exists for
/// exactly this). A full-width band is the smallest repaint unit that
/// cannot get either case wrong.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum ContentWork {
    #[default]
    Clean,
    Rows {
        sheet: u32,
        spans: Vec<RowSpan>,
    },
    Panes(PaneRegionMask),
}

impl ContentWork {
    /// The merge table pinned by the Stage 2 plan. `PendingWork`'s
    /// `mark_rows`, `mark_panes`, and `merge` all route through this one
    /// implementation instead of reproducing the rules:
    ///
    /// | existing | incoming | result |
    /// | --- | --- | --- |
    /// | `Clean` | anything | incoming |
    /// | `Rows(A)` | rows on `A` | normalized, merged spans |
    /// | `Rows(A)` | rows on another sheet | `Panes(ALL)` |
    /// | `Rows` | any pane work | `Panes(ALL)` |
    /// | `Panes(a)` | `Panes(b)` | `Panes(a \| b)` |
    /// | `Panes` | `Rows` | `Panes(ALL)`; row precision never returns |
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (ContentWork::Clean, incoming) => incoming,
            (existing, ContentWork::Clean) => existing,
            (
                ContentWork::Rows {
                    sheet: s1,
                    spans: mut existing,
                },
                ContentWork::Rows {
                    sheet: s2,
                    spans: incoming,
                },
            ) if s1 == s2 => {
                existing.extend(incoming);
                Self::normalize_rows(s1, existing)
            }
            // Guard above failed: same shape, different sheets. No
            // representation preserves both, so fall back whole-grid.
            (ContentWork::Rows { .. }, ContentWork::Rows { .. }) => {
                ContentWork::Panes(PaneRegionMask::ALL)
            }
            // Rows meeting an unscoped pane mask, either order: never
            // recover row precision once pane-level work has mixed in.
            (ContentWork::Rows { .. }, ContentWork::Panes(_))
            | (ContentWork::Panes(_), ContentWork::Rows { .. }) => {
                ContentWork::Panes(PaneRegionMask::ALL)
            }
            (ContentWork::Panes(a), ContentWork::Panes(b)) => ContentWork::Panes(a | b),
        }
    }

    /// Sort, merge overlapping/adjacent bands, and degrade to
    /// `Panes(ALL)` once the disjoint count stops paying for itself
    /// against one whole-pane walk.
    fn normalize_rows(sheet: u32, mut spans: Vec<RowSpan>) -> Self {
        spans.sort_by_key(|span| span.r1);
        let mut merged: Vec<RowSpan> = Vec::with_capacity(spans.len());
        for span in spans {
            match merged.last_mut() {
                // +1: adjacent bands repaint as one strip.
                Some(last) if span.r1 <= last.r2 + 1 => last.r2 = last.r2.max(span.r2),
                _ => merged.push(span),
            }
        }
        if merged.len() > MAX_DAMAGE_SPANS {
            ContentWork::Panes(PaneRegionMask::ALL)
        } else {
            ContentWork::Rows {
                sheet,
                spans: merged,
            }
        }
    }
}

/// One typed value describing everything queued for the next paint
/// attempt: geometry rebuild, view movement, content damage, and overlay
/// repaint. Fields stay private — callers only see intent through the
/// `mark_*` / `has_*` helpers below, so every producer routes through the
/// same normalization and merge rules instead of reimplementing them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PendingWork {
    geometry: GeometryWork,
    view: bool,
    content: ContentWork,
    overlay: bool,
}

impl PendingWork {
    pub(crate) fn mark_geometry(&mut self) {
        self.geometry = GeometryWork::Rebuild;
    }

    pub(crate) fn mark_view(&mut self) {
        self.view = true;
    }

    pub(crate) fn mark_overlay(&mut self) {
        self.overlay = true;
    }

    pub(crate) fn mark_rows(&mut self, sheet: u32, span: RowSpan) {
        let incoming = ContentWork::Rows {
            sheet,
            spans: vec![span.normalized()],
        };
        self.content = std::mem::take(&mut self.content).merge(incoming);
    }

    /// An empty mask still means pane content changed somewhere unknown;
    /// normalizing it to `ALL` keeps today's conservative rule that a
    /// content notification can never read back as clean just because its
    /// mask happened to be empty.
    pub(crate) fn mark_panes(&mut self, mask: PaneRegionMask) {
        let mask = if mask.is_empty() {
            PaneRegionMask::ALL
        } else {
            mask
        };
        self.content = std::mem::take(&mut self.content).merge(ContentWork::Panes(mask));
    }

    pub(crate) fn has_geometry(&self) -> bool {
        self.geometry == GeometryWork::Rebuild
    }

    pub(crate) fn has_view(&self) -> bool {
        self.view
    }

    pub(crate) fn has_content(&self) -> bool {
        self.content != ContentWork::Clean
    }

    pub(crate) fn has_overlay(&self) -> bool {
        self.overlay
    }

    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub(crate) fn content(&self) -> &ContentWork {
        &self.content
    }

    /// Folds `other` into `self`. Associative, commutative, and idempotent
    /// for the representative values rAF coalescing produces: geometry,
    /// view, and overlay escalate by OR; content follows
    /// `ContentWork::merge`.
    pub(crate) fn merge(&mut self, other: Self) {
        if other.geometry == GeometryWork::Rebuild {
            self.geometry = GeometryWork::Rebuild;
        }
        self.view |= other.view;
        self.overlay |= other.overlay;
        self.content = std::mem::take(&mut self.content).merge(other.content);
    }

    /// Diagnostic-only snapshot of which categories carry work. Never
    /// mutated as queued state, and must not gain a `grid_dirty()`-style
    /// convenience predicate — regime eligibility reads the typed fields
    /// above directly, with the view-specific `Viewport`-then-`Overlay`
    /// fallback left to the dispatcher.
    pub(crate) fn flags(&self) -> WorkFlags {
        let mut flags = WorkFlags::empty();
        if self.has_view() {
            flags |= WorkFlags::VIEW;
        }
        if self.has_content() {
            flags |= WorkFlags::CONTENT;
        }
        if self.has_geometry() {
            flags |= WorkFlags::GEOMETRY;
        }
        if self.has_overlay() {
            flags |= WorkFlags::OVERLAY;
        }
        flags
    }
}

bitflags::bitflags! {
    /// Diagnostic projection of a `PendingWork` snapshot — never queued
    /// state itself. Bit values are pinned to the recorder work-byte
    /// layout inherited from `GridSignals`; renumbering them would corrupt
    /// recorded fixtures.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct WorkFlags: u8 {
        const VIEW     = 0b0001;
        const CONTENT  = 0b0010;
        const GEOMETRY = 0b0100;
        const OVERLAY  = 0b1000;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(mut a: PendingWork, b: PendingWork) -> PendingWork {
        a.merge(b);
        a
    }

    fn rows(sheet: u32, r1: i32, r2: i32) -> PendingWork {
        let mut work = PendingWork::default();
        work.mark_rows(sheet, RowSpan { r1, r2 });
        work
    }

    // `PendingWork::default()` is the merge identity in both positions —
    // the retry contract's `mem::take(&mut self.pending)` relies on an
    // empty value never adding spurious work when folded back in.
    #[test]
    fn merge_identity_holds_both_directions() {
        let mut work = PendingWork::default();
        work.mark_geometry();
        work.mark_view();
        work.mark_overlay();
        work.mark_rows(1, RowSpan { r1: 3, r2: 5 });

        assert_eq!(merged(work.clone(), PendingWork::default()), work);
        assert_eq!(merged(PendingWork::default(), work.clone()), work);
    }

    // Folding identical work back into itself — the shape of a repeated
    // rAF raise before the frame drains — must not escalate any further.
    #[test]
    fn merge_is_idempotent() {
        let mut work = PendingWork::default();
        work.mark_view();
        work.mark_rows(1, RowSpan { r1: 3, r2: 5 });

        assert_eq!(merged(work.clone(), work.clone()), work);
    }

    // Two independent raises (geometry here, overlay-plus-content there)
    // must combine to the same value regardless of which one the
    // scheduler observes first.
    #[test]
    fn merge_is_commutative_for_a_representative_pair() {
        let mut a = PendingWork::default();
        a.mark_geometry();
        a.mark_rows(1, RowSpan { r1: 0, r2: 2 });

        let mut b = PendingWork::default();
        b.mark_overlay();
        b.mark_rows(1, RowSpan { r1: 5, r2: 7 });

        assert_eq!(merged(a.clone(), b.clone()), merged(b, a));
    }

    // Grouping must not matter either. Cross-sheet rows force content to
    // degrade; both association orders must land on the same `Panes(ALL)`
    // result, exercising both the Rows/Rows-cross-sheet arm and the
    // Rows/Panes arm along the way.
    #[test]
    fn merge_is_associative_for_a_representative_triple() {
        let a = rows(1, 0, 2);
        let b = rows(2, 5, 7);
        let c = rows(1, 10, 12);

        let left = merged(merged(a.clone(), b.clone()), c.clone());
        let right = merged(a, merged(b, c));

        assert_eq!(left, right);
        assert_eq!(*left.content(), ContentWork::Panes(PaneRegionMask::ALL));
    }

    // A reversed span (r1 > r2) must normalize before it ever reaches
    // `ContentWork`, or it would merge fine today and then intersect to
    // nothing at paint time.
    #[test]
    fn mark_rows_normalizes_a_reversed_span() {
        let mut work = PendingWork::default();
        work.mark_rows(1, RowSpan { r1: 10, r2: 4 });

        assert_eq!(
            *work.content(),
            ContentWork::Rows {
                sheet: 1,
                spans: vec![RowSpan { r1: 4, r2: 10 }],
            }
        );
    }

    // Touching bands (next r1 == last r2 + 1) repaint as one strip rather
    // than as two separately-clipped rectangles.
    #[test]
    fn adjacent_spans_merge_into_one_band() {
        let mut work = PendingWork::default();
        work.mark_rows(1, RowSpan { r1: 0, r2: 3 });
        work.mark_rows(1, RowSpan { r1: 4, r2: 6 });

        assert_eq!(
            *work.content(),
            ContentWork::Rows {
                sheet: 1,
                spans: vec![RowSpan { r1: 0, r2: 6 }],
            }
        );
    }

    // A second sheet's rows can't be expressed alongside the first
    // sheet's spans, so the pair degrades to the whole-grid pane mask.
    #[test]
    fn cross_sheet_rows_degrade_to_all_panes() {
        let mut work = PendingWork::default();
        work.mark_rows(1, RowSpan { r1: 0, r2: 2 });
        work.mark_rows(2, RowSpan { r1: 0, r2: 2 });

        assert_eq!(*work.content(), ContentWork::Panes(PaneRegionMask::ALL));
    }

    // More disjoint bands than `MAX_DAMAGE_SPANS` stops paying for itself
    // against one whole-pane walk.
    #[test]
    fn over_cap_span_count_degrades_to_all_panes() {
        let mut work = PendingWork::default();
        for i in 0..=(MAX_DAMAGE_SPANS as i32) {
            let r = i * 3;
            work.mark_rows(1, RowSpan { r1: r, r2: r });
        }

        assert_eq!(*work.content(), ContentWork::Panes(PaneRegionMask::ALL));
    }

    // Once pane-level work has mixed in, row precision never comes back
    // — regardless of which one is marked first.
    #[test]
    fn rows_then_panes_degrades_to_all_panes() {
        let mut work = PendingWork::default();
        work.mark_rows(1, RowSpan { r1: 0, r2: 2 });
        work.mark_panes(PaneRegionMask::TOP_LEFT);

        assert_eq!(*work.content(), ContentWork::Panes(PaneRegionMask::ALL));
    }

    #[test]
    fn panes_then_rows_degrades_to_all_panes() {
        let mut work = PendingWork::default();
        work.mark_panes(PaneRegionMask::TOP_LEFT);
        work.mark_rows(1, RowSpan { r1: 0, r2: 2 });

        assert_eq!(*work.content(), ContentWork::Panes(PaneRegionMask::ALL));
    }

    // An empty mask still means "something changed, region unknown" — it
    // must never read back as clean.
    #[test]
    fn empty_mask_normalizes_to_all_panes() {
        let mut work = PendingWork::default();
        work.mark_panes(PaneRegionMask::EMPTY);

        assert_eq!(*work.content(), ContentWork::Panes(PaneRegionMask::ALL));
        assert!(work.has_content());
    }

    // Bit values are pinned to the recorder work-byte layout
    // inherited from `GridSignals`; a silent renumber would corrupt
    // recorded fixtures.
    #[test]
    fn work_flags_bit_layout_is_pinned() {
        assert_eq!(WorkFlags::VIEW.bits(), 0b0001);
        assert_eq!(WorkFlags::CONTENT.bits(), 0b0010);
        assert_eq!(WorkFlags::GEOMETRY.bits(), 0b0100);
        assert_eq!(WorkFlags::OVERLAY.bits(), 0b1000);
    }
}
