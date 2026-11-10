use vertex_i18n::src;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsAdapterPreferenceType {
    #[default]
    PerformanceProfile,
    ExplicitAdapter,
}

impl GraphicsAdapterPreferenceType {
    pub const ALL: [Self; 2] = [Self::PerformanceProfile, Self::ExplicitAdapter];

    pub const fn settings_label(self) -> &'static str {
        match self {
            Self::PerformanceProfile => src!("Performance Profile"),
            Self::ExplicitAdapter => src!("Explicit Adapter"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsAdapterProfile {
    Default,
    HighPerformance,
    LowPower,
    DiscreteOnly,
    IntegratedOnly,
}

impl GraphicsAdapterProfile {
    pub const ALL: [Self; 5] = [
        Self::Default,
        Self::HighPerformance,
        Self::LowPower,
        Self::DiscreteOnly,
        Self::IntegratedOnly,
    ];

    pub const fn settings_label(self) -> &'static str {
        match self {
            Self::Default => src!("Default"),
            Self::HighPerformance => src!("High Performance"),
            Self::LowPower => src!("Low Power"),
            Self::DiscreteOnly => src!("Discrete Only"),
            Self::IntegratedOnly => src!("Integrated Only"),
        }
    }
}

impl Default for GraphicsAdapterProfile {
    fn default() -> Self {
        Self::LowPower
    }
}
