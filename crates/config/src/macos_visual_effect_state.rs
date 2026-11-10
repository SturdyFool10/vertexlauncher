use vertex_i18n::src;
use serde::{Deserialize, Serialize};

/// NSVisualEffectView state selection for macOS blur/backdrop effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MacosVisualEffectState {
    FollowsWindowActiveState,
    Active,
    Inactive,
}

impl MacosVisualEffectState {
    pub const ALL: [MacosVisualEffectState; 3] = [
        MacosVisualEffectState::FollowsWindowActiveState,
        MacosVisualEffectState::Active,
        MacosVisualEffectState::Inactive,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            MacosVisualEffectState::FollowsWindowActiveState => src!("Follows Window Active State"),
            MacosVisualEffectState::Active => src!("Active"),
            MacosVisualEffectState::Inactive => src!("Inactive"),
        }
    }
}
