use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRect {
    pub min: TextPoint,
    pub max: TextPoint,
}

impl TextRect {
    pub const NOTHING: Self = Self::from_min_max(
        TextPoint::new(f32::INFINITY, f32::INFINITY),
        TextPoint::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
    );

    pub const fn from_min_max(min: TextPoint, max: TextPoint) -> Self {
        Self { min, max }
    }

    pub fn from_min_size(min: TextPoint, size: TextVector) -> Self {
        Self::from_min_max(min, TextPoint::new(min.x + size.x, min.y + size.y))
    }

    pub fn from_center_size(center: TextPoint, size: TextVector) -> Self {
        let half = TextVector::new(size.x * 0.5, size.y * 0.5);
        Self::from_min_max(
            TextPoint::new(center.x - half.x, center.y - half.y),
            TextPoint::new(center.x + half.x, center.y + half.y),
        )
    }

    pub fn width(self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn union(self, other: Self) -> Self {
        if self == Self::NOTHING {
            return other;
        }
        if other == Self::NOTHING {
            return self;
        }
        Self::from_min_max(
            TextPoint::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            TextPoint::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        )
    }
}

impl Default for TextRect {
    fn default() -> Self {
        Self::NOTHING
    }
}

impl From<Rect> for TextRect {
    fn from(value: Rect) -> Self {
        Self::from_min_max(value.min.into(), value.max.into())
    }
}

impl From<TextRect> for Rect {
    fn from(value: TextRect) -> Self {
        Rect::from_min_max(value.min.into(), value.max.into())
    }
}
