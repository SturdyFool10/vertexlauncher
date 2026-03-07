use config::{Config, UiFontFamily};
use egui::Ui;
use instances::InstanceStore;
use textui::TextUi;

use crate::ui::theme::Theme;

mod console;
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
    Console,
    Instance,
}

impl AppScreen {
    pub const FIXED_NAV: [AppScreen; 5] = [
        AppScreen::Library,
        AppScreen::Skins,
        AppScreen::Settings,
        AppScreen::Legal,
        AppScreen::Console,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AppScreen::Library => "Library",
            AppScreen::Skins => "Skins",
            AppScreen::Settings => "Settings",
            AppScreen::Legal => "Legal",
            AppScreen::Console => "Console",
            AppScreen::Instance => "Instance",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScreenOutput {
    pub instances_changed: bool,
    pub requested_screen: Option<AppScreen>,
}

#[derive(Debug, Clone)]
pub struct LaunchAuthContext {
    pub account_key: String,
    pub player_name: String,
    pub player_uuid: String,
    pub access_token: String,
    pub xuid: Option<String>,
    pub user_type: String,
}

pub fn render(
    ui: &mut Ui,
    screen: AppScreen,
    selected_instance_id: Option<&str>,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    active_account_owns_minecraft: bool,
    config: &mut Config,
    instances: &mut InstanceStore,
    available_ui_fonts: &[UiFontFamily],
    available_themes: &[Theme],
    text_ui: &mut TextUi,
) -> ScreenOutput {
    match screen {
        AppScreen::Library => {
            library::render(ui, text_ui, selected_instance_id, instances);
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
        AppScreen::Console => {
            console::render(ui, text_ui);
            ScreenOutput::default()
        }
        AppScreen::Instance => {
            let output = instance::render(
                ui,
                text_ui,
                selected_instance_id,
                active_username,
                active_launch_auth,
                active_account_owns_minecraft,
                instances,
                config,
            );
            ScreenOutput {
                instances_changed: output.instances_changed,
                requested_screen: output.requested_screen,
            }
        }
    }
}
