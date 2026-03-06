use egui::Ui;
use textui::{LabelOptions, TextUi};

pub fn render(ui: &mut Ui, text_ui: &mut TextUi, selected_instance_id: Option<&str>) {
    let text_color = ui.visuals().text_color();
    let heading = LabelOptions {
        font_size: 30.0,
        line_height: 34.0,
        weight: 700,
        color: text_color,
        wrap: false,
        ..LabelOptions::default()
    };
    let body = LabelOptions {
        color: text_color,
        ..LabelOptions::default()
    };

    let _ = text_ui.label(ui, "skins_heading", "Skins", &heading);
    ui.add_space(8.0);
    let _ = text_ui.label(ui, "skins_desc", "Skin management UI goes here.", &body);

    if let Some(instance_id) = selected_instance_id {
        ui.add_space(8.0);
        let _ = text_ui.label(
            ui,
            "skins_instance_selected",
            &format!("Selected instance: {instance_id}"),
            &body,
        );
    }
}
