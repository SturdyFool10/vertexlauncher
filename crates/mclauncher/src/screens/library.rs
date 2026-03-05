use egui::Ui;

pub fn render(ui: &mut Ui, selected_profile_id: Option<&str>) {
    ui.heading("Library");
    ui.add_space(8.0);
    ui.label("Manage installed content and versions here.");

    if let Some(profile_id) = selected_profile_id {
        ui.add_space(8.0);
        ui.label(format!("Scoped to profile: {profile_id}"));
    }
}
