use serde::{Deserialize, Serialize};

/// Texel-level anti-aliasing mode for skin preview rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkinPreviewTexelAaMode {
    /// No texel anti-aliasing applied.
    Off,
    /// Anti-aliasing at texel boundaries for cleaner edges.
    TexelBoundary,
}

impl SkinPreviewTexelAaMode {
    /// Array of all available skin preview texel AA modes.
    pub const ALL: [SkinPreviewTexelAaMode; 2] = [
        SkinPreviewTexelAaMode::Off,
        SkinPreviewTexelAaMode::TexelBoundary,
    ];

    /// Human-readable label for UI display.
    pub const fn label(self) -> &'static str {
        match self {
            SkinPreviewTexelAaMode::Off => "Off",
            SkinPreviewTexelAaMode::TexelBoundary => "Texel Border AA",
        }
    }
}
