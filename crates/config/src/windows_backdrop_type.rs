use vertex_i18n::src;
use serde::{Deserialize, Serialize};

/// Backdrop type selection for Windows window appearance effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsBackdropType {
    /// Automatically select best backdrop type based on system capabilities.
    Auto,
    /// Explicitly disable DWM system backdrop material.
    None,
    /// Mica effect - uses app theme colors with transparency.
    Mica,
    /// Acrylic effect - frosted glass appearance with blur.
    Acrylic,
    /// Alternative mica effect variant.
    MicaAlt,
    /// Legacy blur effect for older Windows versions.
    LegacyBlur,
    /// The older acrylic (accent policy) backdrop, which takes a tint colour and strength.
    LegacyAcrylic,
}

impl WindowsBackdropType {
    /// Array of all available Windows backdrop types in preference order.
    pub const ALL: [WindowsBackdropType; 7] = [
        WindowsBackdropType::Auto,
        WindowsBackdropType::None,
        WindowsBackdropType::Mica,
        WindowsBackdropType::Acrylic,
        WindowsBackdropType::MicaAlt,
        WindowsBackdropType::LegacyBlur,
        WindowsBackdropType::LegacyAcrylic,
    ];

    /// Human-readable label for UI display.
    pub const fn label(self) -> &'static str {
        match self {
            WindowsBackdropType::Auto => src!("Auto"),
            WindowsBackdropType::None => src!("None"),
            WindowsBackdropType::Mica => src!("Mica"),
            WindowsBackdropType::Acrylic => src!("Acrylic"),
            WindowsBackdropType::MicaAlt => src!("Mica Alt"),
            WindowsBackdropType::LegacyBlur => src!("Legacy Blur"),
            WindowsBackdropType::LegacyAcrylic => src!("Legacy Acrylic"),
        }
    }
}
