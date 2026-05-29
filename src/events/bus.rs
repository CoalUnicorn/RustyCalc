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

    pub fn emit_event(&self, event: SpreadsheetEvent) {
        self.emit_events(vec![event]);
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
