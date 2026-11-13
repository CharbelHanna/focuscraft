//! FocusCraft — focus stacking for photographs shot at different focal planes.
//!
//! # The pipeline's governing rule
//!
//! Everything between loading and writing happens in **unclamped scene-linear `f32`**.
//! Files are linearised on load with their own transfer function; results are encoded on
//! write with exactly the curve the consumer will decode with. No stage in between
//! applies a tone curve or a clamp.
//!
//! This is not stylistic. The predecessor project shipped a bug where frames were
//! display-encoded (gamma 2.38, clamped at 1.0), merged, written as a 16-bit TIFF, and
//! then read back with the sRGB curve — so stacks opened darker and flatter than their
//! own inputs, and every highlight above scene-linear 0.759 was already gone before the
//! merge began. [`color`] exists so there is exactly one place that can reintroduce it.
//!
//! # Layout
//!
//! - [`color`] — transfer functions; the only module that applies a tone curve.
//! - [`frame`] — [`LinearRgb`], the scene-linear buffer every stage operates on.
//! - [`io`] — decoding files into scene-linear, and encoding results back out.

pub mod color;
pub mod frame;
pub mod io;

pub use color::TransferFunction;
pub use frame::{FrameStats, LinearRgb};
pub use io::{LoadedFrame, OutputFormat, OutputSpec, SourceInfo, WriteReport};

/// The crate version, as reported by `focuscraft --version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
