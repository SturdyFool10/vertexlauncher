use super::*;

#[derive(Clone, Debug)]
pub struct TextPathGlyph {
    pub anchor: TextPoint,
    pub tangent: TextVector,
    pub normal: TextVector,
    pub rotation_radians: f32,
    pub local_offset: TextVector,
    pub advance_points: f32,
    pub color: TextColor,
}
