use super::*;

pub(crate) struct WeightedPreviewScene {
    pub(crate) weight: f32,
    pub(crate) triangles: Vec<RenderTriangle>,
}
