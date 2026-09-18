//! Per-category event signals and the `emit_event(s)` dispatch fan-out.

use leptos::prelude::*;

use super::{
    ContentEvent, FormatEvent, NavigationEvent, SpreadsheetEvent, StructureEvent, ThemeEvent,
};

/// Ordered observer of one emitted batch. Runs before the category signals
/// take ownership of the payloads.
#[cfg(feature = "dev-tools")]
pub type BatchObserver = std::rc::Rc<dyn Fn(u64, &[SpreadsheetEvent])>;

/// Per-category event signals. Each holds events from the most recent
/// `emit_event(s)` call — replaced (not appended) on each emit.
#[derive(Clone, Copy)]
pub struct EventBus {
    pub content: RwSignal<Vec<ContentEvent>>,
    pub format: RwSignal<Vec<FormatEvent>>,
    pub navigation: RwSignal<Vec<NavigationEvent>>,
    pub structure: RwSignal<Vec<StructureEvent>>,
    pub theme: RwSignal<Vec<ThemeEvent>>,
    /// Capture observer. Non-reactive: installing it must not re-run any
    /// subscriber, and it is not a rendering input.
    #[cfg(feature = "dev-tools")]
    batch_observer: StoredValue<Option<BatchObserver>, LocalStorage>,
    /// Monotonic batch counter, bumped once per observed emit.
    #[cfg(feature = "dev-tools")]
    batch_seq: StoredValue<u64, LocalStorage>,
    /// Re-entrancy guard for the debug assertion in [`EventBus::observe`].
    #[cfg(feature = "dev-tools")]
    observing: StoredValue<bool, LocalStorage>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            content: RwSignal::new(vec![]),
            format: RwSignal::new(vec![]),
            navigation: RwSignal::new(vec![]),
            structure: RwSignal::new(vec![]),
            theme: RwSignal::new(vec![]),
            #[cfg(feature = "dev-tools")]
            batch_observer: StoredValue::new_local(None),
            #[cfg(feature = "dev-tools")]
            batch_seq: StoredValue::new_local(0),
            #[cfg(feature = "dev-tools")]
            observing: StoredValue::new_local(false),
        }
    }

    /// Install or clear the capture observer.
    ///
    /// The observer runs **before** the category signals take the payloads,
    /// because afterwards there is no intact batch to borrow. It therefore
    /// must not read the category signals — it sees the events only — and it
    /// must not emit a worksheet event.
    #[cfg(feature = "dev-tools")]
    pub fn set_batch_observer(&self, observer: Option<BatchObserver>) {
        self.batch_observer.set_value(observer);
    }

    /// Hand one borrowed batch to the observer, if one is installed.
    ///
    /// Returns whether an observer saw it. The batch counter advances only
    /// when an observer is present, so a build without capture keeps today's
    /// path exactly.
    #[cfg(feature = "dev-tools")]
    fn observe(&self, events: &[SpreadsheetEvent]) -> bool {
        let Some(observer) = self.batch_observer.get_value() else {
            return false;
        };
        debug_assert!(
            !self.observing.get_value(),
            "batch observer re-entered: an observer must not emit a worksheet event"
        );
        let batch_id = self.batch_seq.get_value().wrapping_add(1);
        self.batch_seq.set_value(batch_id);
        self.observing.set_value(true);
        observer(batch_id, events);
        self.observing.set_value(false);
        true
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
        // One batch identity, then the borrowed batch, then the category
        // write. `from_ref` borrows the event without cloning or allocating.
        #[cfg(feature = "dev-tools")]
        self.observe(std::slice::from_ref(&event));
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
        // With an observer installed the batch must exist as one borrowed
        // slice before any payload moves. Without one, the events go straight
        // to the per-category distribution: no extra vector, no counter read.
        #[cfg(feature = "dev-tools")]
        if self.batch_observer.get_value().is_some() {
            let batch: Vec<SpreadsheetEvent> = new_events.into_iter().collect();
            self.observe(&batch);
            return self.publish(batch);
        }
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
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
