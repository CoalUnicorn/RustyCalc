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
use crate::theme::{Theme, use_rusty_calc_theme};

/// One-shot command from the PerfPanel record button to the Worksheet
/// dispatch Effect. The Effect drains it (`set(None)`) after handing the
/// call to the iron-canvas orchestrator. Exists in both build flavors —
/// in prod (no `dev-tools` feature) it is written but never read, since the
/// PerfPanel button is hidden by the runtime `recordingSupported()` guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingCmd {
    Start,
    Stop,
}

/// One-shot command from the PerfPanel export buttons to the Worksheet
/// dispatch Effect. Same drain pattern as [`RecordingCmd`]. `Svg` is served
/// by `IronCanvas::exportSvg` (always on); `Pdf` is served by
/// `IronCanvas::exportPdf` (gated behind the `export → iron-canvas-web/pdf`
/// feature chain, orthogonal to `dev-tools`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportCmd {
    Svg,
    Pdf,
}

/// One-shot command from the PlaybackPanel to the Worksheet dispatch
/// Effect. Same drain pattern as [`RecordingCmd`]. `Load` carries owned
/// `.icr` bytes — read once by the Effect, then cleared.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum PlaybackCmd {
    Load(Vec<u8>),
    Seek(u32),
    Play,
    Pause,
    Exit,
}

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
    pub(crate) show_perf_panel: Split<bool>,
    /// `true` while iron-canvas is capturing frames. Updated by the
    /// Worksheet dispatch Effect after a successful start/stop.
    pub recording_active: Split<bool>,
    /// Pending command from the PerfPanel button. Cleared by Worksheet
    /// once dispatched. See [`RecordingCmd`].
    pub recording_cmd: Split<Option<RecordingCmd>>,
    /// Pending export command from the PerfPanel SVG/PDF buttons.
    /// Cleared by Worksheet once the file download has been triggered.
    pub export_cmd: Split<Option<ExportCmd>>,
    /// Pending playback command. Cleared by Worksheet once dispatched.
    pub playback_cmd: Split<Option<PlaybackCmd>>,
    /// `true` once an `.icr` is loaded and playback has taken ownership of
    /// the live canvases; `false` again on Exit.
    pub playback_loaded: Split<bool>,
    /// Mirrors `IronCanvas::isPlaying()` — synced from the rAF tick.
    pub playback_playing: Split<bool>,
    /// Current displayed frame, synced from the rAF tick.
    pub playback_frame: Split<u32>,
    /// Total frames in the loaded recording. Set on Load, zeroed on Exit.
    pub playback_frame_count: Split<u32>,
    pub perf: PerfTimings,
    /// Bumped when the workbook registry changes (create/delete/rename/group).
    pub registry_version: RwSignal<u64>,
}

impl AppState {
    pub fn new(events: EventBus) -> Self {
        let UseColorModeReturn { mode, set_mode, .. } = use_rusty_calc_theme();
        Self {
            events,
            theme_mode: mode,
            set_theme_mode: set_mode,
            sidebar_open: Split::new(false),
            collapsed_groups: Split::new(vec![]),
            show_perf_panel: Split::new(cfg!(feature = "dev-tools")),
            recording_active: Split::new(false),
            recording_cmd: Split::new(None),
            export_cmd: Split::new(None),
            playback_cmd: Split::new(None),
            playback_loaded: Split::new(false),
            playback_playing: Split::new(false),
            playback_frame: Split::new(0),
            playback_frame_count: Split::new(0),
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
