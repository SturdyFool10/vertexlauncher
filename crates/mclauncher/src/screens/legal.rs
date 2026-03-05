use egui::{ScrollArea, Ui};

use crate::assets;

pub fn render(ui: &mut Ui) {
    ui.heading("Legal");
    ui.add_space(8.0);
    ui.label("Bundled icon license (compiled into the binary):");
    ui.add_space(6.0);

    ScrollArea::vertical().show(ui, |ui| {
        ui.monospace(assets::TABLER_LICENSE);
    });
}
