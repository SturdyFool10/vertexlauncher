use egui::{Color32, Stroke, Vec2};

use crate::LabelOptions;

#[derive(Clone, Debug)]
pub struct TooltipOptions {
    pub text: LabelOptions,
    pub background: Color32,
    pub stroke: Stroke,
    pub corner_radius: u8,
    pub padding: Vec2,
    pub offset: Vec2,
}

impl Default for TooltipOptions {
    fn default() -> Self {
        let mut text = LabelOptions::default();
        text.font_size = 14.0;
        text.line_height = 18.0;
        text.wrap = true;

        Self {
            text,
            background: Color32::from_rgba_premultiplied(14, 16, 20, 245),
            stroke: Stroke::new(1.0, Color32::from_rgb(42, 48, 58)),
            corner_radius: 6,
            padding: egui::vec2(8.0, 6.0),
            offset: egui::vec2(10.0, 6.0),
        }
    }
}
