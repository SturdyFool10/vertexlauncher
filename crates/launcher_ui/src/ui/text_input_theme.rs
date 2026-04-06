use egui::{Color32, CornerRadius, Ui};
use textui_egui::prelude::*;

pub fn themed_text_input_options(ui: &Ui, monospace: bool) -> InputOptions {
    let visuals = ui.visuals();
    let selection = visuals.selection.bg_fill;
    let inactive = visuals.widgets.inactive;
    let hovered = visuals.widgets.hovered;
    let focused = visuals.widgets.active;

    InputOptions {
        text_color: visuals.text_color(),
        cursor_color: visuals.text_cursor.stroke.color,
        selection_color: Color32::from_rgba_premultiplied(
            selection.r(),
            selection.g(),
            selection.b(),
            110,
        ),
        selected_text_color: focused.fg_stroke.color,
        background_color: inactive.bg_fill,
        background_color_hovered: Some(hovered.bg_fill),
        background_color_focused: Some(focused.bg_fill),
        stroke: inactive.bg_stroke,
        stroke_hovered: Some(hovered.bg_stroke),
        stroke_focused: Some(focused.bg_stroke),
        corner_radius: max_corner_radius(inactive.corner_radius),
        monospace,
        ..InputOptions::default()
    }
}

fn max_corner_radius(corner_radius: CornerRadius) -> u8 {
    corner_radius
        .nw
        .max(corner_radius.ne)
        .max(corner_radius.sw)
        .max(corner_radius.se)
}
