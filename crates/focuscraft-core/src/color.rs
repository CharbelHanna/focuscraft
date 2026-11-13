//! Transfer functions — the only place in FocusCraft where a tone curve is applied.
//!
//! # Why this module is isolated
//!
//! FocusCraft exists partly because of a bug in its predecessor: frames were decoded,
//! put through a display encoding (gamma 2.38 plus a contrast line, clamped at 1.0),
//! merged, and written as a 16-bit TIFF — which the consumer then read back with the
//! *sRGB* curve. Two different curves, so the result opened darker and flatter than the
//! frames that made it. The clamp also destroyed every highlight above scene-linear
//! 0.759 before the merge even ran.
//!
//! The rule that follows, and that this module enforces:
//!
//! 1. Whatever curve the consumer decodes with, encode with exactly its inverse.
//!    [`TransferFunction::encode`] and [`TransferFunction::decode`] are exact inverses
//!    of each other for every variant, and are tested as such.
//! 2. Nothing clamps. Scene-linear values above 1.0 are real highlights and survive the
//!    whole pipeline. Clamping, if it must happen at all, happens once in the encoder
//!    for integer output formats, and nowhere else.
//!
//! Every curve is extended symmetrically about zero (`f(-x) == -f(x)`) so that negative
//! values — which arise legitimately from sharpening, from wide-gamut primaries, and
//! from pyramid reconstruction — round-trip instead of being silently crushed.

use serde::{Deserialize, Serialize};
use std::fmt;

/// sRGB piecewise threshold, on the encoded side.
const SRGB_ENCODED_THRESHOLD: f32 = 0.040_448_237;
/// sRGB piecewise threshold, on the linear side.
const SRGB_LINEAR_THRESHOLD: f32 = SRGB_ENCODED_THRESHOLD / SRGB_SLOPE;
/// Slope of the sRGB linear segment near black.
const SRGB_SLOPE: f32 = 12.92;
const SRGB_ALPHA: f32 = 0.055;
const SRGB_GAMMA: f32 = 2.4;

/// How pixel values in a file relate to scene-linear light.
///
/// "Decode" always means *encoded value → scene-linear*, and "encode" always means
/// *scene-linear → encoded value*, regardless of which direction a given file format
/// happens to store.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum TransferFunction {
    /// Values are already proportional to scene light. Decode and encode are identity.
    Linear,
    /// The sRGB transfer function (IEC 61966-2-1), including its near-black linear segment.
    Srgb,
    /// A pure power law. `exponent` is the *decoding* exponent, i.e. display gamma:
    /// `linear = encoded.powf(exponent)`. A file described as "gamma 2.2 encoded" has
    /// `Gamma { exponent: 2.2 }`.
    Gamma { exponent: f32 },
}

impl TransferFunction {
    /// Encoded value → scene-linear. Never clamps.
    pub fn decode(self, encoded: f32) -> f32 {
        signed(encoded, |v| match self {
            Self::Linear => v,
            Self::Srgb => {
                if v <= SRGB_ENCODED_THRESHOLD {
                    v / SRGB_SLOPE
                } else {
                    ((v + SRGB_ALPHA) / (1.0 + SRGB_ALPHA)).powf(SRGB_GAMMA)
                }
            }
            Self::Gamma { exponent } => v.powf(exponent),
        })
    }

    /// Scene-linear → encoded value. Never clamps; values above 1.0 encode to above 1.0.
    ///
    /// Clamping to a format's representable range is the writer's job, done once, at the
    /// point of quantisation — see `io::write`.
    pub fn encode(self, linear: f32) -> f32 {
        signed(linear, |v| match self {
            Self::Linear => v,
            Self::Srgb => {
                if v <= SRGB_LINEAR_THRESHOLD {
                    v * SRGB_SLOPE
                } else {
                    (1.0 + SRGB_ALPHA) * v.powf(1.0 / SRGB_GAMMA) - SRGB_ALPHA
                }
            }
            Self::Gamma { exponent } => v.powf(1.0 / exponent),
        })
    }

    /// Short stable name, used in CLI output and in project files.
    pub fn name(self) -> String {
        match self {
            Self::Linear => "linear".to_string(),
            Self::Srgb => "srgb".to_string(),
            Self::Gamma { exponent } => format!("gamma{exponent}"),
        }
    }

    /// Parses the names produced by [`TransferFunction::name`], for CLI arguments.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "linear" => Ok(Self::Linear),
            "srgb" => Ok(Self::Srgb),
            other => {
                let digits = other.strip_prefix("gamma").ok_or_else(|| {
                    format!(
                        "unknown transfer function {other:?} (expected linear, srgb or gamma<N>)"
                    )
                })?;
                let exponent: f32 = digits
                    .trim()
                    .parse()
                    .map_err(|_| format!("{digits:?} is not a valid gamma exponent"))?;
                if !(exponent.is_finite() && exponent > 0.0) {
                    return Err(format!(
                        "gamma exponent must be finite and positive, got {exponent}"
                    ));
                }
                Ok(Self::Gamma { exponent })
            }
        }
    }
}

impl fmt::Display for TransferFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

/// Applies `f` to the magnitude and restores the sign, so every curve is odd.
///
/// Without this, a negative input hits `powf` with a fractional exponent and returns
/// NaN — which then propagates silently through a merge.
#[inline]
fn signed(v: f32, f: impl Fn(f32) -> f32) -> f32 {
    if v < 0.0 { -f(-v) } else { f(v) }
}

/// Rec.709 / sRGB luminance weights, for use on **scene-linear** values.
///
/// These coefficients are only physically meaningful in linear light; applying them to
/// gamma-encoded values is a common and quiet mistake.
pub const LUMA_WEIGHTS: [f32; 3] = [0.212_639, 0.715_169, 0.072_192];

/// Scene-linear RGB → scene-linear luminance.
#[inline]
pub fn luma(rgb: [f32; 3]) -> f32 {
    rgb[0] * LUMA_WEIGHTS[0] + rgb[1] * LUMA_WEIGHTS[1] + rgb[2] * LUMA_WEIGHTS[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVES: [TransferFunction; 4] = [
        TransferFunction::Linear,
        TransferFunction::Srgb,
        TransferFunction::Gamma { exponent: 2.2 },
        TransferFunction::Gamma { exponent: 2.38 },
    ];

    /// The central guarantee: encode and decode are exact inverses, including well above
    /// 1.0 and below 0.0. This is the test that would have caught the original bug.
    #[test]
    fn encode_decode_round_trips() {
        let samples = [
            -4.0, -1.0, -0.25, -0.001, 0.0, 1e-6, 0.0031, 0.05, 0.18, 0.5, 0.759, 1.0, 1.5, 4.0,
            16.0, 1000.0,
        ];
        for curve in CURVES {
            for v in samples {
                let round_tripped = curve.decode(curve.encode(v));
                assert!(
                    (round_tripped - v).abs() <= 1e-4 * v.abs().max(1e-3),
                    "{curve:?}: {v} -> {} -> {round_tripped}",
                    curve.encode(v)
                );
            }
        }
    }

    /// Highlights above scene-linear 0.759 are exactly what the predecessor destroyed.
    #[test]
    fn highlights_above_the_old_clamp_point_survive() {
        for curve in CURVES {
            for v in [0.76, 0.9, 1.0, 2.5, 12.0] {
                let encoded = curve.encode(v);
                assert!(encoded.is_finite(), "{curve:?} encoded {v} to {encoded}");
                assert!(
                    curve.decode(encoded) > 0.759,
                    "{curve:?} clamped {v} down to {}",
                    curve.decode(encoded)
                );
            }
        }
    }

    /// Negative values must not become NaN — they arise from sharpening and from
    /// Laplacian pyramid reconstruction, and a NaN there poisons an entire merge.
    #[test]
    fn negatives_stay_finite_and_odd() {
        for curve in CURVES {
            for v in [-0.5f32, -1.0, -3.0] {
                assert!(
                    curve.encode(v).is_finite(),
                    "{curve:?} encode({v}) was not finite"
                );
                assert!(
                    curve.decode(v).is_finite(),
                    "{curve:?} decode({v}) was not finite"
                );
                assert!(curve.encode(v) < 0.0, "{curve:?} lost the sign of {v}");
                assert!(
                    (curve.encode(v) + curve.encode(-v)).abs() < 1e-6,
                    "{curve:?} is not odd about zero at {v}"
                );
            }
        }
    }

    /// The sRGB piecewise segments must meet, or there is a step at the join that shows
    /// up as banding in deep shadow.
    #[test]
    fn srgb_segments_are_continuous() {
        let curve = TransferFunction::Srgb;
        let below = curve.decode(SRGB_ENCODED_THRESHOLD - 1e-6);
        let above = curve.decode(SRGB_ENCODED_THRESHOLD + 1e-6);
        assert!(
            (below - above).abs() < 1e-6,
            "discontinuity: {below} vs {above}"
        );
    }

    /// Anchor points, so a refactor that silently swaps encode and decode fails here.
    #[test]
    fn known_srgb_values() {
        let curve = TransferFunction::Srgb;
        // Mid-grey 0.5 encoded is ~0.2140 linear.
        assert!(
            (curve.decode(0.5) - 0.214_041).abs() < 1e-4,
            "{}",
            curve.decode(0.5)
        );
        // 18% scene-linear grey encodes to ~0.4614.
        assert!(
            (curve.encode(0.18) - 0.461_354).abs() < 1e-4,
            "{}",
            curve.encode(0.18)
        );
        assert_eq!(curve.encode(0.0), 0.0);
        assert!((curve.encode(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn linear_is_identity() {
        for v in [-2.0, 0.0, 0.18, 1.0, 9.0] {
            assert_eq!(TransferFunction::Linear.encode(v), v);
            assert_eq!(TransferFunction::Linear.decode(v), v);
        }
    }

    #[test]
    fn names_round_trip_through_parse() {
        for curve in CURVES {
            assert_eq!(TransferFunction::parse(&curve.name()), Ok(curve));
        }
        assert!(TransferFunction::parse("nonsense").is_err());
        assert!(TransferFunction::parse("gamma0").is_err());
        assert!(TransferFunction::parse("gammaX").is_err());
    }

    #[test]
    fn luma_weights_sum_to_one() {
        let sum: f32 = LUMA_WEIGHTS.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5, "weights sum to {sum}");
        assert!((luma([1.0, 1.0, 1.0]) - 1.0).abs() < 1e-5);
    }
}
