use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TextColor {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl TextColor {
    pub const TRANSPARENT: Self = Self::from_rgba8(0, 0, 0, 0);
    pub const WHITE: Self = Self::from_rgba8(255, 255, 255, 255);

    pub const fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn r(self) -> u8 {
        self.r
    }

    pub const fn g(self) -> u8 {
        self.g
    }

    pub const fn b(self) -> u8 {
        self.b
    }

    pub const fn a(self) -> u8 {
        self.a
    }

    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn to_normalized_gamma_f32(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }
}

impl From<Color32> for TextColor {
    fn from(value: Color32) -> Self {
        Self::from_rgba8(value.r(), value.g(), value.b(), value.a())
    }
}

impl From<TextColor> for Color32 {
    fn from(value: TextColor) -> Self {
        Color32::from_rgba_premultiplied(value.r, value.g, value.b, value.a)
    }
}
