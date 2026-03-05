use config::Config;
use egui::Ui;

mod legal;
mod library;
mod settings;
mod skins;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppScreen {
    Library,
    Skins,
    Settings,
    Legal,
}

impl AppScreen {
    pub const FIXED_NAV: [AppScreen; 4] = [
        AppScreen::Library,
        AppScreen::Skins,
        AppScreen::Settings,
        AppScreen::Legal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AppScreen::Library => "Library",
            AppScreen::Skins => "Skins",
            AppScreen::Settings => "Settings",
            AppScreen::Legal => "Legal",
        }
    }
}

pub fn render(
    ui: &mut Ui,
    screen: AppScreen,
    selected_profile_id: Option<&str>,
    config: &mut Config,
) {
    match screen {
        AppScreen::Library => library::render(ui, selected_profile_id),
        AppScreen::Skins => skins::render(ui, selected_profile_id),
        AppScreen::Settings => settings::render(ui, config),
        AppScreen::Legal => legal::render(ui),
    }
}
