use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsApiPreference {
    #[default]
    Auto,
    Vulkan,
    Metal,
    Dx12,
}

impl GraphicsApiPreference {
    pub const ALL: [Self; 4] = [Self::Auto, Self::Vulkan, Self::Metal, Self::Dx12];

    pub const fn settings_label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Vulkan => "Vulkan",
            Self::Metal => "Metal",
            Self::Dx12 => "DirectX 12",
        }
    }
}
