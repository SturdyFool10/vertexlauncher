use super::super::*;

pub(super) fn render_preview_overlay_controls(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut SkinManagerState,
    rect: Rect,
) -> bool {
    let button_size = egui::vec2(154.0, 32.0);
    let button_gap = 8.0;
    let base_x = rect.left() + 14.0;
    let base_y = rect.bottom() - 46.0;

    let mut button_clicked = false;

    let motion_rect = Rect::from_min_size(
        egui::pos2(base_x, base_y - (button_size.y + button_gap)),
        button_size,
    );
    let toggle_rect = Rect::from_min_size(egui::pos2(base_x, base_y), button_size);

    let motion_text = match state.preview_motion_mode {
        PreviewMotionMode::Idle => "Motion: Idle",
        PreviewMotionMode::Walk => "Motion: Walk",
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(motion_rect), |ui| {
        let mut toggle_style = style::neutral_button(ui);
        toggle_style.min_size = motion_rect.size();
        let response = text_ui.button(ui, "skins_toggle_motion_mode", motion_text, &toggle_style);
        let should_force_focus = ui.ctx().data_mut(|data| {
            data.get_temp::<bool>(egui::Id::new(FORCE_MOTION_FOCUS_ID))
                .unwrap_or(false)
        });
        if should_force_focus {
            response.request_focus();
            ui.ctx()
                .data_mut(|data| data.remove::<bool>(egui::Id::new(FORCE_MOTION_FOCUS_ID)));
        }
        if response.clicked() {
            state.preview_motion_mode = match state.preview_motion_mode {
                PreviewMotionMode::Idle => PreviewMotionMode::Walk,
                PreviewMotionMode::Walk => PreviewMotionMode::Idle,
            };
        }
    });

    let toggle_text = if state.show_elytra {
        "Elytra: On"
    } else {
        "Elytra: Off"
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(toggle_rect), |ui| {
        let mut toggle_style = style::neutral_button(ui);
        toggle_style.min_size = toggle_rect.size();
        let response = text_ui.button(
            ui,
            "skins_toggle_elytra_overlay",
            toggle_text,
            &toggle_style,
        );
        button_clicked = response.clicked();
    });

    button_clicked
}
