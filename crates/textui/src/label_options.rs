use crate::{DEFAULT_ELLIPSIS, TextFundamentals};
use egui::{Color32, Vec2};

/// Styling options for plain/rich labels.
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
