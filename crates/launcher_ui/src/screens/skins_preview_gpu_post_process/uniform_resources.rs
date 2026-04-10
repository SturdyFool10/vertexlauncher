use super::*;

pub(super) struct SkinPreviewPostProcessUniformResources {
    pub(super) uniform_bind_group: wgpu::BindGroup,
    pub(super) uniform_buffer: wgpu::Buffer,
    pub(super) scalar_uniform_bind_group_layout: wgpu::BindGroupLayout,
    pub(super) scalar_uniform_bind_group: wgpu::BindGroup,
    pub(super) scalar_uniform_buffer: wgpu::Buffer,
}
