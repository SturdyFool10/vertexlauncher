use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextPoint {
    pub x: f32,
    pub y: f32,
}

impl TextPoint {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl From<Pos2> for TextPoint {
    fn from(value: Pos2) -> Self {
        Self::new(value.x, value.y)
    }
}

impl From<TextPoint> for Pos2 {
    fn from(value: TextPoint) -> Self {
        Pos2::new(value.x, value.y)
    }
}
