use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextVector {
    pub x: f32,
    pub y: f32,
}

impl TextVector {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub const fn splat(value: f32) -> Self {
        Self::new(value, value)
    }
}

impl From<Vec2> for TextVector {
    fn from(value: Vec2) -> Self {
        Self::new(value.x, value.y)
    }
}

impl From<TextVector> for Vec2 {
    fn from(value: TextVector) -> Self {
        Vec2::new(value.x, value.y)
    }
}
