use super::*;

pub(super) struct SkinPreviewPostProcessRenderTargets {
    pub(super) accumulation_texture: wgpu::Texture,
    pub(super) accumulation_view: wgpu::TextureView,
    pub(super) accumulation_bind_group: wgpu::BindGroup,
    pub(super) scene_resolve_texture: wgpu::Texture,
    pub(super) scene_resolve_view: wgpu::TextureView,
    pub(super) scene_resolve_bind_group: wgpu::BindGroup,
    pub(super) scene_msaa_texture: Option<wgpu::Texture>,
    pub(super) scene_msaa_view: Option<wgpu::TextureView>,
    pub(super) scene_depth_texture: wgpu::Texture,
    pub(super) scene_depth_view: wgpu::TextureView,
    pub(super) scene_depth_bind_group: wgpu::BindGroup,
    pub(super) post_process_texture: wgpu::Texture,
    pub(super) post_process_view: wgpu::TextureView,
    pub(super) post_process_bind_group: wgpu::BindGroup,
    pub(super) taa_history_texture: wgpu::Texture,
    pub(super) taa_history_view: wgpu::TextureView,
    pub(super) taa_history_bind_group: wgpu::BindGroup,
    pub(super) taa_history_valid: bool,
    pub(super) render_target_size: [u32; 2],
    pub(super) present_source: PresentSource,
}
