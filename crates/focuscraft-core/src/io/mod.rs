//! Loading frames into scene-linear `f32`, and writing results back out.
//!
//! The two halves of this module are deliberate mirrors: [`read::load`] applies a
//! transfer function's `decode`, [`write::write`] applies the matching `encode`. The
//! output's transfer function is always an explicit choice recorded in [`OutputSpec`],
//! never something inherited from whichever file happened to be read first.

pub mod read;
pub mod write;

pub use read::{LoadedFrame, SampleKind, SourceInfo, load, probe};
pub use write::{OutputFormat, OutputSpec, WriteReport, write};
