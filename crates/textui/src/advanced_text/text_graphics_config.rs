use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGraphicsConfig {
    pub renderer_backend: TextRendererBackend,
    pub atlas_sampling: TextAtlasSampling,
    pub atlas_page_target_px: usize,
    pub atlas_padding_px: usize,
    pub graphics_api: TextGraphicsApi,
    pub gpu_power_preference: TextGpuPowerPreference,
    pub rasterization: TextRasterizationConfig,
}

impl Default for TextGraphicsConfig {
    fn default() -> Self {
        Self {
            renderer_backend: TextRendererBackend::Auto,
            atlas_sampling: TextAtlasSampling::Linear,
            atlas_page_target_px: 1024,
            atlas_padding_px: 1,
            graphics_api: TextGraphicsApi::Auto,
            gpu_power_preference: TextGpuPowerPreference::Auto,
            rasterization: TextRasterizationConfig::default(),
        }
    }
}
