use config::{Config, UiFontFamily};
use egui::Ui;
use instances::InstanceStore;
use textui::TextUi;

use crate::ui::theme::Theme;

mod instance;
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
    Instance,
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
            AppScreen::Instance => "Instance",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScreenOutput {
    pub instances_changed: bool,
}

pub fn render(
    ui: &mut Ui,
    screen: AppScreen,
    selected_instance_id: Option<&str>,
    config: &mut Config,
    instances: &mut InstanceStore,
    available_ui_fonts: &[UiFontFamily],
    available_themes: &[Theme],
    text_ui: &mut TextUi,
) -> ScreenOutput {
    match screen {
        AppScreen::Library => {
            library::render(ui, text_ui, selected_instance_id);
            ScreenOutput::default()
        }
        AppScreen::Skins => {
            skins::render(ui, text_ui, selected_instance_id);
            ScreenOutput::default()
        }
        AppScreen::Settings => {
            settings::render(ui, text_ui, config, available_ui_fonts, available_themes);
            ScreenOutput::default()
        }
        AppScreen::Legal => {
            legal::render(ui, text_ui);
            ScreenOutput::default()
        }
        AppScreen::Instance => ScreenOutput {
            instances_changed: instance::render(
                ui,
                text_ui,
                selected_instance_id,
                instances,
                config,
            ),
        },
    }
}
