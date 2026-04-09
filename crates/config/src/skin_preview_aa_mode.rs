use serde::{Deserialize, Serialize};

/// Anti-aliasing mode selection for skin preview rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkinPreviewAaMode {
    /// No anti-aliasing applied.
    Off,
    /// Multi-sample anti-aliasing at GPU level.
    Msaa,
    /// Spatial multi-sample anti-aliasing as post-process effect.
    Smaa,
    /// Fast approximate anti-aliasing as post-process effect.
    Fxaa,
    /// Temporal anti-aliasing using frame history.
    Taa,
    /// Combined FXAA and temporal anti-aliasing.
    FxaaTaa,
}

impl SkinPreviewAaMode {
    /// Array of all available skin preview AA modes in preference order.
    pub const ALL: [SkinPreviewAaMode; 6] = [
        SkinPreviewAaMode::Msaa,
        SkinPreviewAaMode::Smaa,
        SkinPreviewAaMode::Fxaa,
        SkinPreviewAaMode::Taa,
        SkinPreviewAaMode::FxaaTaa,
        SkinPreviewAaMode::Off,
    ];

    /// Human-readable label for UI display.
    pub const fn label(self) -> &'static str {
        match self {
            SkinPreviewAaMode::Off => "Off",
            SkinPreviewAaMode::Msaa => "MSAA (GPU)",
            SkinPreviewAaMode::Smaa => "SMAA (GPU Post)",
            SkinPreviewAaMode::Fxaa => "FXAA (Post)",
            SkinPreviewAaMode::Taa => "TAA (Temporal)",
            SkinPreviewAaMode::FxaaTaa => "FXAA + TAA (Temporal)",
        }
    }
}
