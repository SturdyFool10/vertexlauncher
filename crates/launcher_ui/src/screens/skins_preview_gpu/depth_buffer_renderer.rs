use super::*;

use super::skins_preview_gpu_post_process::SkinPreviewPostProcessWgpuCallback;

/// Renders a single preview scene with depth testing.
///
/// `preview_msaa_samples` values below `1` are clamped to `1`.
///
/// This function does not panic.
pub(in super::super) fn render_preview_scene_with_depth_buffer(
    ui: &Ui,
    rect: Rect,
    triangles: &[RenderTriangle],
    skin_sample: Arc<RgbaImage>,
    cape_sample: Option<Arc<RgbaImage>>,
    wgpu_target_format: wgpu::TextureFormat,
    preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
) {
    let callback = SkinPreviewPostProcessWgpuCallback::from_scene(
        triangles,
        skin_sample,
        cape_sample,
        wgpu_target_format,
        if preview_aa_mode == SkinPreviewAaMode::Msaa {
            preview_msaa_samples.max(1)
        } else {
            1
        },
        preview_msaa_samples.max(1),
        preview_aa_mode,
        preview_texel_aa_mode,
    );
    let callback_shape = egui_wgpu::Callback::new_paint_callback(rect, callback);
    ui.painter().add(callback_shape);
}
