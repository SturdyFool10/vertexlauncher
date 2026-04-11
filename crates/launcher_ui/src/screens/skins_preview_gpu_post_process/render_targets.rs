use super::*;

/// Couples an MSAA (or single-sample) depth render texture with its view.
pub(super) struct DepthAttachmentSet {
    /// Depth texture used as a render attachment. May be MSAA (samples > 1).
    pub(super) _render_texture: wgpu::Texture,
    pub(super) render_view: wgpu::TextureView,
}

impl DepthAttachmentSet {
    pub(super) fn new(render_texture: wgpu::Texture, render_view: wgpu::TextureView) -> Self {
        Self {
            _render_texture: render_texture,
            render_view,
        }
    }
}

pub(super) struct SkinPreviewPostProcessRenderTargets {
    pub(super) accumulation_texture: wgpu::Texture,
    pub(super) accumulation_view: wgpu::TextureView,
    pub(super) accumulation_bind_group: wgpu::BindGroup,
    pub(super) scene_resolve_texture: wgpu::Texture,
    pub(super) scene_resolve_view: wgpu::TextureView,
    pub(super) scene_resolve_bind_group: wgpu::BindGroup,
    pub(super) scene_msaa_texture: Option<wgpu::Texture>,
    pub(super) scene_msaa_view: Option<wgpu::TextureView>,
    pub(super) scene_depth: DepthAttachmentSet,
    pub(super) scene_depth_linear_texture: wgpu::Texture,
    pub(super) scene_depth_linear_view: wgpu::TextureView,
    pub(super) scene_depth_linear_bind_group: wgpu::BindGroup,
    pub(super) scene_depth_linear_msaa_view: Option<wgpu::TextureView>,
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
