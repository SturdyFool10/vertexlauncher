use std::hash::Hash;

use egui::{Color32, CornerRadius, Response, Sense, Ui, Vec2};
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
    text_ui: &mut TextUi,
    id_source: impl Hash,
    label: &str,
    selected: bool,
    min_size: Vec2,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(min_size, Sense::click());
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    let stroke = if selected {
        ui.visuals().selection.stroke
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_stroke
    } else {
        ui.visuals().widgets.inactive.bg_stroke
    };
    let text_color = if selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    };

    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(8),
        stroke,
        egui::StrokeKind::Inside,
    );

    let text_rect = rect.shrink2(egui::vec2(10.0, 4.0));
    let label_options = LabelOptions {
        color: text_color,
        wrap: true,
        ..LabelOptions::default()
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
        ui.set_clip_rect(text_rect.intersect(ui.clip_rect()));
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let _ = text_ui.label(ui, id_source, label, &label_options);
        });
    });

    response
}

fn max_corner_radius(corner_radius: CornerRadius) -> u8 {
    corner_radius
        .nw
        .max(corner_radius.ne)
        .max(corner_radius.sw)
        .max(corner_radius.se)
}
