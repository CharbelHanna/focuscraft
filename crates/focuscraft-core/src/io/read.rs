//! Decoding files into scene-linear `f32`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::DynamicImage;
use rayon::prelude::*;

use crate::color::TransferFunction;
use crate::frame::LinearRgb;

/// How samples are stored in the source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleKind {
    UInt8,
    UInt16,
    Float32,
}

impl SampleKind {
    pub fn bits(self) -> u8 {
        match self {
            Self::UInt8 => 8,
            Self::UInt16 => 16,
            Self::Float32 => 32,
        }
    }

    /// The transfer function assumed when the file does not state one.
    ///
    /// No common interchange format reliably carries its transfer function, so this is a
    /// convention, not a detection: float files are taken to be scene-linear, integer
    /// files to be sRGB-encoded. Both are the overwhelmingly common cases, and both can
    /// be overridden per-load. The assumption is reported in [`SourceInfo::assumed`] so
    /// callers can surface it rather than let it stay invisible.
    pub fn conventional_transfer(self) -> TransferFunction {
        match self {
            Self::Float32 => TransferFunction::Linear,
            Self::UInt8 | Self::UInt16 => TransferFunction::Srgb,
        }
    }
}

/// What a file turned out to contain.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceInfo {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub sample: SampleKind,
    /// The transfer function used to linearise this file.
    pub transfer: TransferFunction,
    /// True when `transfer` came from convention rather than from the caller.
    pub assumed: bool,
}

/// A frame linearised and ready for the pipeline.
#[derive(Debug, Clone)]
pub struct LoadedFrame {
    pub image: LinearRgb,
    pub info: SourceInfo,
}

/// Reads a file's shape without decoding its pixels.
pub fn probe(path: impl AsRef<Path>) -> Result<SourceInfo> {
    let path = path.as_ref();
    let reader = image::ImageReader::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("identifying the format of {}", path.display()))?;
    let (width, height) = reader
        .into_dimensions()
        .with_context(|| format!("reading the dimensions of {}", path.display()))?;

    // Sample depth is not available without decoding, so probe reports the conservative
    // default; `load` fills in the real one.
    Ok(SourceInfo {
        path: path.to_path_buf(),
        width,
        height,
        sample: SampleKind::UInt8,
        transfer: SampleKind::UInt8.conventional_transfer(),
        assumed: true,
    })
}

/// Decodes a file and linearises it into scene-linear `f32` RGB.
///
/// `transfer_override` forces the transfer function used to linearise; `None` takes the
/// convention for the file's sample type (see [`SampleKind::conventional_transfer`]).
pub fn load(
    path: impl AsRef<Path>,
    transfer_override: Option<TransferFunction>,
) -> Result<LoadedFrame> {
    let path = path.as_ref();
    let decoded = image::ImageReader::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("identifying the format of {}", path.display()))?
        .decode()
        .with_context(|| format!("decoding {}", path.display()))?;

    let sample = sample_kind(&decoded);
    let transfer = transfer_override.unwrap_or_else(|| sample.conventional_transfer());

    let width = decoded.width();
    let height = decoded.height();

    // `to_rgb32f` rescales integer samples to 0..=1 without touching their tone curve —
    // it is a pure numeric conversion. Linearisation is ours to do, below, and the test
    // `eight_bit_samples_are_scaled_not_gamma_corrected` pins that assumption.
    let mut data = decoded.to_rgb32f().into_raw();
    data.par_iter_mut().for_each(|v| *v = transfer.decode(*v));

    Ok(LoadedFrame {
        image: LinearRgb::from_vec(width, height, data),
        info: SourceInfo {
            path: path.to_path_buf(),
            width,
            height,
            sample,
            transfer,
            assumed: transfer_override.is_none(),
        },
    })
}

fn sample_kind(img: &DynamicImage) -> SampleKind {
    match img {
        DynamicImage::ImageLuma8(_)
        | DynamicImage::ImageLumaA8(_)
        | DynamicImage::ImageRgb8(_)
        | DynamicImage::ImageRgba8(_) => SampleKind::UInt8,
        DynamicImage::ImageLuma16(_)
        | DynamicImage::ImageLumaA16(_)
        | DynamicImage::ImageRgb16(_)
        | DynamicImage::ImageRgba16(_) => SampleKind::UInt16,
        DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_) => SampleKind::Float32,
        // `DynamicImage` is `#[non_exhaustive]`; an unknown variant is safest treated as
        // 8-bit, which only affects the assumed transfer function.
        _ => SampleKind::UInt8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    /// Pins the assumption that `image`'s f32 conversion is a pure rescale. If a future
    /// version of the crate starts applying sRGB itself, every frame would be linearised
    /// twice — and this test is what catches it.
    #[test]
    fn eight_bit_samples_are_scaled_not_gamma_corrected() {
        let dir = temp_dir();
        let path = dir.path().join("grey.png");
        let img: ImageBuffer<Rgb<u8>, _> = ImageBuffer::from_pixel(2, 2, Rgb([128, 128, 128]));
        img.save(&path).unwrap();

        // Loaded as Linear, the value must be exactly 128/255 — no curve applied.
        let frame = load(&path, Some(TransferFunction::Linear)).unwrap();
        let px = frame.image.pixel(0, 0);
        assert!((px[0] - 128.0 / 255.0).abs() < 1e-6, "got {}", px[0]);

        // Loaded conventionally, it must be sRGB-decoded.
        let frame = load(&path, None).unwrap();
        let expected = TransferFunction::Srgb.decode(128.0 / 255.0);
        assert!((frame.image.pixel(0, 0)[0] - expected).abs() < 1e-6);
        assert_eq!(frame.info.transfer, TransferFunction::Srgb);
        assert!(frame.info.assumed);
    }

    #[test]
    fn reports_sample_depth_and_dimensions() {
        let dir = temp_dir();

        let png8 = dir.path().join("a.png");
        ImageBuffer::<Rgb<u8>, _>::from_pixel(4, 3, Rgb([10, 20, 30]))
            .save(&png8)
            .unwrap();
        let info = load(&png8, None).unwrap().info;
        assert_eq!((info.width, info.height), (4, 3));
        assert_eq!(info.sample, SampleKind::UInt8);

        let png16 = dir.path().join("b.png");
        ImageBuffer::<Rgb<u16>, _>::from_pixel(4, 3, Rgb([1000, 2000, 3000]))
            .save(&png16)
            .unwrap();
        let info = load(&png16, None).unwrap().info;
        assert_eq!(info.sample, SampleKind::UInt16);
        assert_eq!(info.sample.bits(), 16);
    }

    #[test]
    fn override_is_recorded_as_not_assumed() {
        let dir = temp_dir();
        let path = dir.path().join("c.png");
        ImageBuffer::<Rgb<u8>, _>::from_pixel(1, 1, Rgb([200, 200, 200]))
            .save(&path)
            .unwrap();
        let frame = load(&path, Some(TransferFunction::Gamma { exponent: 2.2 })).unwrap();
        assert!(!frame.info.assumed);
        assert_eq!(
            frame.info.transfer,
            TransferFunction::Gamma { exponent: 2.2 }
        );
    }

    #[test]
    fn probe_reads_dimensions_without_decoding() {
        let dir = temp_dir();
        let path = dir.path().join("d.png");
        ImageBuffer::<Rgb<u8>, _>::from_pixel(7, 5, Rgb([0, 0, 0]))
            .save(&path)
            .unwrap();
        let info = probe(&path).unwrap();
        assert_eq!((info.width, info.height), (7, 5));
    }

    #[test]
    fn missing_file_is_an_error_naming_the_path() {
        let err = load("definitely-not-here.png", None).unwrap_err();
        assert!(
            format!("{err}").contains("definitely-not-here.png"),
            "{err}"
        );
    }
}
