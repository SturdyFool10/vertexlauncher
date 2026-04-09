use super::*;

#[derive(Clone, Copy, Debug)]
pub struct TextAtlasQuad {
    pub atlas_page_index: usize,
    pub positions: [TextPoint; 4],
    pub uvs: [TextPoint; 4],
    pub tint: TextColor,
    pub is_color: bool,
}
