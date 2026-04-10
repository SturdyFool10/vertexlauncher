use super::super::super::*;

pub(super) fn render_skin_picker_status(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &SkinManagerState,
) {
    let muted = style::muted(ui);

    if state.pick_skin_in_progress {
        ui.add_space(style::SPACE_XS);
        ui.horizontal(|ui| {
            ui.spinner();
            let _ = text_ui.label(
                ui,
                "skins_pick_file_loading",
                "Loading selected skin in the background...",
                &muted,
            );
        });
    }

    if let Some(path) = state.pending_skin_path.as_deref() {
        ui.add_space(style::SPACE_XS);
        let _ = text_ui.label(
            ui,
            "skins_selected_path",
            path.as_os_str().to_string_lossy().as_ref(),
            &muted,
        );
    }
}
