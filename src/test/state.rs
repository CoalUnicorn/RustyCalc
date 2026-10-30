use crate::Owner;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn status_initializes_to_none() {
    let owner = Owner::new();
    owner.with(|| {
        let state = crate::state::WorkbookState::new(crate::events::EventBus::new());
        assert_eq!(state.status.get_untracked(), None);
    });
}

/// The metadata epoch advances on content, format, structure, and theme
/// events (any of which can change a link or a merge list) and stays put on
/// navigation alone. The canvas reads it to decide whether its validated
/// link/merge snapshot is still current, so a navigation-only repaint must
/// not invalidate it.
#[wasm_bindgen_test]
fn metadata_epoch_advances_on_metadata_events_only() {
    use crate::events::{
        ContentEvent, FormatEvent, NavigationEvent, SpreadsheetEvent, StructureEvent, ThemeEvent,
    };
    use leptos::prelude::GetValue;

    let owner = Owner::new();
    owner.with(|| {
        let bus = crate::events::EventBus::new();
        let epoch = || bus.metadata_seq.get_value();

        assert_eq!(epoch(), 0, "a fresh bus starts at zero");

        bus.emit_event(SpreadsheetEvent::Navigation(
            NavigationEvent::SelectionChanged {
                address: crate::coord::CellAddress {
                    sheet: 0,
                    row: 1,
                    column: 1,
                },
            },
        ));
        assert_eq!(
            epoch(),
            0,
            "navigation cannot change a link or a merge list"
        );

        bus.emit_event(SpreadsheetEvent::Content(
            ContentEvent::CalculationUpdated {
                affected_sheets: vec![0],
            },
        ));
        assert_eq!(epoch(), 1, "content can change a link list");

        bus.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
            sheet: 0,
            col: None,
            row: None,
        }));
        assert_eq!(epoch(), 2, "format can change a link or a merge list");

        bus.emit_event(SpreadsheetEvent::Structure(
            StructureEvent::MergedCellsChanged { sheet: 0 },
        ));
        assert_eq!(epoch(), 3, "structure can change a merge list");

        bus.emit_event(SpreadsheetEvent::Theme(ThemeEvent::PaletteUpdated));
        assert_eq!(epoch(), 4, "theme can change derived link colors");

        // A batch with a navigation event but no metadata category is a no-op
        // for the epoch; one with a metadata category advances it once.
        bus.emit_events([SpreadsheetEvent::Navigation(
            NavigationEvent::ViewportScrolled {
                sheet: 0,
                top_row: 2,
                left_col: 2,
            },
        )]);
        assert_eq!(epoch(), 4, "a navigation-only batch stays put");

        bus.emit_events([
            SpreadsheetEvent::Navigation(NavigationEvent::ViewportScrolled {
                sheet: 0,
                top_row: 3,
                left_col: 3,
            }),
            SpreadsheetEvent::Content(ContentEvent::NamedRangesChanged),
        ]);
        assert_eq!(
            epoch(),
            5,
            "one metadata event in a batch advances the epoch once"
        );
    });
}
