//! Generates the small test images in `testdata/`.
//!
//! Run from the repository root:
//!
//! ```text
//! cargo run -p focuscraft-core --example make-testdata
//! ```
//!
//! The files are committed, so this only needs re-running when the set changes. It
//! exists because the images must have *known* values — a gradient whose encoded samples
//! are exactly `x/255`, and a float image holding genuine above-1.0 highlights — so that
//! tests and manual inspection can assert against arithmetic rather than against whatever
//! a photo happened to contain.

use std::path::Path;

use anyhow::Result;
use focuscraft_core::color::TransferFunction;
use focuscraft_core::frame::LinearRgb;
use focuscraft_core::io::{self, OutputFormat, OutputSpec};

fn main() -> Result<()> {
    let dir = Path::new("testdata");
    std::fs::create_dir_all(dir)?;

    write_srgb_gradient(&dir.join("gradient-srgb.png"))?;
    write_linear_highlights(&dir.join("highlights-linear.tif"))?;

    println!("wrote test images to {}", dir.display());
    Ok(())
}

/// A horizontal ramp, authored in *encoded* sRGB so the stored samples are exact.
fn write_srgb_gradient(path: &Path) -> Result<()> {
    const W: u32 = 256;
    const H: u32 = 32;

    let mut img = LinearRgb::new(W, H);
    for y in 0..H {
        for x in 0..W {
            // The value we want in the file, before linearisation.
            let encoded = x as f32 / (W - 1) as f32;
            let linear = TransferFunction::Srgb.decode(encoded);
            img.set_pixel(x, y, [linear, linear, linear]);
        }
    }

    let report = io::write(path, &img, OutputSpec::conventional(OutputFormat::Png8))?;
    println!(
        "{}: {W}x{H} 8-bit sRGB ramp, clipped {} high / {} low",
        path.display(),
        report.clipped_high,
        report.clipped_low
    );
    Ok(())
}

/// Scene-linear content reaching 8.0 — four stops above the point where an integer
/// format gives up, and well past the 0.759 where the predecessor's clamp destroyed data.
fn write_linear_highlights(path: &Path) -> Result<()> {
    const W: u32 = 64;
    const H: u32 = 64;

    let mut img = LinearRgb::new(W, H);
    for y in 0..H {
        for x in 0..W {
            // Left half: an ordinary 18% grey. Right half: a bright specular highlight.
            let linear = if x < W / 2 { 0.18 } else { 8.0 };
            img.set_pixel(x, y, [linear, linear * 0.9, linear * 0.8]);
        }
    }

    let report = io::write(path, &img, OutputSpec::conventional(OutputFormat::TiffF32))?;
    assert!(!report.clipped_any(), "float output must never clip");
    println!(
        "{}: {W}x{H} 32-bit linear, max {:.2} scene-linear",
        path.display(),
        report.max_linear
    );
    Ok(())
}
