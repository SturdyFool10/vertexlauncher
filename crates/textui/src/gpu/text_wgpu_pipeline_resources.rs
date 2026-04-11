use super::*;

pub(super) struct TextWgpuPipelineResources {
    pub(super) target_format: wgpu::TextureFormat,
    pub(super) atlas_sampling: TextAtlasSampling,
    /// When true, atlas textures use `Rgba8UnormSrgb` (auto sRGB→linear decode on sample)
    /// and the shader applies a manual sRGB encode before writing to non-sRGB surfaces.
    /// This is the physically-correct color pipeline for HDR-aware rendering.
    pub(super) linear_pipeline: bool,
    /// When true, outputting to HDR surface - shader passes through in scene-linear space.
    /// When false, applies tone mapping + sRGB encode for SDR output.
    pub(super) output_is_hdr: bool,
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) texture_bind_group_layout: wgpu::BindGroupLayout,
    pub(super) sampler: wgpu::Sampler,
    pub(super) uniform_buffer: wgpu::Buffer,
    pub(super) uniform_bind_group: wgpu::BindGroup,
}
