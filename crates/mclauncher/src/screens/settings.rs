use egui::Ui;

pub fn render(ui: &mut Ui) {
    ui.heading("Settings");
    ui.add_space(8.0);
    ui.label("Launcher settings and preferences.");
}
