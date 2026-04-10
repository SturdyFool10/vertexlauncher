use super::*;

#[path = "preview_character_renderer/motion_blur_render_path.rs"]
mod motion_blur_render_path;
#[path = "preview_character_renderer/render_path.rs"]
mod render_path;
#[path = "preview_character_renderer/single_scene_render_path.rs"]
mod single_scene_render_path;

use self::motion_blur_render_path::render_motion_blur_preview_path;
use self::render_path::PreviewRenderPath;
use self::single_scene_render_path::render_single_scene_preview_path;

/// Draws the current skin preview, selecting either the motion-blur accumulation path
/// or the single-scene depth-buffer path.
///
/// Motion blur is only rendered when a skin sample is available.
/// `preview_motion_blur_amount` is consumed by the scene-sampling stage, which clamps it
/// to `0.0..=1.0`.
///
/// This function does not panic.
pub(in super::super) fn render_preview_character(
    ui: &Ui,
    rect: Rect,
    skin_sample: Option<Arc<RgbaImage>>,
    cape_sample: Option<Arc<RgbaImage>>,
    default_elytra_sample: Option<Arc<RgbaImage>>,
    cape_uv: FaceUvs,
    yaw: f32,
    yaw_velocity: f32,
    preview_pose: PreviewPose,
    variant: MinecraftSkinVariant,
    show_elytra: bool,
    expressions_enabled: bool,
    expression_layout: Option<DetectedExpressionsLayout>,
    wgpu_target_format: wgpu::TextureFormat,
    preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
    preview_motion_blur_enabled: bool,
    preview_motion_blur_amount: f32,
    preview_motion_blur_shutter_frames: f32,
    preview_motion_blur_sample_count: usize,
    preview_3d_layers_enabled: bool,
) {
    let scene = build_character_scene(
        rect,
        cape_uv,
        yaw,
        preview_pose,
        variant,
        preview_3d_layers_enabled,
        show_elytra,
        expressions_enabled,
        expression_layout,
        skin_sample.clone(),
        cape_sample.clone(),
        default_elytra_sample.clone(),
    );

    match select_preview_render_path(preview_motion_blur_enabled, skin_sample.as_ref()) {
        PreviewRenderPath::MotionBlur { skin_sample } => {
            if render_motion_blur_preview_path(
                ui,
                rect,
                skin_sample,
                scene.cape_render_sample.clone(),
                cape_uv,
                yaw,
                yaw_velocity,
                preview_pose,
                variant,
                preview_3d_layers_enabled,
                show_elytra,
                expressions_enabled,
                expression_layout,
                cape_sample,
                default_elytra_sample,
                preview_motion_blur_amount,
                preview_motion_blur_shutter_frames,
                preview_motion_blur_sample_count,
                wgpu_target_format,
                preview_msaa_samples,
                preview_aa_mode,
                preview_texel_aa_mode,
            ) {
                return;
            }
        }
        PreviewRenderPath::SingleScene => {}
    }

    render_single_scene_preview_path(
        ui,
        rect,
        &scene.triangles,
        skin_sample,
        scene.cape_render_sample,
        wgpu_target_format,
        preview_msaa_samples,
        preview_aa_mode,
        preview_texel_aa_mode,
    );
}

fn select_preview_render_path(
    preview_motion_blur_enabled: bool,
    skin_sample: Option<&Arc<RgbaImage>>,
) -> PreviewRenderPath {
    if preview_motion_blur_enabled {
        if let Some(skin_sample) = skin_sample.cloned() {
            return PreviewRenderPath::MotionBlur { skin_sample };
        }
    }
    PreviewRenderPath::SingleScene
}
