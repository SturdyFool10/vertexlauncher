use super::*;

#[derive(Clone, Debug)]
pub struct RichTextStyle {
    pub color: TextColor,
    pub monospace: bool,
    pub italic: bool,
    pub weight: u16,
}

impl Default for RichTextStyle {
    fn default() -> Self {
        Self {
            color: TextColor::WHITE,
            monospace: false,
            italic: false,
            weight: 400,
        }
    }
}
