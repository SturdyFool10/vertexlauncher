use super::super::skin_drop_zone::render_skin_drop_zone;
use super::super::*;

#[path = "skin_picker_status.rs"]
mod skin_picker_status;
use self::skin_picker_status::render_skin_picker_status;

pub(super) fn render_skin_picker_section(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut SkinManagerState,
) {
    ui.add_space(style::SPACE_LG);
    let _ = text_ui.label(
        ui,
        "skins_picker_heading",
        "Skin Image",
        &style::section_heading(ui),
    );

    render_skin_drop_zone(ui, text_ui, state);
    render_skin_picker_status(ui, text_ui, state);
}
