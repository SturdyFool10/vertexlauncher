use super::*;

/// Couples an MSAA (or single-sample) depth render texture with its resolved 1× counterpart.
///
/// The `sample_bind_group` is **always** built from the 1× resolve view at construction time.
/// There is no separate assignment path, so it is structurally impossible to accidentally bind
/// the MSAA view to a non-multisampled bind-group layout.
pub(super) struct DepthAttachmentSet {
    /// Depth texture used as a render attachment. May be MSAA (samples > 1).
    pub(super) _render_texture: wgpu::Texture,
    pub(super) render_view: wgpu::TextureView,
    /// Always 1× depth texture. Populated by [`MsaaResolvePool::auto_resolve`] each frame
    /// when MSAA is active, or rendered to directly when samples == 1.
    _resolve_texture: wgpu::Texture,
    _resolve_view: wgpu::TextureView,
    /// Bind group that always references `resolve_view`. Safe to pass to any shader layout
    /// with `multisampled = false`, regardless of whether MSAA is enabled.
    pub(super) sample_bind_group: wgpu::BindGroup,
}

impl DepthAttachmentSet {
    pub(super) fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        render_texture: wgpu::Texture,
        render_view: wgpu::TextureView,
        resolve_texture: wgpu::Texture,
        resolve_view: wgpu::TextureView,
    ) -> Self {
        // Bind group is built here — the only place — ensuring it always uses the resolve view.
        let sample_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skins-preview-depth-sample"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&resolve_view),
            }],
        });
        Self {
            _render_texture: render_texture,
            render_view,
            _resolve_texture: resolve_texture,
            _resolve_view: resolve_view,
            sample_bind_group,
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
