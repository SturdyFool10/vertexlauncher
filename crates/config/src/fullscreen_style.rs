use vertex_i18n::src;
use serde::{Deserialize, Serialize};

/// How the launcher goes fullscreen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullscreenStyle {
    /// The window covers the screen without changing the display mode. Switching in and out is instant.
    Borderless,
    /// The window takes over the display (and its video mode) exclusively, where the platform supports it.
    Exclusive,
}

impl FullscreenStyle {
    pub const ALL: [FullscreenStyle; 2] = [FullscreenStyle::Borderless, FullscreenStyle::Exclusive];

    pub const fn label(self) -> &'static str {
        match self {
            FullscreenStyle::Borderless => src!("Borderless"),
            FullscreenStyle::Exclusive => src!("Exclusive"),
        }
    }
}
