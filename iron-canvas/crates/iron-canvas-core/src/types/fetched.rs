//! [`Fetched`] — the outcome of a single content fetch against the host model.

/// Outcome of a single content fetch against the host model. Replaces the
/// overloaded `Option<T>` on the content accessors: `None` used to mean three
/// different things; each is now a named variant the caller must handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched<T> {
    /// The model answered with a concrete value.
    Value(T),
    /// Legitimately nothing — blank cell, empty value, no decoration, or an
    /// engine without the capability. All collapse here because the renderer
    /// paints them identically: there is nothing to draw.
    Absent,
    /// Transient bridge failure (JS threw / payload didn't deserialize). The
    /// caller MUST NOT treat this as empty — keep the prior frame's pixels and
    /// re-query next frame.
    BridgeFailed,
}

impl<T> Fetched<T> {
    /// Collapse to `Option`, discarding the `Absent`/`BridgeFailed` distinction.
    /// The escape hatch for callers that genuinely don't care — audit each use,
    /// since erasing the distinction is exactly the old trap.
    pub fn value(self) -> Option<T> {
        match self {
            Fetched::Value(v) => Some(v),
            Fetched::Absent | Fetched::BridgeFailed => None,
        }
    }

    pub fn unwrap_or(self, default: T) -> T {
        match self {
            Fetched::Value(v) => v,
            Fetched::Absent | Fetched::BridgeFailed => default,
        }
    }

    pub fn is_bridge_failed(&self) -> bool {
        matches!(self, Fetched::BridgeFailed)
    }

    /// Take the value out of a `&mut` slot, leaving `Absent` behind, and
    /// collapse to `Option`. Mirrors [`Option::take`] for the take-able
    /// pane-cache scratch buffers, where the consumed slot is overwritten by
    /// the next frame's fetch. Collapses `BridgeFailed` to `None` like
    /// [`Self::value`] — the hold decision happens in the preflight, upstream.
    pub fn take_value(&mut self) -> Option<T> {
        std::mem::replace(self, Fetched::Absent).value()
    }
}

// Deliberately no `From<Option<T>>`: forcing explicit construction is the point.
// An implicit bridge would re-introduce the ambiguity `Fetched` exists to kill —
// the caller could no longer see whether a `None` meant "absent" or "failed".

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_keeps_only_the_concrete_answer() {
        assert_eq!(Fetched::Value(7).value(), Some(7));
        assert_eq!(Fetched::<i32>::Absent.value(), None);
        assert_eq!(Fetched::<i32>::BridgeFailed.value(), None);
    }

    #[test]
    fn unwrap_or_substitutes_for_both_empty_kinds() {
        assert_eq!(Fetched::Value(7).unwrap_or(0), 7);
        assert_eq!(Fetched::Absent.unwrap_or(0), 0);
        assert_eq!(Fetched::BridgeFailed.unwrap_or(0), 0);
    }

    #[test]
    fn is_bridge_failed_is_exclusive_to_the_failure_variant() {
        assert!(Fetched::<i32>::BridgeFailed.is_bridge_failed());
        assert!(!Fetched::<i32>::Absent.is_bridge_failed());
        assert!(!Fetched::Value(7).is_bridge_failed());
    }
}
