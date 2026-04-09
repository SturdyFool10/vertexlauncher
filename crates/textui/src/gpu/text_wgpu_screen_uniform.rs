use super::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(super) struct TextWgpuScreenUniform {
    pub(super) screen_size_points: [f32; 2],
    pub(super) _padding: [f32; 2],
}
