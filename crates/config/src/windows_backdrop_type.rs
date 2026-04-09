use serde::{Deserialize, Serialize};

/// Backdrop type selection for Windows window appearance effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsBackdropType {
    /// Automatically select best backdrop type based on system capabilities.
    Auto,
    /// Mica effect - uses app theme colors with transparency.
    Mica,
    /// Acrylic effect - frosted glass appearance with blur.
    Acrylic,
    /// Alternative mica effect variant.
    MicaAlt,
    /// Legacy blur effect for older Windows versions.
    LegacyBlur,
}

impl WindowsBackdropType {
    /// Array of all available Windows backdrop types in preference order.
    pub const ALL: [WindowsBackdropType; 5] = [
        WindowsBackdropType::Auto,
        WindowsBackdropType::Mica,
        WindowsBackdropType::Acrylic,
        WindowsBackdropType::MicaAlt,
        WindowsBackdropType::LegacyBlur,
    ];

    /// Human-readable label for UI display.
    pub const fn label(self) -> &'static str {
        match self {
            WindowsBackdropType::Auto => "Auto",
            WindowsBackdropType::Mica => "Mica",
            WindowsBackdropType::Acrylic => "Acrylic",
            WindowsBackdropType::MicaAlt => "Mica Alt",
            WindowsBackdropType::LegacyBlur => "Legacy Blur",
        }
    }
}
