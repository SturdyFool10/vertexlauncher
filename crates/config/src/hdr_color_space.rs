use vertex_i18n::src;
use serde::{Deserialize, Serialize};

/// The signal the swapchain carries when HDR output is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HdrColorSpace {
    /// HDR10: Rec.2020 primaries with the PQ (ST 2084) curve. What most HDR monitors expect.
    Hdr10St2084,
    /// scRGB: linear extended-range Rec.709 values, where 1.0 is SDR white (Windows' native HDR format).
    ScrgbLinear,
    /// HDR10 with the HLG curve (broadcast-style HDR).
    Hdr10Hlg,
    /// Dolby Vision signalling, where the platform supports it.
    DolbyVision,
}

impl HdrColorSpace {
    pub const ALL: [HdrColorSpace; 4] =
        [HdrColorSpace::Hdr10St2084, HdrColorSpace::ScrgbLinear, HdrColorSpace::Hdr10Hlg, HdrColorSpace::DolbyVision];

    pub const fn label(self) -> &'static str {
        match self {
            HdrColorSpace::Hdr10St2084 => src!("HDR10 (PQ)"),
            HdrColorSpace::ScrgbLinear => src!("scRGB (linear)"),
            HdrColorSpace::Hdr10Hlg => src!("HDR10 (HLG)"),
            HdrColorSpace::DolbyVision => src!("Dolby Vision"),
        }
    }
}
