use egui::Ui;

use crate::assets;
use crate::screens::AppScreen;
use crate::ui::components::icon_button;
use crate::ui::style;

use super::SidebarOutput;

const HOME_FOCUS_REQUEST_ID: &str = "sidebar_home_focus_request";

pub fn request_home_focus(ctx: &egui::Context, requested: bool) {
    let key = egui::Id::new(HOME_FOCUS_REQUEST_ID);
    ctx.data_mut(|data| {
        if requested {
            data.insert_temp(key, true);
        } else {
            data.remove::<bool>(key);
        }
    });
}

pub fn render(
    ui: &mut Ui,
    active_screen: AppScreen,
    output: &mut SidebarOutput,
    max_icon_width: f32,
) {
    let row_height = max_icon_width.max(1.0);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = style::SPACE_SM;
        for screen in AppScreen::FIXED_NAV {
            let selected = active_screen == screen
                || (active_screen == AppScreen::DiscoverDetail && screen == AppScreen::Discover);
            let (icon_id, icon_bytes) = icon_for_screen(screen);
            let response = ui
                .allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), row_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        icon_button::svg(
                            ui,
                            icon_id,
                            icon_bytes,
                            screen.label(),
                            selected,
                            max_icon_width,
                        )
                    },
                )
                .inner;
            let home_focus_requested = ui
                .ctx()
                .data_mut(|data| {
                    data.get_temp::<bool>(egui::Id::new(HOME_FOCUS_REQUEST_ID))
                        .unwrap_or(false)
                });
            if screen == AppScreen::Home
                && (ui.ctx().memory(|memory| memory.focused().is_none()) || home_focus_requested)
            {
                response.request_focus();
                request_home_focus(ui.ctx(), false);
            }
            if response.clicked() {
                output.selected_screen = Some(screen);
            }
        }
    });
}

fn icon_for_screen(screen: AppScreen) -> (&'static str, &'static [u8]) {
    match screen {
        AppScreen::Home => ("home", assets::HOME_SVG),
        AppScreen::Library => ("library", assets::LIBRARY_SVG),
        AppScreen::Discover => ("discover", assets::DISCOVER_SVG),
        AppScreen::DiscoverDetail => ("discover", assets::DISCOVER_SVG),
        AppScreen::ContentBrowser => ("content_browser", assets::SHOPPING_CART_SVG),
        AppScreen::Skins => ("skin_selector", assets::SKIN_SELECTOR_SVG),
        AppScreen::Settings => ("settings", assets::SETTINGS_SVG),
        AppScreen::Legal => ("legal", assets::LEGAL_SVG),
        AppScreen::Console => ("console", assets::TERMINAL_SVG),
        AppScreen::Instance => ("library", assets::LIBRARY_SVG),
    }
}
