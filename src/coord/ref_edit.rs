//! Point-mode formula reference editing (deferred extraction).
//!
//! Currently most editing logic lives in `super::types::RefNode` inherent
//! methods.  Future extraction targets: `from_cell_area`, `with_area`,
//! `area`, `extend_trailing`, `extend_with_anchor`, `relocate_to`.
//!
//! Keeping these methods on `RefNode` is idiomatic — they're the type's
//! core behavior, not standalone functions.  Extraction to a separate
//! module would require `pub(crate)` accessors on `RefNode::inner` and
//! increases the public API surface.

// (stub — extraction deferred per EXT-2 "trigger: next time modified")
