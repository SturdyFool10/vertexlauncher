use super::*;

/// Builds normalized weighted preview scenes for motion blur accumulation.
///
/// `amount` is clamped to `0.0..=1.0`. `sample_count` values below `2` are promoted to
/// `2`. Returns an empty vector when the blur amount or angular span is too small to
/// produce a visible effect.
///
/// This function does not panic.
pub(in super::super) fn build_motion_blur_scene_samples(
    rect: Rect,
    cape_uv: FaceUvs,
    yaw: f32,
    yaw_velocity: f32,
    preview_pose: PreviewPose,
    shutter_frames: f32,
    sample_count: usize,
    variant: MinecraftSkinVariant,
    preview_3d_layers_enabled: bool,
    show_elytra: bool,
    expressions_enabled: bool,
    expression_layout: Option<DetectedExpressionsLayout>,
    skin_sample: Option<Arc<RgbaImage>>,
    cape_sample: Option<Arc<RgbaImage>>,
    default_elytra_sample: Option<Arc<RgbaImage>>,
    amount: f32,
) -> Vec<WeightedPreviewScene> {
    let amount = amount.clamp(0.0, 1.0);
    if amount <= 0.001 {
        return Vec::new();
    }

    let sample_count = sample_count.max(2);
    let shutter_seconds = motion_blur_shutter_seconds(shutter_frames);
    if shutter_seconds * yaw_velocity.abs() <= MOTION_BLUR_MIN_ANGULAR_SPAN {
        return Vec::new();
    }

    let center = (sample_count.saturating_sub(1)) as f32 * 0.5;
    let mut weights = Vec::with_capacity(sample_count);
    let mut total_weight = 0.0;

    for index in 0..sample_count {
        let distance = (index as f32 - center).abs();
        let normalized_distance = if center <= f32::EPSILON {
            0.0
        } else {
            distance / center
        };
        let falloff = egui::lerp(4.8..=1.35, amount);
        let edge_floor = egui::lerp(0.0..=0.08, amount * amount);
        let weight = (1.0 - normalized_distance * normalized_distance)
            .max(0.0)
            .powf(falloff)
            .max(edge_floor)
            .max(0.02);
        weights.push(weight);
        total_weight += weight;
    }

    let total_weight = total_weight.max(f32::EPSILON);
    let mut scenes = Vec::with_capacity(sample_count);
    for (index, raw_weight) in weights.into_iter().enumerate() {
        let sample_t = if sample_count <= 1 {
            0.5
        } else {
            index as f32 / (sample_count - 1) as f32
        };
        let time_offset = (sample_t - 0.5) * shutter_seconds;
        let sample_yaw = yaw + time_offset * yaw_velocity;
        let sample_pose = PreviewPose {
            time_seconds: preview_pose.time_seconds + time_offset,
            idle_cycle: ((preview_pose.time_seconds + time_offset) * 1.15).sin(),
            walk_cycle: ((preview_pose.time_seconds + time_offset) * 3.3).sin(),
            locomotion_blend: preview_pose.locomotion_blend,
        };
        let scene = build_character_scene(
            rect,
            cape_uv,
            sample_yaw,
            sample_pose,
            variant,
            preview_3d_layers_enabled,
            show_elytra,
            expressions_enabled,
            expression_layout,
            skin_sample.clone(),
            cape_sample.clone(),
            default_elytra_sample.clone(),
        );
        scenes.push(WeightedPreviewScene {
            weight: raw_weight / total_weight,
            triangles: scene.triangles,
        });
    }

    scenes
}

fn motion_blur_shutter_seconds(shutter_frames: f32) -> f32 {
    let frame = 1.0 / PREVIEW_TARGET_FPS;
    frame * shutter_frames.max(0.0)
}
