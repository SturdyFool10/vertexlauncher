use vertex_i18n::src;
use serde::{Deserialize, Serialize};

/// NSVisualEffectView blending mode selection for macOS blur/backdrop effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MacosVisualEffectBlendingMode {
    BehindWindow,
    WithinWindow,
}

impl MacosVisualEffectBlendingMode {
    pub const ALL: [MacosVisualEffectBlendingMode; 2] = [
        MacosVisualEffectBlendingMode::BehindWindow,
        MacosVisualEffectBlendingMode::WithinWindow,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            MacosVisualEffectBlendingMode::BehindWindow => src!("Behind Window"),
            MacosVisualEffectBlendingMode::WithinWindow => src!("Within Window"),
        }
    }
}
