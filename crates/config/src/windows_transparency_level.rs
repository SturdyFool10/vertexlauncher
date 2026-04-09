use serde::{Deserialize, Serialize};

/// Legacy transparency level for Windows window appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsTransparencyLevel {
    /// No transparency - fully opaque window background.
    Solid,
    /// Low transparency with 70% opacity.
    Low,
    /// Medium transparency with 50% opacity.
    Medium,
    /// High transparency with 30% opacity.
    High,
}

impl WindowsTransparencyLevel {
    /// Returns the UI opacity percentage corresponding to this transparency level.
    pub const fn ui_opacity_percent(self) -> u8 {
        match self {
            WindowsTransparencyLevel::Solid => 100,
            WindowsTransparencyLevel::Low => 70,
            WindowsTransparencyLevel::Medium => 50,
            WindowsTransparencyLevel::High => 30,
        }
    }
}
