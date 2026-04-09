use super::*;

#[derive(Clone, Debug)]
pub struct TextGpuScene {
    pub atlas_pages: Vec<TextAtlasPageData>,
    pub quads: Vec<TextGpuQuad>,
    pub bounds_min: [f32; 2],
    pub bounds_max: [f32; 2],
    pub size_points: [f32; 2],
    pub fingerprint: u64,
}
