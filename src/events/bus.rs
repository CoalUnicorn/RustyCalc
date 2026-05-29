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
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            content: RwSignal::new(vec![]),
            format: RwSignal::new(vec![]),
            navigation: RwSignal::new(vec![]),
            structure: RwSignal::new(vec![]),
            theme: RwSignal::new(vec![]),
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
    }

    /// Empty a category that still carries events from the previous emit,
    /// preserving its allocation. No-op when already empty so the common
    /// single-category tick doesn't touch the other four signals.
    fn clear_stale<T: Send + Sync + 'static>(sig: RwSignal<Vec<T>>) {
        if !sig.with_untracked(Vec::is_empty) {
            sig.update(|v| v.clear());
        }
    }

    pub fn emit_events(&self, new_events: impl IntoIterator<Item = SpreadsheetEvent>) {
        let mut content = vec![];
        let mut format = vec![];
        let mut navigation = vec![];
        let mut structure = vec![];
        let mut theme = vec![];

        for event in new_events {
            #[cfg(debug_assertions)]
            {
                use std::cell::Cell;
                thread_local! { static LAST: Cell<f64> = const { Cell::new(0.0) }; }
                let now = crate::perf::now();
                LAST.with(|t| {
                    t.set(now);
                });
            }
            match event {
                SpreadsheetEvent::Content(e) => content.push(e),
                SpreadsheetEvent::Format(e) => format.push(e),
                SpreadsheetEvent::Navigation(e) => navigation.push(e),
                SpreadsheetEvent::Structure(e) => structure.push(e),
                SpreadsheetEvent::Theme(e) => theme.push(e),
            }
        }

        // Replace all 5 signals so no stale events from the previous action remain.
        // Use update() not set(): set() uses PartialEq and suppresses notification
        // when the same event fires twice on the same range. update() always notifies.
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
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
