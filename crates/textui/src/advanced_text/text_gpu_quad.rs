#[derive(Clone, Debug)]
pub struct TextGpuQuad {
    pub atlas_page_index: usize,
    pub positions: [[f32; 2]; 4],
    pub uvs: [[f32; 2]; 4],
    pub tint_rgba: [u8; 4],
}
