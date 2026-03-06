use config::ConfigFormat;
use eframe::egui;
use textui::{ButtonOptions, LabelOptions, TextUi};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModalAction {
    None,
    Create(ConfigFormat),
    Cancel,
}

pub fn render(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    selected_format: &mut ConfigFormat,
    config_creation_error: Option<&str>,
) -> ModalAction {
    let mut action = ModalAction::None;

    egui::Window::new("Config format")
        .id(egui::Id::new("config_format_modal_window"))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .title_bar(false)
        .frame(
            egui::Frame::new()
                .fill(ctx.style().visuals.window_fill)
                .stroke(egui::Stroke::new(
                    1.0,
                    ctx.style().visuals.widgets.hovered.bg_stroke.color,
                ))
                .corner_radius(egui::CornerRadius::same(14))
                .inner_margin(egui::Margin::same(14)),
        )
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            let text_color = ui.visuals().text_color();
            let heading = LabelOptions {
                font_size: 28.0,
                line_height: 32.0,
                weight: 700,
                color: text_color,
                wrap: false,
                ..LabelOptions::default()
            };
            let body = LabelOptions {
                color: text_color,
                ..LabelOptions::default()
            };
            let mut muted = body.clone();
            muted.color = ui.visuals().weak_text_color();
            let _ = text_ui.label(ui, "config_modal_heading", "Config format", &heading);
            let _ = text_ui.label(
                ui,
                "config_modal_subheading",
                "Pick how Vertex stores your launcher settings.",
                &muted,
            );
            ui.add_space(4.0);

            let radio_style = ButtonOptions {
                min_size: egui::vec2(ui.available_width(), 38.0),
                corner_radius: 8,
                padding: egui::vec2(10.0, 6.0),
                text_color: text_color,
                fill: ui.visuals().widgets.inactive.bg_fill,
                fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                fill_active: ui.visuals().widgets.active.bg_fill,
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().widgets.inactive.bg_stroke,
                ..ButtonOptions::default()
            };
            egui::Frame::new()
                .fill(ui.visuals().widgets.noninteractive.bg_fill)
                .stroke(egui::Stroke::new(
                    1.0,
                    ui.visuals().widgets.noninteractive.bg_stroke.color,
                ))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    if text_ui
                        .selectable_button(
                            ui,
                            "config_modal_fmt_toml",
                            "TOML (recommended for readability)",
                            *selected_format == ConfigFormat::Toml,
                            &radio_style,
                        )
                        .clicked()
                    {
                        *selected_format = ConfigFormat::Toml;
                    }
                    if text_ui
                        .selectable_button(
                            ui,
                            "config_modal_fmt_json",
                            "JSON (best for tooling and scripts)",
                            *selected_format == ConfigFormat::Json,
                            &radio_style,
                        )
                        .clicked()
                    {
                        *selected_format = ConfigFormat::Json;
                    }
                });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            let _ = text_ui.label(
                ui,
                "config_modal_desc",
                "Choose a format to create your initial launcher config.",
                &body,
            );

            if let Some(err) = config_creation_error {
                ui.add_space(6.0);
                let mut err_style = body.clone();
                err_style.color = ui.visuals().error_fg_color;
                let _ = text_ui.label(ui, "config_modal_err", err, &err_style);
            }

            ui.add_space(12.0);
            let mut create_clicked = false;
            let mut cancel_clicked = false;

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let create_style = ButtonOptions {
                    min_size: egui::vec2(136.0, 32.0),
                    text_color: ui.visuals().widgets.active.fg_stroke.color,
                    fill: ui.visuals().selection.bg_fill,
                    fill_hovered: ui.visuals().selection.bg_fill.gamma_multiply(1.1),
                    fill_active: ui.visuals().selection.bg_fill.gamma_multiply(0.9),
                    fill_selected: ui.visuals().selection.bg_fill,
                    stroke: ui.visuals().selection.stroke,
                    ..ButtonOptions::default()
                };
                let cancel_style = ButtonOptions {
                    min_size: egui::vec2(100.0, 32.0),
                    text_color: text_color,
                    fill: ui.visuals().widgets.inactive.bg_fill,
                    fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                    fill_active: ui.visuals().widgets.active.bg_fill,
                    fill_selected: ui.visuals().selection.bg_fill,
                    stroke: ui.visuals().widgets.inactive.bg_stroke,
                    ..ButtonOptions::default()
                };
                create_clicked = text_ui
                    .button(ui, "config_modal_create", "Create config", &create_style)
                    .clicked();
                cancel_clicked = text_ui
                    .button(ui, "config_modal_cancel", "Cancel", &cancel_style)
                    .clicked();
            });

            if cancel_clicked {
                action = ModalAction::Cancel;
            } else if create_clicked {
                action = ModalAction::Create(*selected_format);
            }
        });

    action
}
