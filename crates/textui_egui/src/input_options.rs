use egui::{Color32, Stroke, Vec2};
use textui::{EguiInputOptions, TextFundamentals};

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

impl InputOptions {
    pub(crate) fn to_core_input_options(&self) -> EguiInputOptions {
        EguiInputOptions {
            font_size: self.font_size,
            line_height: self.line_height,
            text_color: self.text_color,
            cursor_color: self.cursor_color,
            selection_color: self.selection_color,
            selected_text_color: self.selected_text_color,
            background_color: self.background_color,
            background_color_hovered: self.background_color_hovered,
            background_color_focused: self.background_color_focused,
            stroke: self.stroke,
            stroke_hovered: self.stroke_hovered,
            stroke_focused: self.stroke_focused,
            corner_radius: self.corner_radius,
            padding: self.padding,
            monospace: self.monospace,
            min_width: self.min_width,
            desired_width: self.desired_width,
            desired_rows: self.desired_rows,
            placeholder_text: self.placeholder_text.clone(),
            placeholder_color: self.placeholder_color,
            fundamentals: self.fundamentals.clone(),
        }
    }
}
