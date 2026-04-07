use std::hash::Hash;

use egui::{Ui, vec2};
use textui::TextUi;
use textui_egui::TextUiEguiExt;

use crate::tab_button;

pub fn fill_tab_row<T>(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    id_source: impl Hash,
    active_tab: &mut T,
    tabs: &[(T, &str)],
    height: f32,
    spacing: f32,
) where
    T: Copy + PartialEq,
{
    if tabs.is_empty() {
        return;
    }
    let width =
        ((ui.available_width() - spacing * (tabs.len() as f32 - 1.0)) / tabs.len() as f32).max(0.0);
    ui.push_id(id_source, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = spacing;
            for &(tab, label) in tabs {
                let selected = *active_tab == tab;
                if text_ui
                    .selectable_button(
                        ui,
                        ("fill_tab_row", label),
                        label,
                        selected,
                        &tab_button(ui, selected, vec2(width, height)),
                    )
                    .clicked()
                {
                    *active_tab = tab;
                }
            }
        });
    });
}
