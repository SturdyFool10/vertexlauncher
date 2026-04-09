#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextPathOptions {
    pub start_offset_points: f32,
    pub normal_offset_points: f32,
    pub rotate_glyphs: bool,
}

impl Default for TextPathOptions {
    fn default() -> Self {
        Self {
            start_offset_points: 0.0,
            normal_offset_points: 0.0,
            rotate_glyphs: true,
        }
    }
}
