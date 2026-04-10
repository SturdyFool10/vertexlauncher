use super::super::*;

/// Builds motion-blur samples and submits the weighted GPU render path when sampling
/// produces at least one scene. Otherwise this function renders the non-blurred path.
///
/// `preview_motion_blur_amount` is forwarded to the scene sampler, which clamps it to
/// `0.0..=1.0`. This function does not panic.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_motion_blur_preview_path(
    ui: &Ui,
    rect: Rect,
    skin_sample: Arc<RgbaImage>,
    cape_render_sample: Option<Arc<RgbaImage>>,
    cape_uv: FaceUvs,
    yaw: f32,
    yaw_velocity: f32,
    preview_pose: PreviewPose,
    variant: MinecraftSkinVariant,
    preview_3d_layers_enabled: bool,
    show_elytra: bool,
    expressions_enabled: bool,
    expression_layout: Option<DetectedExpressionsLayout>,
    cape_sample: Option<Arc<RgbaImage>>,
    default_elytra_sample: Option<Arc<RgbaImage>>,
    preview_motion_blur_amount: f32,
    preview_motion_blur_shutter_frames: f32,
    preview_motion_blur_sample_count: usize,
    wgpu_target_format: wgpu::TextureFormat,
    preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
) -> bool {
    let motion_blur_samples = build_motion_blur_scene_samples(
        rect,
        cape_uv,
        yaw,
        yaw_velocity,
        preview_pose,
        preview_motion_blur_shutter_frames,
        preview_motion_blur_sample_count,
        variant,
        preview_3d_layers_enabled,
        show_elytra,
        expressions_enabled,
        expression_layout,
        Some(Arc::clone(&skin_sample)),
        cape_sample,
        default_elytra_sample,
        preview_motion_blur_amount,
    );

    if motion_blur_samples.is_empty() {
        return false;
    }

    render_weighted_motion_blur_scene_wgpu(
        ui,
        rect,
        &motion_blur_samples,
        skin_sample,
        cape_render_sample,
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
    true
}
