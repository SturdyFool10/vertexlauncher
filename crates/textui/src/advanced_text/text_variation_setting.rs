#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextVariationSetting {
    pub tag: [u8; 4],
    value_bits: u32,
}

impl TextVariationSetting {
    pub const fn from_bits(tag: [u8; 4], value_bits: u32) -> Self {
        Self { tag, value_bits }
    }

    pub fn new(tag: [u8; 4], value: f32) -> Self {
        Self::from_bits(tag, value.to_bits())
    }

    pub const fn value_bits(self) -> u32 {
        self.value_bits
    }

    pub fn value(self) -> f32 {
        f32::from_bits(self.value_bits)
    }
}
