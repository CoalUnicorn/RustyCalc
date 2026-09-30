//! Worksheet render contracts: the engine-neutral, resolved grid data a frame
//! paints and queries.
//!
//! An adapter resolves engine values — IronCalc links, merged cells, colors —
//! into these types once, at the model boundary. Core never imports IronCalc,
//! XML, or a host type, and nothing here is a second document model: a value
//! that only an engine evaluation can produce (a formula-generated link, a
//! resolved series) arrives as an already-evaluated result.
//!
//! Scope is grid render data only. Drawing and chart schemas MUST NOT enter
//! this directory; they get their own canvas crates.

pub mod links;
pub mod merges;
pub mod snapshot;
