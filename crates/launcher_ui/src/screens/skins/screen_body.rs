use super::*;

pub(super) fn render_skin_screen_contents(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut SkinManagerState,
    streamer_mode: bool,
) {
    render_skin_screen_sections(ui, text_ui, state, streamer_mode);
}
