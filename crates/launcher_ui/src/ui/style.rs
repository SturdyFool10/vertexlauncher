use egui::{Color32, Ui, Vec2};
use textui_egui::prelude::*;

pub const SPACE_XS: f32 = 4.0;
pub const SPACE_SM: f32 = 6.0;
pub const SPACE_MD: f32 = 8.0;
pub const SPACE_LG: f32 = 10.0;
pub const SPACE_XL: f32 = 12.0;

pub const CONTROL_HEIGHT: f32 = 30.0;
pub const CONTROL_HEIGHT_LG: f32 = 34.0;
pub const CORNER_RADIUS_SM: u8 = 8;
pub const CORNER_RADIUS_MD: u8 = 10;

pub fn page_heading(ui: &Ui) -> LabelOptions {
    heading(ui, 30.0, 34.0)
}

pub fn section_heading(ui: &Ui) -> LabelOptions {
    heading(ui, 20.0, 24.0)
}

pub fn heading(ui: &Ui, font_size: f32, line_height: f32) -> LabelOptions {
    heading_color(ui, font_size, line_height, ui.visuals().text_color())
}

pub fn heading_color(_ui: &Ui, font_size: f32, line_height: f32, color: Color32) -> LabelOptions {
    LabelOptions {
        font_size,
        line_height,
        weight: 700,
        color,
        wrap: false,
        ..LabelOptions::default()
    }
}

pub fn body(ui: &Ui) -> LabelOptions {
    LabelOptions {
        color: ui.visuals().text_color(),
        wrap: true,
        ..LabelOptions::default()
    }
}

pub fn muted(ui: &Ui) -> LabelOptions {
    LabelOptions {
        color: ui.visuals().weak_text_color(),
        wrap: true,
        ..LabelOptions::default()
    }
}

pub fn muted_single_line(ui: &Ui) -> LabelOptions {
    let mut style = muted(ui);
    style.wrap = false;
    style
}

pub fn neutral_button(ui: &Ui) -> ButtonOptions {
    ButtonOptions {
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..ButtonOptions::default()
    }
}

pub fn neutral_button_with_min_size(ui: &Ui, min_size: Vec2) -> ButtonOptions {
    ButtonOptions {
        min_size,
        ..neutral_button(ui)
    }
}
