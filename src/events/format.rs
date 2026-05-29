//! Format-domain events: visual styling, colors, layout.

use crate::coord::{CellAddress, SheetRange};
use crate::model::CssColor;

#[derive(Clone, PartialEq, Debug)]
pub enum FormatEvent {
    #[allow(dead_code)]
    CellStyleChanged {
        address: CellAddress,
    },
    RangeStyleChanged {
        area: SheetRange,
    },
    LayoutChanged {
        sheet: u32,
        col: Option<i32>,
        row: Option<i32>,
    },
    RecentColorsUpdated {
        colors: Vec<CssColor>,
    },
    #[allow(dead_code)]
    DocumentColorsChanged {
        colors: Vec<CssColor>,
    },
    #[allow(dead_code)]
    ConditionalFormattingChanged {
        sheet: u32,
    },
}
