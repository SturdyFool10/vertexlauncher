use super::*;

pub(crate) struct RenderTriangle {
    pub(crate) texture: TriangleTexture,
    pub(crate) pos: [Pos2; 3],
    pub(crate) uv: [Pos2; 3],
    pub(crate) depth: [f32; 3],
    pub(crate) color: Color32,
}
