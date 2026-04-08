use egui::{Color32, Vec2};
use textui::{DEFAULT_ELLIPSIS, TextFundamentals, TextLabelOptions};

#[derive(Clone, Debug)]
pub struct LabelOptions {
    pub font_size: f32,
    pub line_height: f32,
    pub color: Color32,
    pub wrap: bool,
    pub monospace: bool,
    pub weight: u16,
    pub italic: bool,
    pub padding: Vec2,
    pub fundamentals: TextFundamentals,
    /// The string appended when text is truncated.  Defaults to the Unicode
    /// ellipsis character (U+2026).
    pub ellipsis: String,
}

impl LabelOptions {
    pub(crate) fn to_text_label_options(&self) -> TextLabelOptions {
        TextLabelOptions {
            font_size: self.font_size,
            line_height: self.line_height,
            color: self.color.into(),
            wrap: self.wrap,
            monospace: self.monospace,
            weight: self.weight,
            italic: self.italic,
            padding: self.padding.into(),
            fundamentals: self.fundamentals.clone(),
            ellipsis: self.ellipsis.clone(),
        }
    }
}

impl Default for LabelOptions {
    fn default() -> Self {
        Self {
            font_size: 18.0,
            line_height: 27.0,
            color: Color32::WHITE,
            wrap: true,
            monospace: false,
            weight: 400,
            italic: false,
            padding: egui::vec2(0.0, 0.0),
            fundamentals: TextFundamentals::default(),
            ellipsis: DEFAULT_ELLIPSIS.to_owned(),
        }
    }
}
