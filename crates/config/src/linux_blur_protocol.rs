use vertex_i18n::src;
use serde::{Deserialize, Serialize};

/// Linux compositor blur protocol preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxBlurProtocol {
    /// Prefer ext_background_effect_v1 blur support, falling back to KDE blur.
    Auto,
    /// Request blur through the standard Wayland ext_background_effect_v1 protocol.
    ExtBackgroundEffect,
    /// Request blur through KDE/KWin blur protocols or X11 blur hints.
    KdeBlur,
    /// Blur by the Better Blur DX KWin effect, on its own: no compositor blur protocol is used. The window stays
    /// translucent and the effect blurs it because its class is on the effect's window list.
    BetterBlurDx,
}

impl LinuxBlurProtocol {
    pub const ALL: [LinuxBlurProtocol; 4] = [
        LinuxBlurProtocol::Auto,
        LinuxBlurProtocol::ExtBackgroundEffect,
        LinuxBlurProtocol::KdeBlur,
        LinuxBlurProtocol::BetterBlurDx,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            LinuxBlurProtocol::Auto => src!("Auto"),
            LinuxBlurProtocol::ExtBackgroundEffect => src!("ext_background_effect_v1"),
            LinuxBlurProtocol::KdeBlur => src!("KDE Blur"),
            LinuxBlurProtocol::BetterBlurDx => src!("Better Blur DX"),
        }
    }
}
