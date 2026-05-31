mod alignment;
mod color_pickers;
mod font;
mod format_toggles;
mod freeze;
pub(crate) mod icon;
mod named_ranges;
mod number_format;
pub(crate) mod overflow;
pub(crate) mod section;
pub(crate) mod tab_strip;
mod undo_redo;

use leptos::prelude::*;

use crate::events::StructureEvent;
use crate::model::{SheetQuery, frontend_types::ToolbarState};
use crate::state::{ModelStore, WorkbookState};

use alignment::{AlignButtons, VertAlignButtons};
use color_pickers::{BackgroundColorPickerToolbar, TextColorPickerToolbar};
use font::{FontFamily, FontSize};
use format_toggles::{ClearFormat, FormatToggles};
use freeze::FreezePane;
use named_ranges::NamedRangesButton;
use number_format::{NumFmtQuickButtons, NumberFormatPicker};
use undo_redo::UndoRedo;

use overflow::OverflowRow;
use section::{ToolSlot, ToolbarSection};
use tab_strip::TabStrip;

/// Two-tier toolbar: a tab strip selecting a `ToolbarSection`, above a single
/// overflow row whose slots are rebuilt for the active section.
///
/// Context provided to children:
/// - `Memo<ToolbarState>`        - font size/family, bold/italic/color, etc.
/// - `Memo<(bool, bool)>`        - (can_undo, can_redo)
/// - `RwSignal<ToolbarSection>`  - active section, read by `TabStrip`.
#[component]
pub fn Toolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let toolbar_state: Memo<ToolbarState> = Memo::new(move |_| {
        let _ = state.events.format.get();
        let _ = state.events.navigation.get();
        let _ = state.events.content.get();
        model.with_value(|m| m.toolbar_state())
    });

    let undo_redo_state: Memo<(bool, bool)> = Memo::new(move |_| {
        let _ = state.events.content.get();
        model.with_value(|m| (m.can_undo(), m.can_redo()))
    });

    provide_context(toolbar_state);
    provide_context(undo_redo_state);

    let active_section = RwSignal::new(ToolbarSection::default());
    provide_context(active_section);

    // Reset to Home when a fresh workbook is loaded or switched.
    Effect::new(move |_| {
        if state.events.structure.with(|evs| {
            evs.iter()
                .any(|e| matches!(e, StructureEvent::DocumentReset))
        }) {
            active_section.set(ToolbarSection::Home);
        }
    });

    let slots = move || match active_section.get() {
        ToolbarSection::Home => vec![
            ToolSlot::new("Undo/Redo", || view! { <UndoRedo /> }.into_any()),
            ToolSlot::new("Font", || view! { <FontFamily /> <FontSize /> }.into_any()),
            ToolSlot::new("Style", || {
                view! {
                    <FormatToggles />
                    <TextColorPickerToolbar />
                    <BackgroundColorPickerToolbar />
                }
                .into_any()
            }),
            ToolSlot::new("Align", || {
                view! {
                    <AlignButtons />
                    <VertAlignButtons />
                    <ClearFormat />
                }
                .into_any()
            }),
        ],
        ToolbarSection::Data => vec![
            ToolSlot::new("Number", || {
                view! { <NumberFormatPicker /> <NumFmtQuickButtons /> }.into_any()
            }),
            ToolSlot::new("Named ranges", || {
                view! { <NamedRangesButton /> }.into_any()
            }),
        ],
        ToolbarSection::View => vec![ToolSlot::new("Freeze", || {
            view! { <FreezePane /> }.into_any()
        })],
    };

    view! {
        <div class="tb-shell">
            <TabStrip />
            {move || view! { <OverflowRow slots=slots() /> }}
        </div>
    }
}
