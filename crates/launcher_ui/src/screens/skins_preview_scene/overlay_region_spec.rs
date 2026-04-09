use super::*;

#[derive(Clone, Copy)]
pub(super) struct OverlayRegionSpec {
    pub(super) face: OverlayVoxelFace,
    pub(super) tex_x: u32,
    pub(super) tex_y: u32,
    pub(super) width: u32,
    pub(super) height: u32,
}
