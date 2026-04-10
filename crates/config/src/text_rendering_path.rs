use serde::{Deserialize, Serialize};

/// Rendering path selection for text rendering pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextRenderingPath {
    /// Automatically select the best rendering path based on context.
    Auto,
    /// Use alpha mask rendering for text glyphs.
    AlphaMask,
    /// Use signed distance field (SDF) rendering for scalable text.
    Sdf,
    /// Use modified signed distance field (MSDF) for high-quality scaling.
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
