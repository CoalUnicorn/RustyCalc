//! Renderer-owned capture state and the publication boundary.
//!
//! Compiled only with `dev-diagnostics`. [`DiagState`] holds the enable flag,
//! the in-flight buffer that [`capture`](super::capture) fills, and the
//! published last snapshot that `publish_diag` seals for the host. The
//! snapshot vocabulary these fields carry lives in [`super::snapshot`].

use std::cell::{Cell, RefCell, RefMut};

use super::snapshot::{
    DIAG_SCHEMA_VERSION, DiagBufferTruth, DiagCacheResolution, DiagCacheTruth,
    DiagFingerprintTruth, DiagPaintedLayers, FrameDiagnostics,
};
use crate::orchestrator::{FrameOutcome, RenderStrategy};
use crate::pending_work::WorkFlags;
use crate::renderer::cache::BufferTruth;
use crate::renderer::cache::fingerprint::FingerprintTruth;

/// Frame-completion facts assembled by `Orchestrator::finish_attempt` and
/// handed to publication as ONE value. The renderer wrapper and the core
/// sink take this by value so adjacent scalar arguments cannot be swapped
/// at the wrapper boundary. Crate-private: this is a collection input, not
/// part of the published snapshot contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiagCompletion {
    pub attempt_seq: u64,
    pub selected: Option<RenderStrategy>,
    pub work: WorkFlags,
    pub effective: Option<RenderStrategy>,
    pub committed_seq: Option<u64>,
    pub outcome: FrameOutcome,
    pub layers: DiagPaintedLayers,
    pub resolution: DiagCacheResolution,
}

/// Renderer-owned capture state: enable flag, in-flight buffer, published
/// last snapshot. Interior mutability because paint methods run on `&self`.
///
/// `enabled`, `capture`, and `ensure_capture` are `pub(super)` because the
/// capture writers in `super::capture` append to the same buffer.
pub(crate) struct DiagState {
    pub(super) enabled: Cell<bool>,
    pub(super) capture: RefCell<Option<FrameDiagnostics>>,
    published: RefCell<Option<FrameDiagnostics>>,
}

impl Default for DiagState {
    fn default() -> Self {
        Self {
            enabled: Cell::new(false),
            capture: RefCell::new(None),
            published: RefCell::new(None),
        }
    }
}

impl DiagState {
    /// Empty the in-flight capture. Called by `render_pending` at attempt
    /// start so a capture-failure hold cannot inherit the previous
    /// attempt's renderer sections.
    pub(crate) fn reset_capture(&self) {
        *self.capture.borrow_mut() = None;
    }

    /// `&mut` access to a fresh capture. All write sites route through
    /// this so an enable toggle mid-attempt never half-writes.
    pub(super) fn ensure_capture(&self) -> RefMut<'_, Option<FrameDiagnostics>> {
        let mut slot = self.capture.borrow_mut();
        slot.get_or_insert_with(|| FrameDiagnostics {
            schema_version: DIAG_SCHEMA_VERSION,
            ..FrameDiagnostics::default()
        });
        slot
    }
}

impl<P: crate::painter::Painter> crate::renderer::RendererCore<P> {
    /// Committed cache truth at the moment of the read. Shared by the
    /// attempt-start sample (`diag_begin_attempt`) and the post-install read
    /// in `publish_diag`. The in-flight writers in `super::capture` call it
    /// too, so it stays on this side of the module.
    pub(super) fn cache_truth_now(&self) -> DiagCacheTruth {
        DiagCacheTruth {
            layout: self.grid_cache.layout(),
            buffer_truth: if self.grid_cache.buffer_truth() == BufferTruth::Valid {
                DiagBufferTruth::Valid
            } else {
                DiagBufferTruth::Stale
            },
            fingerprint_truth: match self.grid_cache.fingerprint.truth() {
                FingerprintTruth::Exact => DiagFingerprintTruth::Exact,
                FingerprintTruth::Stale => DiagFingerprintTruth::Stale,
            },
        }
    }

    /// Runtime switch. Disabling drops the retained published snapshot so
    /// the web facade's `frameDiagnostics()` returns `undefined`.
    pub(crate) fn set_diag_enabled(&self, enabled: bool) {
        self.diag.enabled.set(enabled);
        if !enabled {
            *self.diag.published.borrow_mut() = None;
            *self.diag.capture.borrow_mut() = None;
        }
    }

    pub(crate) fn diag_reset_capture(&self) {
        self.diag.reset_capture();
    }

    /// Seal the in-flight capture and move it into `published`. Only
    /// `Orchestrator::finish_attempt` calls this, after the cache commit
    /// (if any) was installed — so `committed_after` reads the
    /// post-commit truth and a held attempt keeps
    /// `committed_before == committed_after`.
    pub(crate) fn publish_diag(&self, completion: DiagCompletion) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut snapshot =
            self.diag
                .capture
                .borrow_mut()
                .take()
                .unwrap_or_else(|| FrameDiagnostics {
                    schema_version: DIAG_SCHEMA_VERSION,
                    ..FrameDiagnostics::default()
                });
        snapshot.attempt_seq = completion.attempt_seq;
        snapshot.committed_seq = completion.committed_seq;
        snapshot.selected = completion.selected;
        snapshot.effective = completion.effective;
        snapshot.work = completion.work;
        snapshot.outcome = completion.outcome;
        snapshot.painted_layers = completion.layers;
        snapshot.cache.resolution = completion.resolution;
        let committed_after = self.cache_truth_now();
        // A capture-failure attempt never reaches a grid prepare, so its
        // committed cache could not have changed during the attempt —
        // before == after by construction.
        if snapshot.cache.committed_before.is_none() {
            snapshot.cache.committed_before = Some(committed_after.clone());
        }
        snapshot.cache.committed_after = committed_after;
        *self.diag.published.borrow_mut() = Some(snapshot);
    }

    /// Clone of the last published snapshot. Called by the web facade on
    /// demand only.
    pub(crate) fn last_diag(&self) -> Option<FrameDiagnostics> {
        self.diag.published.borrow().clone()
    }
}
