use super::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(super) struct TextWgpuScreenUniform {
    pub(super) screen_size_points: [f32; 2],
    /// 1.0 = outputting to HDR surface (linear passthrough), 0.0 = SDR surface (tonemap + sRGB encode).
    pub(super) output_is_hdr: f32,
    /// WGSL places the trailing `vec2<f32>` at offset 16, so the CPU struct must include
    /// an explicit 4-byte pad after `output_is_hdr` to keep the buffer 24 bytes wide.
    pub(super) _pad0: f32,
    pub(super) _pad1: [f32; 2],
}
