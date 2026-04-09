use super::*;

#[derive(Clone, Copy)]
pub(crate) struct CuboidSpec {
    pub(crate) size: Vec3,
    pub(crate) pivot_top_center: Vec3,
    pub(crate) rotate_x: f32,
    pub(crate) rotate_z: f32,
    pub(crate) uv: FaceUvs,
    pub(crate) cull_backfaces: bool,
}
