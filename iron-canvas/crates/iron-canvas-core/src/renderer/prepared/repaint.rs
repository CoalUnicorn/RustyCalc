use crate::frame::GridVerdict;
use crate::frame::work::RowSpan;
use crate::renderer::cache::fingerprint::GridFingerprint;
use crate::renderer::repaint::envelope as repaint;
use crate::renderer::repaint::plan::RepaintReason;
#[cfg(feature = "dev-diagnostics")]
use crate::types::coord::RCRange;

pub(crate) enum PreparedFingerprintUpdate {
    Install(GridFingerprint),
    MarkStale,
}

/// Grid-wide repaint decision completed with the data each variant needs
/// during execution. `Cell` and `Range` own their required repaint
/// envelope, so no arm unwraps an `Option` and no arm exists only to
/// assert an impossible state. The `Cell` and `Range` variants stay
/// separate even though they use the same painter: `GridVerdict`
/// distinguishes a single changed cell from a changed range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreparedRepaintPlan {
    Skip,
    Cell { envelope: repaint::Envelope },
    Range { envelope: repaint::Envelope },
    Rows(Vec<RowSpan>),
    Full,
}

impl From<&PreparedRepaintPlan> for GridVerdict {
    fn from(plan: &PreparedRepaintPlan) -> Self {
        match plan {
            PreparedRepaintPlan::Skip => Self::Skip,
            PreparedRepaintPlan::Cell { .. } => Self::Cell,
            PreparedRepaintPlan::Range { .. } => Self::Range,
            PreparedRepaintPlan::Rows(spans) => Self::Rows {
                spans: spans.len().min(u8::MAX as usize) as u8,
                rows: spans
                    .iter()
                    .map(|span| (span.end() - span.start() + 1).max(0) as u32)
                    .sum::<u32>()
                    .min(u16::MAX as u32) as u16,
            },
            PreparedRepaintPlan::Full => Self::Full,
        }
    }
}

pub(crate) struct PreparedRepaint {
    pub(crate) plan: PreparedRepaintPlan,
    pub(crate) candidate: GridFingerprint,
    /// `Some` only when the fingerprint comparison ran. Fresh-built
    /// geometry repaints `Full` without a comparison — its authority is
    /// the attempt's `RebuildReason`, not a fingerprint branch. Read only
    /// by the dev-diagnostics recorder; unread in feature-off builds.
    #[cfg_attr(not(feature = "dev-diagnostics"), allow(dead_code))]
    pub(crate) reason: Option<RepaintReason>,
    #[cfg(feature = "dev-diagnostics")]
    pub(crate) changed_rows: Vec<RowSpan>,
    #[cfg(feature = "dev-diagnostics")]
    pub(crate) changed_cells: Vec<RCRange>,
}
