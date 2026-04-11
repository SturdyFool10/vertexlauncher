use super::*;

pub(super) struct SkinPreviewPostProcessShaderModules {
    pub(super) scene_pipeline: wgpu::RenderPipeline,
    pub(super) accumulate_pipeline: wgpu::RenderPipeline,
    pub(super) ssao_pipeline: wgpu::RenderPipeline,
    pub(super) smaa_pipeline: wgpu::RenderPipeline,
    pub(super) fxaa_pipeline: wgpu::RenderPipeline,
    pub(super) taa_pipeline: wgpu::RenderPipeline,
    pub(super) present_pipeline: wgpu::RenderPipeline,
    pub(super) texture_bind_group_layout: wgpu::BindGroupLayout,
    pub(super) texture_sampler: wgpu::Sampler,
}
