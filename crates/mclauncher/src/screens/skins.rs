use egui::Ui;

pub fn render(ui: &mut Ui, selected_profile_id: Option<&str>) {
    ui.heading("Skins");
    ui.add_space(8.0);
    ui.label("Skin management UI goes here.");

    if let Some(profile_id) = selected_profile_id {
        ui.add_space(8.0);
        ui.label(format!("Selected profile: {profile_id}"));
    }
}
