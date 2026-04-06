use std::hash::Hash;

use egui::{Color32, CornerRadius, Response, Ui, Vec2};
use textui::TextUi;
use textui_egui::prelude::*;

pub fn themed_text_input(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    id_source: impl Hash,
    text: &mut String,
    mut options: InputOptions,
) -> Response {
    let visuals = ui.visuals();
    let selection = visuals.selection.bg_fill;
    let inactive = visuals.widgets.inactive;
    let hovered = visuals.widgets.hovered;
    let focused = visuals.widgets.active;

    options.text_color = visuals.text_color();
    options.cursor_color = visuals.text_cursor.stroke.color;
    options.selection_color =
        Color32::from_rgba_premultiplied(selection.r(), selection.g(), selection.b(), 110);
    options.selected_text_color = focused.fg_stroke.color;
    options.background_color = inactive.bg_fill;
    options.background_color_hovered = Some(hovered.bg_fill);
    options.background_color_focused = Some(focused.bg_fill);
    options.stroke = inactive.bg_stroke;
    options.stroke_hovered = Some(hovered.bg_stroke);
    options.stroke_focused = Some(focused.bg_stroke);
    options.corner_radius = max_corner_radius(inactive.corner_radius);

    text_ui.singleline_input(ui, id_source, text, &options)
}

pub fn selectable_row_button(
    ui: &mut Ui,
    label: impl Into<egui::WidgetText>,
    selected: bool,
    min_size: Vec2,
) -> Response {
    ui.add_sized(
        min_size,
        egui::Button::new(label)
            .selected(selected)
            .fill(if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.inactive.bg_fill
            })
            .stroke(if selected {
                ui.visuals().selection.stroke
            } else {
                ui.visuals().widgets.inactive.bg_stroke
            })
            .corner_radius(CornerRadius::same(8)),
    )
}

fn max_corner_radius(corner_radius: CornerRadius) -> u8 {
    corner_radius
        .nw
        .max(corner_radius.ne)
        .max(corner_radius.sw)
        .max(corner_radius.se)
}
