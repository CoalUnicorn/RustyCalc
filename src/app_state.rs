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
#[cfg(feature = "dev-tools")]
use crate::perf::{AttemptKey, PerfStore};
use crate::state::Split;
use crate::theme::{Theme, use_rusty_calc_theme};

/// One-shot command from the inspector's Tools view to the Worksheet
/// dispatch Effect. The Effect drains it (`set(None)`) after handing the
/// call to the iron-canvas orchestrator.
///
/// Dev-tools only: the button that writes it lives in the inspector, and the
/// coordinator that drains it is gated with the inspector.
#[cfg(feature = "dev-tools")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingCmd {
    Start,
    Stop,
}

/// One-shot command from the Perf panel to the Worksheet capture Effect.
///
/// The panel never touches the canvas: it publishes intent here, and one
/// coordinator drains it. `Set(bool)` is gone because the canvas flag is
/// derived from the store state — a limit stop or a generation change needs
/// no command at all.
#[cfg(feature = "dev-tools")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureCmd {
    Start,
    Pause,
    Resume,
    Finish,
    ExportCaptureJson,
    /// Copy one attempt's JSON. The key names the attempt the inspector has
    /// selected, so the copy follows the selection rather than the newest
    /// record.
    CopyAttemptJson(AttemptKey),
}

/// One-shot command from the inspector's Tools view to the Worksheet
/// dispatch Effect. Same drain pattern as [`RecordingCmd`]. `Svg` is served
/// by `IronCanvas::exportSvg`; `Pdf` is served by `IronCanvas::exportPdf`
/// (gated behind the `export -> iron-canvas-web/pdf` feature chain).
#[cfg(feature = "dev-tools")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportCmd {
    Svg,
    Pdf,
}

/// One-shot command from the PlaybackPanel to the Worksheet dispatch
/// Effect. Same drain pattern as [`RecordingCmd`]. `Load` carries owned
/// `.icr` bytes — read once by the Effect, then cleared.
#[cfg(feature = "dev-tools")]
#[derive(Clone, Debug)]
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
    /// Whether the dev-tools performance inspector window is open. Seeded
    /// open in a `dev-tools` build, where the toolbar launcher can close it.
    pub(crate) inspector_open: Split<bool>,
    /// `true` while iron-canvas is capturing frames. Updated by the
    /// Worksheet dispatch Effect after a successful start/stop.
    #[cfg(feature = "dev-tools")]
    pub recording_active: Split<bool>,
    /// Pending command from the inspector's record button. Cleared by
    /// Worksheet once dispatched. See [`RecordingCmd`].
    #[cfg(feature = "dev-tools")]
    pub recording_cmd: Split<Option<RecordingCmd>>,
    /// Pending capture command from the inspector. Cleared by Worksheet once
    /// dispatched. See [`CaptureCmd`].
    #[cfg(feature = "dev-tools")]
    pub capture_cmd: Split<Option<CaptureCmd>>,
    /// Pending export command from the inspector's Tools view. Cleared by
    /// Worksheet once the file download has been triggered.
    #[cfg(feature = "dev-tools")]
    pub export_cmd: Split<Option<ExportCmd>>,
    /// Pending playback command. Cleared by Worksheet once dispatched.
    #[cfg(feature = "dev-tools")]
    pub playback_cmd: Split<Option<PlaybackCmd>>,
    /// `true` once an `.icr` is loaded and playback has taken ownership of
    /// the live canvases; `false` again on Exit.
    #[cfg(feature = "dev-tools")]
    pub playback_loaded: Split<bool>,
    /// Mirrors `IronCanvas::isPlaying()` — synced from the playback Effect.
    #[cfg(feature = "dev-tools")]
    pub playback_playing: Split<bool>,
    /// Current displayed frame, synced from the playback Effect.
    #[cfg(feature = "dev-tools")]
    pub playback_frame: Split<u32>,
    /// Total frames in the loaded recording. Set on Load, zeroed on Exit.
    #[cfg(feature = "dev-tools")]
    pub playback_frame_count: Split<u32>,
    pub perf: PerfTimings,
    /// Paint-attempt capture store: signals only, so this struct stays `Copy`.
    /// Dev-tools only — a production build retains no capture state.
    #[cfg(feature = "dev-tools")]
    pub perf_store: PerfStore,
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
            inspector_open: Split::new(cfg!(feature = "dev-tools")),
            #[cfg(feature = "dev-tools")]
            recording_active: Split::new(false),
            #[cfg(feature = "dev-tools")]
            recording_cmd: Split::new(None),
            #[cfg(feature = "dev-tools")]
            capture_cmd: Split::new(None),
            #[cfg(feature = "dev-tools")]
            export_cmd: Split::new(None),
            #[cfg(feature = "dev-tools")]
            playback_cmd: Split::new(None),
            #[cfg(feature = "dev-tools")]
            playback_loaded: Split::new(false),
            #[cfg(feature = "dev-tools")]
            playback_playing: Split::new(false),
            #[cfg(feature = "dev-tools")]
            playback_frame: Split::new(0),
            #[cfg(feature = "dev-tools")]
            playback_frame_count: Split::new(0),
            perf: PerfTimings::new(),
            #[cfg(feature = "dev-tools")]
            perf_store: PerfStore::new(),
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
