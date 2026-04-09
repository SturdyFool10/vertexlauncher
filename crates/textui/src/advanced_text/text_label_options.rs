use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct TextLabelOptions {
    pub font_size: f32,
    pub line_height: f32,
    pub color: TextColor,
    pub wrap: bool,
    pub monospace: bool,
    pub weight: u16,
    pub italic: bool,
    pub padding: TextVector,
    pub fundamentals: TextFundamentals,
    pub ellipsis: String,
}

impl Default for TextLabelOptions {
    fn default() -> Self {
        Self {
            font_size: 18.0,
            line_height: 27.0,
            color: TextColor::WHITE,
            wrap: true,
            monospace: false,
            weight: 400,
            italic: false,
            padding: TextVector::ZERO,
            fundamentals: TextFundamentals::default(),
            ellipsis: DEFAULT_ELLIPSIS.to_owned(),
        }
    }
}
