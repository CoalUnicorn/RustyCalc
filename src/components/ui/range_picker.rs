//! Reusable range-picker input: a text field plus an "arm" (⊞) button that
//! captures the live grid selection into the field.
//!
//! This is the UI half of the non-modal drawer range-picking feature. The
//! state half lives in [`WorkbookState::range_capture`]: exactly one field
//! across the app can be *armed* at a time. While armed, an `Effect` here
//! watches the navigation event bus and mirrors every selection change into
//! the bound `value` signal — so dragging a range on the grid (or arrow-key
//! selecting) fills the field live. No mouse/keyboard handler changes are
//! needed: the grid already publishes `NavigationEvent::SelectionRangeChanged`.
//!
//! Manual typing still works and *disarms* the field, so a user's hand-typed
//! range is never clobbered by a stray selection.

use leptos::prelude::*;

use crate::coord::{selection_a1_qualified_absolute, selection_a1_relative};
use crate::state::{ModelStore, RangeCaptureTarget, WorkbookState};

/// The A1 shape a consumer wants captured. Each maps to one of the
/// `selection_a1_*` formatters in `crate::coord`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeFormat {
    /// `B2:D8` — CF sqref.
    SheetRelative,
    /// `Sheet1!$B$2:$D$8` — qualified-absolute form. Part of the primitive's
    /// API; the Named Range "refers to" field currently captures via the
    /// `selection_a1_qualified_absolute` helper directly (it needs to re-run
    /// formula analysis), so no `RangePickerInput` consumer selects this yet.
    #[allow(dead_code)]
    QualifiedAbsolute,
}

impl RangeFormat {
    fn apply(self, model: ModelStore) -> String {
        model.with_value(|m| match self {
            RangeFormat::SheetRelative => selection_a1_relative(m),
            RangeFormat::QualifiedAbsolute => selection_a1_qualified_absolute(m),
        })
    }
}

/// Text input bound to `value`, with a trailing ⊞ button that arms grid
/// capture for `target`. When armed, grid selections flow into `value`
/// formatted per `format`.
#[component]
pub fn RangePickerInput(
    /// The field's text. Owned by the caller so it can read the final value.
    value: RwSignal<String>,
    /// Which logical field this is — drives mutual-exclusion in `range_capture`.
    target: RangeCaptureTarget,
    /// A1 shape to write on capture.
    format: RangeFormat,
    #[prop(optional, into)] placeholder: String,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let is_armed = move || state.range_capture.get() == Some(target);

    // Reactive capture: any selection change, while armed for this target,
    // overwrites the field with the freshly-formatted selection.
    Effect::new(move |_| {
        // Subscribe to the navigation bus — this is the trigger.
        let _ = state.events.navigation.get();
        if state.range_capture.get_untracked() == Some(target) {
            value.set(format.apply(model));
        }
    });

    // ⊞ toggles arming. Arming this target implicitly disarms any other
    // (the signal holds a single `Option`).
    let toggle_arm = move |_: web_sys::MouseEvent| {
        if state.range_capture.get_untracked() == Some(target) {
            state.range_capture.set(None);
        } else {
            state.range_capture.set(Some(target));
        }
    };

    // Manual typing disarms so hand-entered text isn't overwritten by a
    // later selection.
    let on_input = move |ev: web_sys::Event| {
        value.set(event_target_value(&ev));
        if state.range_capture.get_untracked() == Some(target) {
            state.range_capture.set(None);
        }
    };

    let btn_class = move || {
        if is_armed() {
            "rp-arm rp-arm-active"
        } else {
            "rp-arm"
        }
    };

    view! {
        <div class="rp-wrap">
            <input
                class="rp-input"
                type="text"
                prop:value=move || value.get()
                on:input=on_input
                placeholder=placeholder
            />
            <button
                class=btn_class
                on:click=toggle_arm
                type="button"
                title="Pick range from grid"
            >
                "⊞"
            </button>
        </div>
    }
}
