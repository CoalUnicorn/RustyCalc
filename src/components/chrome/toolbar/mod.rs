mod alignment;
mod color_pickers;
mod font;
mod format_toggles;
mod freeze;
mod named_ranges;
mod number_format;
mod undo_redo;

use leptos::prelude::*;

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

/// Top toolbar. Creates two shared memos once and provides them via context so
/// every sub-component reads the same reactive computation instead of each
/// instantiating its own (was: 4 x Memo, 12 subscriptions -> 2 x Memo, 6 subscriptions).
///
/// Context provided to children:
/// - `Memo<ToolbarState>`   - font size/family, bold/italic/color, etc.
/// - `Memo<(bool, bool)>`   - (can_undo, can_redo)
#[component]
pub fn Toolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Re-runs on format changes (cell styling) AND navigation (selection change).
    // visual_events catches theme/canvas redraws that also affect cell style display.
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

    view! {
        <div class="tb">
            <UndoRedo />
            <div class="tb-sep" />
            <NumberFormatPicker />
            <NumFmtQuickButtons />
            <div class="tb-sep" />
            <FontFamily />
            <div class="tb-sep" />
            <FontSize />
            <div class="tb-sep" />
            <TextColorPickerToolbar />
            <div class="tb-sep" />
            <BackgroundColorPickerToolbar />
            <div class="tb-sep" />
            <FormatToggles />
            <AlignButtons />
            <VertAlignButtons />
            <ClearFormat />
            <div class="tb-sep" />
            <FreezePane />
            <div class="tb-sep" />
            <NamedRangesButton />
        </div>
    }
}
