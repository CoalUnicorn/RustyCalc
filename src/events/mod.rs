/*!
# Domain-driven Event System

Typed events representing actual changes in the spreadsheet domain.
Components subscribe to per-category `EventBus` signals and re-render
only when their category fires.

## Event Categories

- **Content**: Cell values, formulas, calculations
- **Format**: Visual styling, colors, layout
- **Structure**: Sheets, rows, columns
- **Navigation**: Selection, scrolling, editing state
- **Theme**: Appearance settings

## Usage

```rust
// Emit a typed event (via WorkbookState)
state.emit_event(SpreadsheetEvent::Format(
    FormatEvent::RangeStyleChanged { area: sa }
));

// Subscribe in an Effect (worksheet.rs pattern)
Effect::new(move |_| {
    let _content = state.events.content.get(); // registers dependency
    // ... render canvas
});
```
*/

mod bus;
mod content;
mod format;
mod navigation;
mod structure;
mod theme;

pub use bus::EventBus;
pub use content::ContentEvent;
pub use format::FormatEvent;
pub use navigation::NavigationEvent;
pub use structure::{Location, StructureEvent};
pub use theme::ThemeEvent;

/// Top-level fan-in wrapper. Lives in `mod.rs` so emit-sites import a single
/// path (`crate::events::SpreadsheetEvent`) regardless of category.
#[derive(Clone, PartialEq, Debug)]
pub enum SpreadsheetEvent {
    Content(ContentEvent),
    Format(FormatEvent),
    Structure(StructureEvent),
    Navigation(NavigationEvent),
    Theme(ThemeEvent),
}
