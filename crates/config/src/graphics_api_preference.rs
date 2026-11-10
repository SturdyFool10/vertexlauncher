use vertex_i18n::src;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsApiPreference {
    #[default]
    Auto,
    Vulkan,
    Metal,
    Dx12,
    /// WebGPU (Dawn): reaches GPUs and platforms where neither Vulkan nor DirectX 12 works.
    #[serde(rename = "web_gpu")]
    WebGpu,
}

impl GraphicsApiPreference {
    pub const ALL: [Self; 5] = [Self::Auto, Self::Vulkan, Self::Metal, Self::Dx12, Self::WebGpu];

    pub const fn settings_label(self) -> &'static str {
        match self {
            Self::Auto => src!("Auto"),
            Self::Vulkan => src!("Vulkan"),
            Self::Metal => src!("Metal"),
            Self::Dx12 => src!("DirectX 12"),
            Self::WebGpu => src!("WebGPU"),
        }
    }
}
