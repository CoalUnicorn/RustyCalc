//! The repaint decision layer: whether to repaint, and the envelope to
//! repaint. Separate from `cell/`, whose files paint cells.

pub(crate) mod envelope;
pub(crate) mod plan;
