use super::super::*;

pub(super) fn render_cape_selector_section(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut SkinManagerState,
) {
    ui.add_space(style::SPACE_MD);
    let _ = text_ui.label(
        ui,
        "skins_cape_heading",
        "Cape",
        &style::section_heading(ui),
    );
    ui.add_space(style::SPACE_XS);

    render_cape_grid(ui, text_ui, state);
}
