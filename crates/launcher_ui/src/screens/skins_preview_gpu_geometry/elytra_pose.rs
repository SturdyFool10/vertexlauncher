use super::elytra_wing_pose::ElytraWingPose;

#[derive(Clone, Copy)]
pub(super) struct ElytraPose {
    pub(super) left: ElytraWingPose,
    pub(super) right: ElytraWingPose,
}
