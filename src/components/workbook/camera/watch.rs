//! Event ↔ source-range intersection. Conservative: variants that carry no
//! locality (or whose locality we can't prove disjoint) return true, so a
//! camera can over-repaint but never go stale.

use crate::coord::SheetRange;
use crate::events::{ContentEvent, FormatEvent};

fn overlaps(a: crate::coord::CellArea, b: crate::coord::CellArea) -> bool {
    a.r1 <= b.r2 && b.r1 <= a.r2 && a.c1 <= b.c2 && b.c1 <= a.c2
}

/// Returns `true` when the given content or format events could affect the
/// rendered appearance of `source`. Over-reports (returns true for
/// locality-unknown variants) — a false negative would leave a camera stale.
pub fn events_touch_source(
    source: SheetRange,
    content: &[ContentEvent],
    format: &[FormatEvent],
) -> bool {
    let area = source.area.normalized();

    let content_hit = content.iter().any(|ev| match ev {
        ContentEvent::CellChanged { address, .. } => {
            address.sheet == source.sheet && area.contains(address.row, address.column)
        }
        ContentEvent::FormulaChanged { address } => {
            address.sheet == source.sheet && area.contains(address.row, address.column)
        }
        ContentEvent::RangeChanged { sheet_area } => {
            sheet_area.sheet == source.sheet && overlaps(area, sheet_area.area.normalized())
        }
        // Recalculation can rewrite any cell value, including a formula *inside*
        // `source` that references an affected sheet we can't see from the range
        // alone. `affected_sheets` names where inputs changed, not every sheet
        // whose displayed values did — so we can't prove the source disjoint.
        // Honor the "never go stale" contract: any recalc is a hit.
        ContentEvent::CalculationUpdated { .. } => true,
        // Named-range edits don't carry a location; conservatively skip —
        // they affect formula text, not cell values visible in a snapshot.
        ContentEvent::NamedRangesChanged => false,
    });

    if content_hit {
        return true;
    }

    format.iter().any(|ev| match ev {
        FormatEvent::CellStyleChanged { address } => {
            address.sheet == source.sheet && area.contains(address.row, address.column)
        }
        FormatEvent::RangeStyleChanged { area: a } => {
            a.sheet == source.sheet && overlaps(area, a.area.normalized())
        }
        // LayoutChanged fires for row/col resizes. If no specific col/row is
        // given the resize is sheet-wide, so we conservatively hit. When one
        // is given, hit only if it falls inside the source rectangle.
        FormatEvent::LayoutChanged { sheet, col, row } => {
            if *sheet != source.sheet {
                return false;
            }
            let col_hit = col.is_none_or(|c| (area.c1..=area.c2).contains(&c));
            let row_hit = row.is_none_or(|r| (area.r1..=area.r2).contains(&r));
            col_hit || row_hit
        }
        // CF rules affect fill/font/border of cells in the source range.
        FormatEvent::ConditionalFormattingChanged { sheet } => *sheet == source.sheet,
        // Palette changes don't alter cell content or per-cell formatting;
        // theme resolution at extract-time already baked those in.
        FormatEvent::RecentColorsUpdated { .. } | FormatEvent::DocumentColorsChanged { .. } => {
            false
        }
    })
}
