//! Application-level UI state, decoupled from workbook/spreadsheet editing state.
//!
//! [`AppState`] holds signals for global UI concerns — theme, sidebar, perf
//! panel — that live outside the spreadsheet domain. The `registry_version`
//! counter replaces the former `Workbook*` structure events: the left drawer
//! subscribes to it instead of the event bus, eliminating spurious re-renders
//! during worksheet scroll.

use leptos::prelude::*;

use crate::events::*;
use crate::perf::PerfTimings;
use crate::state::Split;
use crate::theme::Theme;

#[derive(Clone, Copy)]
pub struct AppState {
    events: EventBus,
    pub(crate) theme: Split<Theme>,
    pub(crate) sidebar_open: Split<bool>,
    pub(crate) collapsed_groups: Split<Vec<String>>,
    // pub(crate) show_perf_panel: Split<bool>,
    pub perf: PerfTimings,
    /// Bumped when the workbook registry changes (create/delete/rename/group).
    pub registry_version: RwSignal<u64>,
}

impl AppState {
    pub fn new(events: EventBus) -> Self {
        Self {
            events,
            theme: Split::new(Theme::from_storage()),
            sidebar_open: Split::new(false),
            collapsed_groups: Split::new(vec![]),
            // show_perf_panel: Split::new(false),
            perf: PerfTimings::new(),
            registry_version: RwSignal::new(0),
        }
    }

    pub fn bump_registry(&self) {
        self.registry_version.update(|v| *v = v.wrapping_add(1));
    }

    pub fn get_theme(&self) -> Theme {
        self.theme.get().resolve_with_system()
    }

    #[allow(dead_code)]
    pub fn get_theme_untracked(&self) -> Theme {
        self.theme.get_untracked().resolve_with_system()
    }

    pub fn set_theme(&self, theme: Theme) {
        self.theme.set(theme);
        self.events
            .emit_event(SpreadsheetEvent::Theme(ThemeEvent::ThemeToggled {
                new_theme: theme,
            }));
    }

    pub fn toggle_theme(&self) {
        let next = match self.theme.get() {
            Theme::Auto => Theme::Light,
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Auto,
        };
        self.set_theme(next);
    }

    #[allow(dead_code)]
    pub fn toggle_light_dark(&self) {
        match self.theme.get() {
            Theme::Auto => {}
            Theme::Light => self.set_theme(Theme::Dark),
            Theme::Dark => self.set_theme(Theme::Light),
        }
    }
}
