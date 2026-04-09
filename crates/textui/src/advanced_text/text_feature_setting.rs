#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextFeatureSetting {
    pub tag: [u8; 4],
    pub value: u16,
}

impl TextFeatureSetting {
    pub const fn new(tag: [u8; 4], value: u16) -> Self {
        Self { tag, value }
    }
}
