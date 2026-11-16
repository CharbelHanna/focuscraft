//! Encoding scene-linear results back out to files.
//!
//! This module owns the **only** clamp in the pipeline. Everything upstream carries
//! unclamped scene-linear values; quantising to an integer format is the one place where
//! a range limit genuinely exists, and when it bites, [`WriteReport`] says so out loud
//! instead of losing highlights quietly.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use anyhow::{Context, Result};
use image::{ImageBuffer, Rgb};

use crate::color::TransferFunction;
use crate::frame::LinearRgb;

/// Container and sample type for the written file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// 16-bit integer TIFF. The default: wide enough for editing, universally readable.
    Tiff16,
    /// 32-bit float TIFF. Keeps values outside 0..=1 exactly; nothing is clipped.
    TiffF32,
    /// 16-bit PNG.
    Png16,
    /// 8-bit PNG. For previews and contact sheets, not for editing.
    Png8,
}

impl OutputFormat {
    /// Whether this format can represent values outside 0..=1.
    pub fn is_high_dynamic_range(self) -> bool {
        matches!(self, Self::TiffF32)
    }

    /// The transfer function that suits this format when the caller has no preference.
    ///
    /// Float output defaults to scene-linear because its purpose is fidelity; integer
    /// output defaults to sRGB because its purpose is being opened by other software.
    pub fn conventional_transfer(self) -> TransferFunction {
        if self.is_high_dynamic_range() {
            TransferFunction::Linear
        } else {
            TransferFunction::Srgb
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Tiff16 | Self::TiffF32 => "tif",
            Self::Png16 | Self::Png8 => "png",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "tiff16" | "tif16" => Ok(Self::Tiff16),
            "tiff-f32" | "tif-f32" | "linear-f32-tiff" => Ok(Self::TiffF32),
            "png16" => Ok(Self::Png16),
            "png8" => Ok(Self::Png8),
            other => Err(format!(
                "unknown output format {other:?} (expected tiff16, tiff-f32, png16 or png8)"
            )),
        }
    }
}

/// A complete, explicit description of how to encode the output.
///
/// Both fields are stated rather than inferred. That is the whole point: the predecessor
/// bug happened because the output encoding was whatever the decode path left behind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputSpec {
    pub format: OutputFormat,
    /// The curve the *consumer* will decode with. The encoder applies exactly its inverse.
    pub transfer: TransferFunction,
}

impl OutputSpec {
    pub fn new(format: OutputFormat, transfer: TransferFunction) -> Self {
        Self { format, transfer }
    }

    /// The conventional pairing for a format.
    pub fn conventional(format: OutputFormat) -> Self {
        Self {
            format,
            transfer: format.conventional_transfer(),
        }
    }
}

/// What happened during quantisation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WriteReport {
    /// Samples above the format's maximum, clipped to white.
    pub clipped_high: usize,
    /// Samples below zero, clipped to black.
    pub clipped_low: usize,
    /// The largest scene-linear sample in the image, clipped or not.
    pub max_linear: f32,
}

impl WriteReport {
    pub fn clipped_any(&self) -> bool {
        self.clipped_high > 0 || self.clipped_low > 0
    }
}

/// Encodes and writes `image` to `path` according to `spec`.
pub fn write(path: impl AsRef<Path>, image: &LinearRgb, spec: OutputSpec) -> Result<WriteReport> {
    let path = path.as_ref();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating the directory {}", parent.display()))?;
    }

    let (width, height) = image.dims();
    let mut report = WriteReport {
        max_linear: f32::NEG_INFINITY,
        ..Default::default()
    };
    for &v in image.as_slice() {
        if v.is_finite() && v > report.max_linear {
            report.max_linear = v;
        }
    }
    if !report.max_linear.is_finite() {
        report.max_linear = 0.0;
    }

    match spec.format {
        OutputFormat::TiffF32 => {
            // Nothing is clipped here; that is this format's reason to exist.
            let data: Vec<f32> = image
                .as_slice()
                .iter()
                .map(|&v| spec.transfer.encode(v))
                .collect();
            let file =
                File::create(path).with_context(|| format!("creating {}", path.display()))?;
            let mut writer = BufWriter::new(file);
            let mut encoder = tiff::encoder::TiffEncoder::new(&mut writer)
                .with_context(|| format!("starting a TIFF for {}", path.display()))?;
            encoder
                .write_image::<tiff::encoder::colortype::RGB32Float>(width, height, &data)
                .with_context(|| format!("writing {}", path.display()))?;
        }
        OutputFormat::Tiff16 => {
            let data = quantise_u16(image, spec.transfer, &mut report);
            let file =
                File::create(path).with_context(|| format!("creating {}", path.display()))?;
            let mut writer = BufWriter::new(file);
            let mut encoder = tiff::encoder::TiffEncoder::new(&mut writer)
                .with_context(|| format!("starting a TIFF for {}", path.display()))?;
            encoder
                .write_image::<tiff::encoder::colortype::RGB16>(width, height, &data)
                .with_context(|| format!("writing {}", path.display()))?;
        }
        OutputFormat::Png16 => {
            let data = quantise_u16(image, spec.transfer, &mut report);
            let buffer: ImageBuffer<Rgb<u16>, _> = ImageBuffer::from_raw(width, height, data)
                .context("image buffer size did not match its dimensions")?;
            buffer
                .save(path)
                .with_context(|| format!("writing {}", path.display()))?;
        }
        OutputFormat::Png8 => {
            let data = quantise_u8(image, spec.transfer, &mut report);
            let buffer: ImageBuffer<Rgb<u8>, _> = ImageBuffer::from_raw(width, height, data)
                .context("image buffer size did not match its dimensions")?;
            buffer
                .save(path)
                .with_context(|| format!("writing {}", path.display()))?;
        }
    }

    Ok(report)
}

/// Encodes, clamps and quantises to 16-bit, counting what the clamp cost.
fn quantise_u16(
    image: &LinearRgb,
    transfer: TransferFunction,
    report: &mut WriteReport,
) -> Vec<u16> {
    image
        .as_slice()
        .iter()
        .map(|&linear| {
            let encoded = transfer.encode(linear);
            let clamped = clamp_counting(encoded, report);
            (clamped * u16::MAX as f32).round() as u16
        })
        .collect()
}

fn quantise_u8(image: &LinearRgb, transfer: TransferFunction, report: &mut WriteReport) -> Vec<u8> {
    image
        .as_slice()
        .iter()
        .map(|&linear| {
            let encoded = transfer.encode(linear);
            let clamped = clamp_counting(encoded, report);
            (clamped * u8::MAX as f32).round() as u8
        })
        .collect()
}

/// The single clamp in FocusCraft. Non-finite samples are written as black rather than
/// cast to an unpredictable integer.
#[inline]
fn clamp_counting(encoded: f32, report: &mut WriteReport) -> f32 {
    if !encoded.is_finite() {
        return 0.0;
    }
    if encoded > 1.0 {
        report.clipped_high += 1;
        1.0
    } else if encoded < 0.0 {
        report.clipped_low += 1;
        0.0
    } else {
        encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::load;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    /// The regression test for the bug that caused this project to exist: a value written
    /// with one curve and read back with the same curve must come back unchanged.
    #[test]
    fn mid_grey_survives_write_then_read() {
        let dir = temp_dir();
        let path = dir.path().join("grey.tif");

        let mut img = LinearRgb::new(2, 2);
        for y in 0..2 {
            for x in 0..2 {
                img.set_pixel(x, y, [0.18, 0.18, 0.18]);
            }
        }

        let spec = OutputSpec::conventional(OutputFormat::Tiff16);
        let report = write(&path, &img, spec).unwrap();
        assert!(
            !report.clipped_any(),
            "mid grey should not clip: {report:?}"
        );

        let reloaded = load(&path, Some(spec.transfer)).unwrap();
        let px = reloaded.image.pixel(0, 0);
        assert!(
            (px[0] - 0.18).abs() < 1e-3,
            "0.18 linear came back as {} — the write and read curves disagree",
            px[0]
        );
    }

    /// Writing with one curve and reading with another is the shape of the predecessor's
    /// bug. This asserts the mismatch really does corrupt the value, so that
    /// `mid_grey_survives_write_then_read` passing means something.
    ///
    /// The *direction* of the shift is not asserted, because it depends on which of the
    /// two curves is the stronger one: encoding with gamma 2.38 and decoding with sRGB
    /// (an overall ~2.2 power) leaves a net exponent below 1, which lifts mid-grey rather
    /// than lowering it. The predecessor was reported as opening darker, which points at
    /// its additional contrast line rather than at the gamma mismatch alone. What matters
    /// here, and what is tested, is that the value does not survive.
    #[test]
    fn mismatched_curves_corrupt_the_value() {
        let dir = temp_dir();
        let path = dir.path().join("mismatch.tif");

        let mut img = LinearRgb::new(1, 1);
        img.set_pixel(0, 0, [0.18, 0.18, 0.18]);

        write(
            &path,
            &img,
            OutputSpec::new(
                OutputFormat::Tiff16,
                TransferFunction::Gamma { exponent: 2.38 },
            ),
        )
        .unwrap();
        let wrong = load(&path, Some(TransferFunction::Srgb))
            .unwrap()
            .image
            .pixel(0, 0)[0];

        let relative_error = (wrong - 0.18).abs() / 0.18;
        assert!(
            relative_error > 0.05,
            "expected a mismatch to visibly shift 0.18, but got {wrong} ({:.1}% off)",
            relative_error * 100.0
        );
    }

    /// Float output must preserve highlights that integer output cannot.
    #[test]
    fn float_output_keeps_values_above_one() {
        let dir = temp_dir();
        let path = dir.path().join("hdr.tif");

        let mut img = LinearRgb::new(1, 1);
        img.set_pixel(0, 0, [4.0, 2.0, 1.5]);

        let spec = OutputSpec::conventional(OutputFormat::TiffF32);
        let report = write(&path, &img, spec).unwrap();
        assert!(
            !report.clipped_any(),
            "float output must not clip: {report:?}"
        );
        assert!((report.max_linear - 4.0).abs() < 1e-6);

        let reloaded = load(&path, Some(TransferFunction::Linear)).unwrap();
        let px = reloaded.image.pixel(0, 0);
        assert!((px[0] - 4.0).abs() < 1e-5, "got {}", px[0]);
        assert!((px[1] - 2.0).abs() < 1e-5, "got {}", px[1]);
    }

    /// Integer output does clip — but reports it rather than hiding it.
    #[test]
    fn integer_output_reports_what_it_clipped() {
        let dir = temp_dir();
        let path = dir.path().join("clipped.tif");

        let mut img = LinearRgb::new(2, 1);
        img.set_pixel(0, 0, [4.0, 4.0, 4.0]);
        img.set_pixel(1, 0, [-1.0, 0.5, 0.5]);

        let report = write(&path, &img, OutputSpec::conventional(OutputFormat::Tiff16)).unwrap();
        assert_eq!(report.clipped_high, 3, "{report:?}");
        assert_eq!(report.clipped_low, 1, "{report:?}");
        assert!(report.clipped_any());
        assert!((report.max_linear - 4.0).abs() < 1e-6);
    }

    #[test]
    fn non_finite_samples_are_written_as_black() {
        let dir = temp_dir();
        let path = dir.path().join("nan.png");

        let mut img = LinearRgb::new(1, 1);
        img.set_pixel(0, 0, [f32::NAN, f32::INFINITY, 0.0]);

        write(&path, &img, OutputSpec::conventional(OutputFormat::Png8)).unwrap();
        let px = load(&path, Some(TransferFunction::Linear))
            .unwrap()
            .image
            .pixel(0, 0);
        assert_eq!(px[0], 0.0);
    }

    #[test]
    fn creates_missing_parent_directories() {
        let dir = temp_dir();
        let path = dir.path().join("nested/deeper/out.png");
        let img = LinearRgb::new(1, 1);
        write(&path, &img, OutputSpec::conventional(OutputFormat::Png8)).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn format_names_parse() {
        assert_eq!(OutputFormat::parse("tiff16"), Ok(OutputFormat::Tiff16));
        assert_eq!(OutputFormat::parse("tiff-f32"), Ok(OutputFormat::TiffF32));
        assert_eq!(OutputFormat::parse("PNG8"), Ok(OutputFormat::Png8));
        assert!(OutputFormat::parse("jpeg").is_err());
    }
}
