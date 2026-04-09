use super::*;

pub(super) struct TextWgpuPipelineResources {
    pub(super) target_format: wgpu::TextureFormat,
    pub(super) atlas_sampling: TextAtlasSampling,
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) texture_bind_group_layout: wgpu::BindGroupLayout,
    pub(super) sampler: wgpu::Sampler,
    pub(super) uniform_buffer: wgpu::Buffer,
    pub(super) uniform_bind_group: wgpu::BindGroup,
}
