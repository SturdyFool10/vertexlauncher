use egui::{Color32, Stroke, Vec2};
use textui::TextFundamentals;

#[derive(Clone, Debug)]
pub struct CodeBlockOptions {
    pub font_size: f32,
    pub line_height: f32,
    pub text_color: Color32,
    pub background_color: Color32,
    pub stroke: Stroke,
    pub wrap: bool,
    pub language: Option<String>,
    pub padding: Vec2,
    pub corner_radius: u8,
    pub fundamentals: TextFundamentals,
}

impl Default for CodeBlockOptions {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            line_height: 22.0,
            text_color: Color32::from_rgb(230, 230, 230),
            background_color: Color32::from_rgb(16, 18, 22),
            stroke: Stroke::new(1.0, Color32::from_rgb(36, 40, 48)),
            wrap: true,
            language: None,
            padding: egui::vec2(10.0, 10.0),
            corner_radius: 8,
            fundamentals: TextFundamentals::default(),
        }
    }
}
