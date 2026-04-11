use super::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct TextWgpuInstance {
    pub(crate) pos_origin: [f32; 2],
    pub(crate) pos_u: [f32; 2],
    pub(crate) pos_v: [f32; 2],
    pub(crate) uv_origin: [f32; 2],
    pub(crate) uv_u: [f32; 2],
    pub(crate) uv_v: [f32; 2],
    pub(crate) color: [u8; 4],
    pub(crate) decode_mode: u32,
}

impl TextWgpuInstance {
    pub(crate) fn from_quad(quad: &PaintTextQuad) -> Self {
        let pos0 = quad.positions[0];
        let pos1 = quad.positions[1];
        let pos3 = quad.positions[3];
        let uv0 = quad.uvs[0];
        let uv1 = quad.uvs[1];
        let uv3 = quad.uvs[3];
        Self {
            pos_origin: [pos0.x, pos0.y],
            pos_u: [pos1.x - pos0.x, pos1.y - pos0.y],
            pos_v: [pos3.x - pos0.x, pos3.y - pos0.y],
            uv_origin: [uv0.x, uv0.y],
            uv_u: [uv1.x - uv0.x, uv1.y - uv0.y],
            uv_v: [uv3.x - uv0.x, uv3.y - uv0.y],
            color: quad.tint.to_array(),
            decode_mode: match quad.content_mode {
                GlyphContentMode::AlphaMask => 0,
                GlyphContentMode::Sdf => 1,
                GlyphContentMode::Msdf => 2,
            },
        }
    }
}
