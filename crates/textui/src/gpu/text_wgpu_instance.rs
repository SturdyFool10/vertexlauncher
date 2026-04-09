use super::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct TextWgpuInstance {
    pub(crate) pos0: [f32; 2],
    pub(crate) pos1: [f32; 2],
    pub(crate) pos2: [f32; 2],
    pub(crate) pos3: [f32; 2],
    pub(crate) uv0: [f32; 2],
    pub(crate) uv1: [f32; 2],
    pub(crate) uv2: [f32; 2],
    pub(crate) uv3: [f32; 2],
    pub(crate) color: [f32; 4],
    pub(crate) decode_mode: f32,
    pub(crate) field_range_px: f32,
    pub(crate) _padding: [f32; 2],
}

impl TextWgpuInstance {
    pub(crate) fn from_quad(quad: &PaintTextQuad) -> Self {
        Self {
            pos0: [quad.positions[0].x, quad.positions[0].y],
            pos1: [quad.positions[1].x, quad.positions[1].y],
            pos2: [quad.positions[2].x, quad.positions[2].y],
            pos3: [quad.positions[3].x, quad.positions[3].y],
            uv0: [quad.uvs[0].x, quad.uvs[0].y],
            uv1: [quad.uvs[1].x, quad.uvs[1].y],
            uv2: [quad.uvs[2].x, quad.uvs[2].y],
            uv3: [quad.uvs[3].x, quad.uvs[3].y],
            color: quad.tint.to_normalized_gamma_f32(),
            decode_mode: match quad.content_mode {
                GlyphContentMode::AlphaMask => 0.0,
                GlyphContentMode::Sdf => 1.0,
                GlyphContentMode::Msdf => 2.0,
            },
            field_range_px: quad.field_range_px,
            _padding: [0.0, 0.0],
        }
    }
}
