use super::*;

use super::skins_preview_gpu_post_process::SkinPreviewPostProcessWgpuCallback;

/// Renders weighted scene samples through the WGPU post-process path to produce
/// motion blur.
///
/// Each scene weight should be finite and non-negative. `scene_msaa_samples` and
/// `present_msaa_samples` values below `1` are clamped by the downstream texture
/// allocation helpers.
///
/// This function does not panic.
pub(in super::super) fn render_weighted_motion_blur_scene_wgpu(
    ui: &Ui,
    rect: Rect,
    scenes: &[WeightedPreviewScene],
    skin_sample: Arc<RgbaImage>,
    cape_sample: Option<Arc<RgbaImage>>,
    target_format: wgpu::TextureFormat,
    scene_msaa_samples: u32,
    present_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
) {
    let callback = SkinPreviewPostProcessWgpuCallback::from_weighted_scenes(
        scenes,
        skin_sample,
        cape_sample,
        target_format,
        scene_msaa_samples,
        present_msaa_samples,
        preview_aa_mode,
        preview_texel_aa_mode,
    );
    let callback_shape = egui_wgpu::Callback::new_paint_callback(rect, callback);
    ui.painter().add(callback_shape);
}
