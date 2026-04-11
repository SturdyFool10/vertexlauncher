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
    /// When true, atlas textures and shading stay in linear light until the
    /// final output transform.
    pub linear_pipeline: bool,
    /// When true, outputting to HDR surface - shader passes through in scene-linear space.
    /// When false, applies tone mapping + sRGB encode for SDR output.
    pub output_is_hdr: bool,
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
            linear_pipeline: false,
            output_is_hdr: false,
        }
    }
}
