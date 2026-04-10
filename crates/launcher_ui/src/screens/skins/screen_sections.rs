use super::*;

#[path = "screen_sections/account_status_section.rs"]
mod account_status_section;
#[path = "screen_sections/cape_selector_section.rs"]
mod cape_selector_section;
#[path = "screen_sections/model_selector_section.rs"]
mod model_selector_section;
#[path = "screen_sections/save_section.rs"]
mod save_section;
#[path = "screen_sections/skin_picker_section.rs"]
mod skin_picker_section;
use self::account_status_section::render_account_status_section;
use self::cape_selector_section::render_cape_selector_section;
use self::model_selector_section::render_model_selector_section;
use self::save_section::render_save_section;
use self::skin_picker_section::render_skin_picker_section;

pub(super) fn render_skin_screen_sections(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut SkinManagerState,
    streamer_mode: bool,
) {
    if !render_account_status_section(ui, text_ui, state, streamer_mode) {
        return;
    }

    ui.add_space(style::SPACE_MD);
    if render_preview_panel(ui, text_ui, state) {
        state.show_elytra = !state.show_elytra;
    }

    render_skin_picker_section(ui, text_ui, state);
    render_model_selector_section(ui, text_ui, state);
    render_cape_selector_section(ui, text_ui, state);
    render_save_section(ui, text_ui, state);
}
