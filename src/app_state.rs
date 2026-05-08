//! Application-level UI state, decoupled from workbook/spreadsheet editing state.
//!
//! [`AppState`] holds signals for global UI concerns — theme, sidebar, perf
//! panel — that live outside the spreadsheet domain. The `registry_version`
//! counter replaces the former `Workbook*` structure events: the left drawer
//! subscribes to it instead of the event bus, eliminating spurious re-renders
//! during worksheet scroll.

use leptos::prelude::*;
use leptos_use::{ColorMode, UseColorModeReturn};

use crate::events::*;
use crate::perf::PerfTimings;
use crate::state::Split;
use crate::theme::{use_rusty_calc_theme, Theme};

#[derive(Clone, Copy)]
pub struct AppState {
    events: EventBus,
    /// Resolved theme from leptos-use: with `.emit_auto(false)`, this signal
    /// only ever carries `Light` or `Dark`, never `Auto`.
    theme_mode: Signal<ColorMode>,
    /// Writes user preference (Auto/Light/Dark) into leptos-use, which
    /// persists to localStorage and updates `<html data-theme>`.
    set_theme_mode: WriteSignal<ColorMode>,
    pub(crate) sidebar_open: Split<bool>,
    pub(crate) collapsed_groups: Split<Vec<String>>,
    #[allow(dead_code)]
    pub(crate) show_perf_panel: Split<bool>,
    pub perf: PerfTimings,
    /// Bumped when the workbook registry changes (create/delete/rename/group).
    pub registry_version: RwSignal<u64>,
}

impl AppState {
    pub fn new(events: EventBus) -> Self {
        let UseColorModeReturn {
            mode, set_mode, ..
        } = use_rusty_calc_theme();
        Self {
            events,
            theme_mode: mode,
            set_theme_mode: set_mode,
            sidebar_open: Split::new(false),
            collapsed_groups: Split::new(vec![]),
            show_perf_panel: Split::new(false),
            perf: PerfTimings::new(),
            registry_version: RwSignal::new(0),
        }
    }

    pub fn bump_registry(&self) {
        self.registry_version.update(|v| *v = v.wrapping_add(1));
    }

    pub fn get_theme(&self) -> Theme {
        self.theme_mode.get().into()
    }

    #[allow(dead_code)]
    pub fn get_theme_untracked(&self) -> Theme {
        self.theme_mode.get_untracked().into()
    }

    pub fn set_theme(&self, theme: Theme) {
        self.set_theme_mode.set(theme.into());
        self.events
            .emit_event(SpreadsheetEvent::Theme(ThemeEvent::ThemeToggled {
                new_theme: theme,
            }));
    }

    pub fn toggle_light_dark(&self) {
        // Resolve Auto to a concrete theme before toggling so Auto -> click -> Dark
        // works correctly rather than silently doing nothing.
        let next = match self.get_theme() {
            Theme::Light | Theme::Auto => Theme::Dark,
            Theme::Dark => Theme::Light,
        };
        self.set_theme(next);
    }
}
