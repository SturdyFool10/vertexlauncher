use super::*;

#[derive(Clone, Copy)]
pub(super) struct OverlayPartSpec {
    pub(super) size: Vec3,
    pub(super) pivot_top_center: Vec3,
    pub(super) rotate_x: f32,
    pub(super) rotate_z: f32,
}
