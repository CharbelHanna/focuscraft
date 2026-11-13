//! The in-memory representation every stage of the pipeline works on.

use crate::color;

/// An image in **scene-linear** RGB, `f32` per channel, interleaved, unclamped.
///
/// Unclamped is load-bearing: values above 1.0 are real highlights, values below 0.0
/// arise from pyramid reconstruction, and both must survive to the encoder. Nothing in
/// the pipeline between [`crate::io::load`] and [`crate::io::write`] may clamp.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearRgb {
    width: u32,
    height: u32,
    /// `width * height * 3` samples, row-major, R G B interleaved.
    data: Vec<f32>,
}

impl LinearRgb {
    pub const CHANNELS: usize = 3;

    /// Creates an all-zero image.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0.0; Self::sample_count(width, height)],
        }
    }

    /// Wraps existing interleaved samples.
    ///
    /// # Panics
    /// If `data.len()` is not exactly `width * height * 3`.
    pub fn from_vec(width: u32, height: u32, data: Vec<f32>) -> Self {
        let expected = Self::sample_count(width, height);
        assert_eq!(
            data.len(),
            expected,
            "LinearRgb::from_vec: {width}x{height} needs {expected} samples, got {}",
            data.len()
        );
        Self {
            width,
            height,
            data,
        }
    }

    fn sample_count(width: u32, height: u32) -> usize {
        (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(Self::CHANNELS))
            .expect("image dimensions overflow usize")
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn dims(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        &mut self.data
    }

    pub fn into_vec(self) -> Vec<f32> {
        self.data
    }

    #[inline]
    fn index(&self, x: u32, y: u32) -> usize {
        debug_assert!(
            x < self.width && y < self.height,
            "({x},{y}) out of {}x{}",
            self.width,
            self.height
        );
        (y as usize * self.width as usize + x as usize) * Self::CHANNELS
    }

    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 3] {
        let i = self.index(x, y);
        [self.data[i], self.data[i + 1], self.data[i + 2]]
    }

    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, rgb: [f32; 3]) {
        let i = self.index(x, y);
        self.data[i] = rgb[0];
        self.data[i + 1] = rgb[1];
        self.data[i + 2] = rgb[2];
    }

    /// Iterates pixels as `[r, g, b]`, row-major.
    pub fn pixels(&self) -> impl Iterator<Item = &[f32; Self::CHANNELS]> {
        let (pixels, remainder) = self.data.as_chunks::<{ Self::CHANNELS }>();
        debug_assert!(
            remainder.is_empty(),
            "sample count is not a multiple of the channel count"
        );
        pixels.iter()
    }

    /// Scene-linear luminance statistics, for `focuscraft inspect`.
    pub fn stats(&self) -> FrameStats {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        let mut sum = 0.0f64;
        let mut above_one = 0usize;
        let mut below_zero = 0usize;
        let mut non_finite = 0usize;

        for px in self.pixels() {
            let y = color::luma(*px);
            if !y.is_finite() {
                non_finite += 1;
                continue;
            }
            min = min.min(y);
            max = max.max(y);
            sum += y as f64;
            if y > 1.0 {
                above_one += 1;
            }
            if y < 0.0 {
                below_zero += 1;
            }
        }

        let counted = self.pixel_count().saturating_sub(non_finite);
        FrameStats {
            min_luma: if counted == 0 { 0.0 } else { min },
            max_luma: if counted == 0 { 0.0 } else { max },
            mean_luma: if counted == 0 {
                0.0
            } else {
                (sum / counted as f64) as f32
            },
            pixels_above_one: above_one,
            pixels_below_zero: below_zero,
            non_finite,
        }
    }
}

/// Summary of a frame's scene-linear luminance, reported by `focuscraft inspect`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStats {
    pub min_luma: f32,
    pub max_luma: f32,
    pub mean_luma: f32,
    /// Pixels brighter than scene-linear 1.0 — highlights a clamping pipeline would lose.
    pub pixels_above_one: usize,
    pub pixels_below_zero: usize,
    pub non_finite: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_pixels() {
        let mut img = LinearRgb::new(3, 2);
        img.set_pixel(2, 1, [0.5, 2.0, -0.25]);
        assert_eq!(img.pixel(2, 1), [0.5, 2.0, -0.25]);
        assert_eq!(img.pixel(0, 0), [0.0, 0.0, 0.0]);
        assert_eq!(img.dims(), (3, 2));
        assert_eq!(img.as_slice().len(), 3 * 2 * 3);
    }

    #[test]
    #[should_panic(expected = "needs 18 samples")]
    fn rejects_mismatched_buffer() {
        LinearRgb::from_vec(3, 2, vec![0.0; 10]);
    }

    #[test]
    fn stats_count_out_of_range_pixels() {
        let mut img = LinearRgb::new(2, 1);
        img.set_pixel(0, 0, [4.0, 4.0, 4.0]);
        img.set_pixel(1, 0, [-1.0, -1.0, -1.0]);
        let s = img.stats();
        assert_eq!(s.pixels_above_one, 1);
        assert_eq!(s.pixels_below_zero, 1);
        assert!((s.max_luma - 4.0).abs() < 1e-5);
        assert!((s.min_luma + 1.0).abs() < 1e-5);
    }

    #[test]
    fn stats_report_non_finite_without_poisoning_the_mean() {
        let mut img = LinearRgb::new(2, 1);
        img.set_pixel(0, 0, [0.5, 0.5, 0.5]);
        img.set_pixel(1, 0, [f32::NAN, 0.0, 0.0]);
        let s = img.stats();
        assert_eq!(s.non_finite, 1);
        assert!((s.mean_luma - 0.5).abs() < 1e-5, "mean was {}", s.mean_luma);
    }
}
