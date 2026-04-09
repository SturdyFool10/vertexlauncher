use serde::{Deserialize, Serialize};

/// Rendering path selection for text rendering backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextRenderingPath {
    /// Automatically select best rendering path based on context.
    Auto,
    /// Simple alpha mask rendering without distance fields.
    AlphaMask,
    /// Signed distance field rendering for scalable text.
    Sdf,
    /// Modified signed distance field with improved edge quality.
    Msdf,
}

impl TextRenderingPath {
    /// Array of all available text rendering paths in preference order.
    pub const ALL: [TextRenderingPath; 4] = [
        TextRenderingPath::Auto,
        TextRenderingPath::AlphaMask,
        TextRenderingPath::Sdf,
        TextRenderingPath::Msdf,
    ];

    /// Human-readable label for UI display.
    pub const fn label(self) -> &'static str {
        match self {
            TextRenderingPath::Auto => "Auto",
            TextRenderingPath::AlphaMask => "Alpha Mask",
            TextRenderingPath::Sdf => "SDF",
            TextRenderingPath::Msdf => "MSDF",
        }
    }
}
