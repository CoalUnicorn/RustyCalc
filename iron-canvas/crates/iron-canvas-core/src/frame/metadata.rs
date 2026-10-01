//! Worksheet metadata capture and its revision-keyed reuse cache.
//!
//! Capturing a sheet's link and merge indexes is the one allocating, fallible
//! model read a paint attempt performs. This module owns it, separately from
//! the scalar [`FrameInputs`](crate::frame::FrameInputs) capture: a failure
//! here holds the attempt exactly as a failed scalar read does, and the two
//! allocating list reads are skipped only when the host can prove both lists
//! are unchanged.
//!
//! Reuse is not publication. A [`MetadataSnapshot`] is stored on capture
//! success, but it is a cached model read, not committed pixels: a later hold
//! leaves the committed frame's own metadata untouched, and the snapshot is
//! rebuilt whenever its revision key moves.

use std::rc::Rc;

use crate::CanvasModel;
use crate::frame::inputs::FrameInputFailure;
use crate::model::sheet::links::LinkIndex;
use crate::model::sheet::merges::MergeTable;
use crate::model::sheet::snapshot::SheetMetadata;

/// The revision key a metadata snapshot was captured under: the model
/// generation (a `set_model` replacement), the sheet, and the host-supplied
/// metadata epoch.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetadataKey {
    model_generation: u64,
    sheet: u32,
    epoch: u64,
}

/// A validated link/merge capture kept by the
/// [`Orchestrator`](crate::Orchestrator) so a later attempt can reuse it
/// instead of re-reading the model and rebuilding both indexes.
///
/// Reuse requires an exact match of all three key fields — the model
/// generation, the sheet, and the host epoch — see [`capture_metadata`].
/// `Rc` so a reuse hands the cached metadata to the frame without a deep
/// clone.
pub(crate) struct MetadataSnapshot {
    key: MetadataKey,
    metadata: Rc<SheetMetadata>,
}

impl MetadataSnapshot {
    /// Wrap one capture under its revision key. The caller stores the result
    /// on capture success (not on commit): the snapshot is a model read, not
    /// committed state.
    pub(crate) fn new(
        model_generation: u64,
        sheet: u32,
        epoch: u64,
        metadata: Rc<SheetMetadata>,
    ) -> Self {
        Self {
            key: MetadataKey {
                model_generation,
                sheet,
                epoch,
            },
            metadata,
        }
    }

    /// The cached metadata. Valid only while the key matches the current
    /// attempt — [`capture_metadata`] is the only reader.
    fn metadata(&self) -> &Rc<SheetMetadata> {
        &self.metadata
    }

    fn matches(&self, key: MetadataKey) -> bool {
        self.key == key
    }
}

/// Capture the sheet's link and merge indexes, or reuse `cached` when it was
/// captured under the current model generation, sheet, and host epoch.
///
/// Both list reads run when there is no epoch, no cache, or any key mismatch —
/// the exact pre-cache behavior. A stale "unchanged" would leave a visible
/// link unclickable or a merged region editable, so the epoch is the host's
/// promise that either list may have changed whenever it changes.
///
/// Either read failing — a `None`, or a list the index rejects — is a hold:
/// `FrameInputFailure::SheetLinks` / `MergedRanges`, never empty data.
pub(crate) fn capture_metadata(
    model: &dyn CanvasModel,
    sheet: u32,
    model_generation: u64,
    epoch: Option<u64>,
    cached: Option<&MetadataSnapshot>,
) -> Result<Rc<SheetMetadata>, FrameInputFailure> {
    if let Some(epoch) = epoch
        && let Some(cache) = cached
        && cache.matches(MetadataKey {
            model_generation,
            sheet,
            epoch,
        })
    {
        return Ok(Rc::clone(cache.metadata()));
    }
    let links = Rc::new(
        LinkIndex::from_cells(
            model
                .get_sheet_links(sheet)
                .ok_or(FrameInputFailure::SheetLinks)?,
        )
        .map_err(|_| FrameInputFailure::SheetLinks)?,
    );
    let merges = Rc::new(
        MergeTable::from_ranges(
            model
                .get_merged_ranges(sheet)
                .ok_or(FrameInputFailure::MergedRanges)?,
        )
        .map_err(|_| FrameInputFailure::MergedRanges)?,
    );
    Ok(Rc::new(SheetMetadata::new(links, merges)))
}
