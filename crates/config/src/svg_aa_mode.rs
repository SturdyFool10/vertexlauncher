use serde::{Deserialize, Serialize};

/// Anti-aliasing mode selection for SVG rendering.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SvgAaMode {
    /// No anti-aliasing applied.
    Off,
    /// Balanced quality with 2x supersampling.
    Balanced,
    /// Crisp edges with 3x supersampling.
    Crisp,
    /// Ultra high quality with 4x supersampling.
    Ultra,
}

impl SvgAaMode {
    /// Array of all available SVG AA modes in preference order.
    pub const ALL: [SvgAaMode; 4] = [
        SvgAaMode::Balanced,
        SvgAaMode::Crisp,
        SvgAaMode::Ultra,
        SvgAaMode::Off,
    ];

    /// Human-readable label for UI display.
    pub const fn label(self) -> &'static str {
        match self {
            SvgAaMode::Off => "Off",
            SvgAaMode::Balanced => "Balanced (SSAA 2x)",
            SvgAaMode::Crisp => "Crisp (SSAA 3x)",
            SvgAaMode::Ultra => "Ultra (SSAA 4x)",
        }
    }

    /// Returns the supersampling scale factor for this AA mode.
    pub const fn supersample_scale(self) -> u32 {
        match self {
            SvgAaMode::Off => 1,
            SvgAaMode::Balanced => 2,
            SvgAaMode::Crisp => 3,
            SvgAaMode::Ultra => 4,
        }
    }
}
