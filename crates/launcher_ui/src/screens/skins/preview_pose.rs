#[derive(Clone, Copy)]
pub(super) struct PreviewPose {
    pub(super) time_seconds: f32,
    pub(super) idle_cycle: f32,
    pub(super) walk_cycle: f32,
    pub(super) locomotion_blend: f32,
}
