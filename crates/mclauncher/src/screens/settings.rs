use config::Config;
use egui::Ui;

use crate::ui::components::settings_widgets;

pub fn render(ui: &mut Ui, config: &mut Config) {
    ui.heading("Settings");
    ui.add_space(8.0);
    //ui.label("Launcher settings and preferences.");
    //ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);

    config.for_each_toggle_mut(|setting, value| {
        ui.push_id(setting.id, |ui| {
            settings_widgets::toggle_row(ui, setting.label, setting.info_tooltip, value);
        });
        ui.add_space(8.0);
    });
}
