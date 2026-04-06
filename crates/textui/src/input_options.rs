use crate::TextFundamentals;
use egui::{Color32, Stroke, Vec2};

/// Styling/behavior options for single/multi-line text inputs.
#[derive(Clone, Debug)]
pub struct InputOptions {
    pub font_size: f32,
    pub line_height: f32,
    pub text_color: Color32,
    pub cursor_color: Color32,
    pub selection_color: Color32,
    pub selected_text_color: Color32,
    pub background_color: Color32,
    pub background_color_hovered: Option<Color32>,
    pub background_color_focused: Option<Color32>,
    pub stroke: Stroke,
    pub stroke_hovered: Option<Stroke>,
    pub stroke_focused: Option<Stroke>,
    pub corner_radius: u8,
    pub padding: Vec2,
    pub monospace: bool,
    pub min_width: f32,
    pub desired_width: Option<f32>,
    pub desired_rows: usize,
    pub placeholder_text: Option<String>,
    pub placeholder_color: Option<Color32>,
    pub fundamentals: TextFundamentals,
}

impl Default for InputOptions {
    fn default() -> Self {
        Self {
            font_size: 18.0,
            line_height: 24.0,
            text_color: Color32::WHITE,
            cursor_color: Color32::from_rgb(90, 170, 255),
            selection_color: Color32::from_rgba_premultiplied(90, 170, 255, 80),
            selected_text_color: Color32::WHITE,
            background_color: Color32::from_rgb(11, 13, 16),
            background_color_hovered: None,
            background_color_focused: None,
            stroke: Stroke::new(1.0, Color32::from_rgb(45, 50, 60)),
            stroke_hovered: None,
            stroke_focused: None,
            corner_radius: 6,
            padding: egui::vec2(8.0, 6.0),
            monospace: false,
            min_width: 64.0,
            desired_width: None,
            desired_rows: 5,
            placeholder_text: None,
            placeholder_color: None,
            fundamentals: TextFundamentals::default(),
        }
    }
}
