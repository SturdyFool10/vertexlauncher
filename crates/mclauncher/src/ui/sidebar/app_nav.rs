use egui::Ui;

use crate::assets;
use crate::screens::AppScreen;
use crate::ui::components::icon_button;

use super::SidebarOutput;

pub fn render(
    ui: &mut Ui,
    active_screen: AppScreen,
    output: &mut SidebarOutput,
    max_icon_width: f32,
) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        for screen in AppScreen::FIXED_NAV {
            let selected = active_screen == screen;
            let (icon_id, icon_bytes) = icon_for_screen(screen);
            let response = ui
                .horizontal_centered(|ui| {
                    icon_button::svg(
                        ui,
                        icon_id,
                        icon_bytes,
                        screen.label(),
                        selected,
                        max_icon_width,
                    )
                })
                .inner;
            if response.clicked() {
                output.selected_screen = Some(screen);
            }
        }
    });
}

fn icon_for_screen(screen: AppScreen) -> (&'static str, &'static [u8]) {
    match screen {
        AppScreen::Library => ("library", assets::LIBRARY_SVG),
        AppScreen::Skins => ("skin_selector", assets::SKIN_SELECTOR_SVG),
        AppScreen::Settings => ("settings", assets::SETTINGS_SVG),
        AppScreen::Legal => ("legal", assets::LEGAL_SVG),
        AppScreen::Instance => ("library", assets::LIBRARY_SVG),
    }
}
