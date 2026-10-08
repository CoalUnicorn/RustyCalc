//! Per-category event signals and the `emit_event(s)` dispatch fan-out.

use leptos::prelude::*;

use super::{
    ContentEvent, FormatEvent, NavigationEvent, SpreadsheetEvent, StructureEvent, ThemeEvent,
};

/// Per-category event signals. Each holds events from the most recent
/// `emit_event(s)` call — replaced (not appended) on each emit.
#[derive(Clone, Copy)]
pub struct EventBus {
    pub content: RwSignal<Vec<ContentEvent>>,
    pub format: RwSignal<Vec<FormatEvent>>,
    pub navigation: RwSignal<Vec<NavigationEvent>>,
    pub structure: RwSignal<Vec<StructureEvent>>,
    pub theme: RwSignal<Vec<ThemeEvent>>,
    /// Monotonic count of emitted batches that can change worksheet
    /// *metadata* — the link list and the merged-range list: content, format,
    /// structure, and theme. Navigation events are excluded: a selection move
    /// cannot change a link or a merge, and counting it would defeat the
    /// canvas's metadata-snapshot reuse for exactly the overlay-only repaints
    /// that reuse targets. Non-reactive; the canvas reads it with
    /// `get_value` and compares it, so a value that does not move is free.
    pub metadata_seq: StoredValue<u64, LocalStorage>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            content: RwSignal::new(vec![]),
            format: RwSignal::new(vec![]),
            navigation: RwSignal::new(vec![]),
            structure: RwSignal::new(vec![]),
            theme: RwSignal::new(vec![]),
            metadata_seq: StoredValue::new_local(0),
        }
    }

    /// Single-event fast path (the common case: arrow-key nav fires ~30/s,
    /// each touching exactly one category). Overwrite the target category in
    /// place — `clear() + push()` reuses the Vec's existing allocation, so the
    /// steady-state path allocates zero times after the first event. Other
    /// categories are cleared only if they still hold events from the previous
    /// emit, skipping up to four redundant signal writes per tick.
    ///
    /// `update` is used (not `set`) so a repeated event on the same range still
    /// notifies — `set`'s `PartialEq` check would suppress it.
    pub fn emit_event(&self, event: SpreadsheetEvent) {
        // A navigation event cannot change a link or a merge list, so it does
        // not advance the metadata epoch. Everything else can.
        let touches_metadata = !matches!(event, SpreadsheetEvent::Navigation(_));
        match event {
            SpreadsheetEvent::Content(e) => {
                self.content.update(|v| {
                    v.clear();
                    v.push(e);
                });
                Self::clear_stale(self.format);
                Self::clear_stale(self.navigation);
                Self::clear_stale(self.structure);
                Self::clear_stale(self.theme);
            }
            SpreadsheetEvent::Format(e) => {
                self.format.update(|v| {
                    v.clear();
                    v.push(e);
                });
                Self::clear_stale(self.content);
                Self::clear_stale(self.navigation);
                Self::clear_stale(self.structure);
                Self::clear_stale(self.theme);
            }
            SpreadsheetEvent::Navigation(e) => {
                self.navigation.update(|v| {
                    v.clear();
                    v.push(e);
                });
                Self::clear_stale(self.content);
                Self::clear_stale(self.format);
                Self::clear_stale(self.structure);
                Self::clear_stale(self.theme);
            }
            SpreadsheetEvent::Structure(e) => {
                self.structure.update(|v| {
                    v.clear();
                    v.push(e);
                });
                Self::clear_stale(self.content);
                Self::clear_stale(self.format);
                Self::clear_stale(self.navigation);
                Self::clear_stale(self.theme);
            }
            SpreadsheetEvent::Theme(e) => {
                self.theme.update(|v| {
                    v.clear();
                    v.push(e);
                });
                Self::clear_stale(self.content);
                Self::clear_stale(self.format);
                Self::clear_stale(self.navigation);
                Self::clear_stale(self.structure);
            }
        }
        if touches_metadata {
            self.bump_metadata_seq();
        }
    }

    /// Empty a category that still carries events from the previous emit,
    /// preserving its allocation. No-op when already empty so the common
    /// single-category tick doesn't touch the other four signals.
    fn clear_stale<T: Send + Sync + 'static>(sig: RwSignal<Vec<T>>) {
        if !sig.with_untracked(Vec::is_empty) {
            sig.update(|v| v.clear());
        }
    }

    /// Advance the metadata epoch. Non-reactive read (`get_value`) in the
    /// canvas, so bumping never re-runs a subscriber.
    fn bump_metadata_seq(&self) {
        self.metadata_seq
            .set_value(self.metadata_seq.get_value().wrapping_add(1));
    }

    pub fn emit_events(&self, new_events: impl IntoIterator<Item = SpreadsheetEvent>) {
        self.publish(new_events);
    }

    /// Distribute one emit into the five category signals, replacing each.
    ///
    /// Non-empty categories use `update()`: `set()` would compare via
    /// `PartialEq` and suppress notification when the same event fires twice
    /// on the same range, whereas `update()` always notifies. Empty categories
    /// use `set(vec![])` so an already-empty signal stays a silent no-op.
    fn publish(&self, new_events: impl IntoIterator<Item = SpreadsheetEvent>) {
        let mut content = vec![];
        let mut format = vec![];
        let mut navigation = vec![];
        let mut structure = vec![];
        let mut theme = vec![];

        for event in new_events {
            match event {
                SpreadsheetEvent::Content(e) => content.push(e),
                SpreadsheetEvent::Format(e) => format.push(e),
                SpreadsheetEvent::Navigation(e) => navigation.push(e),
                SpreadsheetEvent::Structure(e) => structure.push(e),
                SpreadsheetEvent::Theme(e) => theme.push(e),
            }
        }

        // Content, format, structure, or theme in the batch can change a link
        // or a merge list; navigation alone cannot. Record it before the
        // vectors are moved into their signals.
        let touches_metadata =
            !(content.is_empty() && format.is_empty() && structure.is_empty() && theme.is_empty());

        // Replace all 5 signals so no stale events from the previous action remain.
        if content.is_empty() {
            self.content.set(vec![]);
        } else {
            self.content.update(|v| *v = content);
        }
        if format.is_empty() {
            self.format.set(vec![]);
        } else {
            self.format.update(|v| *v = format);
        }
        if navigation.is_empty() {
            self.navigation.set(vec![]);
        } else {
            self.navigation.update(|v| *v = navigation);
        }
        if structure.is_empty() {
            self.structure.set(vec![]);
        } else {
            self.structure.update(|v| *v = structure);
        }
        if theme.is_empty() {
            self.theme.set(vec![]);
        } else {
            self.theme.update(|v| *v = theme);
        }
        // Content, format, structure, or theme in the batch can change a link
        // or a merge list; navigation alone cannot.
        if touches_metadata {
            self.bump_metadata_seq();
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
