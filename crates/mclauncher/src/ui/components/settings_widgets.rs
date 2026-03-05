use std::hash::Hash;

use egui::{self, Align, Layout, Response, Sense, Ui};

use crate::assets;

#[derive(Clone, Copy, Debug)]
struct ControlMetrics {
    right_padding: f32,
    control_height: f32,
    switch_width: f32,
    dropdown_width: f32,
    icon_size: f32,
}

pub fn toggle_row(
    ui: &mut Ui,
    label: &str,
    info_tooltip: Option<&str>,
    value: &mut bool,
) -> Response {
    let metrics = control_metrics(ui);

    ui.horizontal(|ui| {
        let mut label_response = ui.add(egui::Label::new(label).sense(Sense::click()));
        if label_response.clicked() {
            *value = !*value;
            label_response.mark_changed();
        }

        if info_tooltip.is_some() {
            ui.add_space(6.0);
            info_hint(ui, info_tooltip);
        }

        let switch_response = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(metrics.right_padding);
                switch(ui, value, metrics)
            })
            .inner;

        switch_response.union(label_response)
    })
    .inner
}

#[allow(dead_code)]
pub fn dropdown_row(
    ui: &mut Ui,
    id_source: impl Hash,
    label: &str,
    info_tooltip: Option<&str>,
    selected_index: &mut usize,
    options: &[&str],
) -> Response {
    let metrics = control_metrics(ui);

    ui.horizontal(|ui| {
        let label_response = ui.label(label);

        if info_tooltip.is_some() {
            ui.add_space(6.0);
            info_hint(ui, info_tooltip);
        }

        let dropdown_response = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(metrics.right_padding);
                ui.push_id(id_source, |ui| {
                    dropdown(ui, selected_index, options, metrics)
                })
                .inner
            })
            .inner;

        dropdown_response.union(label_response)
    })
    .inner
}

pub fn info_hint(ui: &mut Ui, tooltip: Option<&str>) -> Response {
    let metrics = control_metrics(ui);
    let icon = themed_svg_image(
        "settings-info-circle",
        assets::INFO_CIRCLE_SVG,
        metrics.icon_size,
        ui.visuals().weak_text_color(),
    )
    .sense(Sense::hover())
    .fit_to_exact_size(egui::vec2(metrics.icon_size, metrics.icon_size));

    let response = ui.add(icon);
    if let Some(text) = tooltip {
        response.on_hover_text(text)
    } else {
        response
    }
}

fn switch(ui: &mut Ui, value: &mut bool, metrics: ControlMetrics) -> Response {
    let desired_size = egui::vec2(metrics.switch_width, metrics.control_height);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click());

    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }

    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, "")
    });

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *value);
        let off_bg = ui.visuals().widgets.inactive.bg_fill;
        let on_bg = ui.visuals().selection.bg_fill;
        let bg_fill: egui::Color32 =
            egui::lerp(egui::Rgba::from(off_bg)..=egui::Rgba::from(on_bg), how_on).into();
        let bg_stroke = ui.visuals().widgets.inactive.bg_stroke;
        let corner_radius = rect.height() / 2.0;
        ui.painter().rect(
            rect,
            corner_radius,
            bg_fill,
            bg_stroke,
            egui::StrokeKind::Inside,
        );

        let knob_margin = (metrics.control_height * 0.10).clamp(2.0, 4.0);
        let knob_radius = (rect.height() - (knob_margin * 2.0)) / 2.0;
        let knob_x = egui::lerp(
            (rect.left() + knob_margin + knob_radius)..=(rect.right() - knob_margin - knob_radius),
            how_on,
        );
        let knob_center = egui::pos2(knob_x, rect.center().y);
        let knob_fill = ui.visuals().widgets.noninteractive.fg_stroke.color;
        ui.painter().circle(
            knob_center,
            knob_radius,
            knob_fill,
            egui::Stroke::new(1.0, bg_stroke.color),
        );
    }

    response
}

#[allow(dead_code)]
fn dropdown(
    ui: &mut Ui,
    selected_index: &mut usize,
    options: &[&str],
    metrics: ControlMetrics,
) -> Response {
    let selected_text = options.get(*selected_index).copied().unwrap_or("Select...");
    let icon = themed_svg_image(
        "settings-dropdown-chevron",
        assets::CHEVRON_DOWN_SVG,
        metrics.icon_size,
        ui.visuals().text_color(),
    )
    .fit_to_exact_size(egui::vec2(metrics.icon_size, metrics.icon_size));

    let button = egui::Button::image_and_text(icon, selected_text)
        .min_size(egui::vec2(metrics.dropdown_width, metrics.control_height))
        .frame(true);

    let (mut response, popup) =
        egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
            let mut changed = false;
            ui.set_min_width(metrics.dropdown_width);

            for (index, option) in options.iter().enumerate() {
                if ui
                    .selectable_label(*selected_index == index, *option)
                    .clicked()
                {
                    *selected_index = index;
                    changed = true;
                    ui.close();
                }
            }

            changed
        });

    if let Some(inner) = popup {
        if inner.inner {
            response.mark_changed();
        }
    }

    response
}

fn control_metrics(ui: &Ui) -> ControlMetrics {
    let viewport_width = ui.ctx().input(|i| i.content_rect().width()).max(320.0);
    let text_height = ui.text_style_height(&egui::TextStyle::Body).max(14.0);
    let control_height = (viewport_width * 0.024).clamp(22.0, 34.0);

    ControlMetrics {
        right_padding: (viewport_width * 0.01).clamp(8.0, 20.0),
        control_height,
        switch_width: (control_height * 1.95).clamp(42.0, 72.0),
        dropdown_width: (viewport_width * 0.18).clamp(170.0, 320.0),
        icon_size: text_height.clamp(14.0, 20.0),
    }
}

fn themed_svg_image(
    icon_id: &str,
    svg_bytes: &[u8],
    icon_size: f32,
    color: egui::Color32,
) -> egui::Image<'static> {
    let themed_svg = apply_svg_color(svg_bytes, color);
    let uri = format!(
        "bytes://vertex-settings-icons/{icon_id}-{:02x}{:02x}{:02x}.svg",
        color.r(),
        color.g(),
        color.b()
    );
    egui::Image::from_bytes(uri, themed_svg).fit_to_exact_size(egui::vec2(icon_size, icon_size))
}

fn apply_svg_color(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    String::from_utf8_lossy(svg_bytes)
        .replace("currentColor", &color_hex)
        .into_bytes()
}
