use super::*;

#[derive(Clone, Debug)]
pub struct TextPathLayout {
    pub glyphs: Vec<TextPathGlyph>,
    pub bounds: TextRect,
    pub total_advance_points: f32,
    pub path_length_points: f32,
}
