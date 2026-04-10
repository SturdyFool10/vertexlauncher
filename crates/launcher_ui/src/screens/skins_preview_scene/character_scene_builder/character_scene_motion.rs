use super::*;

pub(super) struct CharacterSceneMotion {
    pub(super) arm_width: f32,
    pub(super) locomotion_blend: f32,
    pub(super) bob: f32,
    pub(super) leg_swing: f32,
    pub(super) arm_swing: f32,
    pub(super) torso_idle_tilt: f32,
    pub(super) head_idle_tilt: f32,
    pub(super) cape_walk_phase: f32,
}

impl CharacterSceneMotion {
    pub(super) fn new(preview_pose: PreviewPose, variant: MinecraftSkinVariant) -> Self {
        let arm_width = if variant == MinecraftSkinVariant::Slim {
            3.0
        } else {
            4.0
        };
        let idle_sway = preview_pose.idle_cycle;
        let walk_phase = preview_pose.walk_cycle;
        let locomotion_blend = preview_pose.locomotion_blend;
        let stride_phase = walk_phase * locomotion_blend;
        let bob_idle = 0.08 + (idle_sway * 0.5 + 0.5) * 0.12;
        let bob_walk = stride_phase.abs() * 0.58
            + (preview_pose.time_seconds * 6.6).cos().abs() * 0.04 * locomotion_blend;
        let bob = egui::lerp(bob_idle..=bob_walk, locomotion_blend);
        let leg_swing = stride_phase * 0.72;
        let arm_idle = (preview_pose.time_seconds * 1.35).sin() * 0.055;
        let arm_swing = (-stride_phase * 0.82) + arm_idle * (1.0 - locomotion_blend * 0.45);
        let torso_idle_tilt =
            idle_sway * 0.035 * (1.0 - locomotion_blend * 0.6) + stride_phase * 0.05;
        let head_idle_tilt =
            idle_sway * 0.055 * (1.0 - locomotion_blend * 0.55) - stride_phase * 0.035;

        Self {
            arm_width,
            locomotion_blend,
            bob,
            leg_swing,
            arm_swing,
            torso_idle_tilt,
            head_idle_tilt,
            cape_walk_phase: stride_phase,
        }
    }
}
