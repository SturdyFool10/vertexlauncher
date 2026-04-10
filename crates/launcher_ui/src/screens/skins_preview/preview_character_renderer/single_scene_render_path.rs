use super::super::*;

/// Submits one already-built preview scene through the GPU depth-buffer render path.
///
/// `preview_msaa_samples` values below `1` are clamped by the downstream render path.
/// This function does not panic.
pub(super) fn render_single_scene_preview_path(
    ui: &Ui,
    rect: Rect,
    triangles: &[RenderTriangle],
    skin_sample: Option<Arc<RgbaImage>>,
    cape_sample: Option<Arc<RgbaImage>>,
    wgpu_target_format: wgpu::TextureFormat,
    preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
) {
    render_preview_scene_with_depth_buffer(
        ui,
        rect,
        triangles,
        skin_sample.expect("single-scene GPU preview requires a decoded skin sample"),
        cape_sample,
        wgpu_target_format,
        preview_msaa_samples,
        preview_aa_mode,
        preview_texel_aa_mode,
    );
}
