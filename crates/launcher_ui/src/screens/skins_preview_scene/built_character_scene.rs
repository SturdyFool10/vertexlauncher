use super::*;

pub(crate) struct BuiltCharacterScene {
    pub(crate) triangles: Vec<RenderTriangle>,
    pub(crate) cape_render_sample: Option<Arc<RgbaImage>>,
}
