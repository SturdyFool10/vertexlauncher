use super::super::*;

pub(super) fn render_save_section(ui: &mut Ui, text_ui: &mut TextUi, state: &mut SkinManagerState) {
    let viewport_width = ui.clip_rect().width().max(1.0);
    let button_style =
        style::neutral_button_with_min_size(ui, egui::vec2(160.0, style::CONTROL_HEIGHT));

    ui.add_space(style::SPACE_MD);
    let mut save_style = button_style.clone();
    let save_width = ui.available_width().min(viewport_width).max(1.0);
    save_style.min_size = egui::vec2(save_width, style::CONTROL_HEIGHT_LG);
    save_style.fill = ui.visuals().selection.bg_fill;
    save_style.fill_hovered = ui.visuals().selection.bg_fill.gamma_multiply(1.15);
    save_style.fill_active = ui.visuals().selection.bg_fill.gamma_multiply(0.92);
    save_style.text_color = ui.visuals().strong_text_color();

    let can_save = state.can_save();
    let response = ui.add_enabled_ui(can_save && !state.save_in_progress, |ui| {
        text_ui.button(ui, "skins_save", "Save", &save_style)
    });
    if response.inner.clicked() {
        state.start_save();
    }
}
