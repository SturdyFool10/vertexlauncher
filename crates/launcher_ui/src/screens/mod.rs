use std::collections::HashMap;

use config::{Config, UiFontFamily};
use curseforge::set_api_key_override as set_curseforge_api_key_override;
use eframe::egui_wgpu::wgpu;
use egui::Ui;
use instances::InstanceStore;
use textui::TextUi;

use crate::ui::{context_menu, theme::Theme};

mod console;
mod content_browser;
mod home;
mod instance;
mod legal;
mod library;
mod platform;
mod settings;
mod skins;

pub use content_browser::ContentBrowserState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppScreen {
    Home,
    Library,
    ContentBrowser,
    Skins,
    Settings,
    Legal,
    Console,
    Instance,
}

impl AppScreen {
    pub const FIXED_NAV: [AppScreen; 6] = [
        AppScreen::Home,
        AppScreen::Library,
        AppScreen::Skins,
        AppScreen::Settings,
        AppScreen::Legal,
        AppScreen::Console,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AppScreen::Home => "Home",
            AppScreen::Library => "Library",
            AppScreen::ContentBrowser => "Content Browser",
            AppScreen::Skins => "Skin Manager",
            AppScreen::Settings => "Settings",
            AppScreen::Legal => "Legal",
            AppScreen::Console => "Console",
            AppScreen::Instance => "Instance",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScreenOutput {
    pub instances_changed: bool,
    pub requested_screen: Option<AppScreen>,
    pub selected_instance_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LaunchAuthContext {
    pub account_key: String,
    pub player_name: String,
    pub player_uuid: String,
    pub access_token: Option<String>,
    pub xuid: Option<String>,
    pub user_type: String,
}

#[derive(Debug, Clone, Default)]
pub struct SettingsInfo {
    pub cpu: String,
    pub gpu: String,
    pub memory: String,
    pub graphics_driver: String,
    pub app_version: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingLaunchIntent {
    pub nonce: u64,
    pub instance_id: String,
    pub quick_play_singleplayer: Option<String>,
    pub quick_play_multiplayer: Option<String>,
}

pub(crate) fn queue_launch_intent(ctx: &egui::Context, intent: PendingLaunchIntent) {
    let id = egui::Id::new("pending_launch_intent");
    ctx.data_mut(|data| data.insert_temp(id, intent));
}

pub(crate) fn peek_launch_intent(ctx: &egui::Context) -> Option<PendingLaunchIntent> {
    let id = egui::Id::new("pending_launch_intent");
    ctx.data_mut(|data| data.get_temp::<PendingLaunchIntent>(id))
}

pub fn handle_escape(
    ctx: &egui::Context,
    screen: AppScreen,
    selected_instance_id: Option<&str>,
) -> bool {
    let output = match screen {
        AppScreen::Home => home::handle_escape(ctx),
        AppScreen::Library => library::handle_escape(ctx),
        AppScreen::Instance => instance::handle_escape(ctx, selected_instance_id),
        _ => false,
    };
    output
}

pub fn render(
    ui: &mut Ui,
    screen: AppScreen,
    skin_manager_opened: bool,
    skin_manager_account_switched: bool,
    selected_instance_id: Option<&str>,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    active_account_owns_minecraft: bool,
    streamer_mode: bool,
    config: &mut Config,
    instances: &mut InstanceStore,
    account_avatars_by_key: &HashMap<String, Vec<u8>>,
    wgpu_target_format: Option<wgpu::TextureFormat>,
    skin_preview_msaa_samples: u32,
    available_ui_fonts: &[UiFontFamily],
    available_themes: &[Theme],
    settings_info: &SettingsInfo,
    content_browser_state: &mut ContentBrowserState,
    text_ui: &mut TextUi,
) -> ScreenOutput {
    let content_browser_open_id = ui.make_persistent_id("content_browser_open_state");
    let content_browser_was_open = ui
        .ctx()
        .data_mut(|data| data.get_temp::<bool>(content_browser_open_id))
        .unwrap_or(false);
    let content_browser_is_open = screen == AppScreen::ContentBrowser;
    let reset_content_browser = content_browser_is_open && !content_browser_was_open;
    ui.ctx()
        .data_mut(|data| data.insert_temp(content_browser_open_id, content_browser_is_open));

    set_curseforge_api_key_override(
        (!config.curseforge_api_key().trim().is_empty())
            .then(|| config.curseforge_api_key().to_owned()),
    );

    let output = match screen {
        AppScreen::Home => {
            let output = home::render(ui, text_ui, instances, config, streamer_mode);
            ScreenOutput {
                instances_changed: false,
                requested_screen: output.requested_screen,
                selected_instance_id: output.selected_instance_id,
            }
        }
        AppScreen::Library => {
            let installations_root =
                std::path::PathBuf::from(config.minecraft_installations_root());
            let output = library::render(
                ui,
                text_ui,
                selected_instance_id,
                active_username,
                active_launch_auth,
                active_account_owns_minecraft,
                streamer_mode,
                instances,
                installations_root.as_path(),
                config,
                account_avatars_by_key,
            );
            ScreenOutput {
                instances_changed: false,
                requested_screen: output.requested_screen,
                selected_instance_id: output.selected_instance_id,
            }
        }
        AppScreen::ContentBrowser => {
            let output = content_browser::render(
                ui,
                text_ui,
                selected_instance_id,
                instances,
                config,
                content_browser_state,
                reset_content_browser,
            );
            ScreenOutput {
                instances_changed: false,
                requested_screen: output.requested_screen,
                selected_instance_id: None,
            }
        }
        AppScreen::Skins => {
            skins::render(
                ui,
                text_ui,
                selected_instance_id,
                active_launch_auth,
                skin_manager_opened,
                skin_manager_account_switched,
                streamer_mode,
                wgpu_target_format,
                skin_preview_msaa_samples,
                config.skin_preview_aa_mode(),
                config.skin_preview_motion_blur_enabled(),
                config.skin_preview_motion_blur_amount(),
                config.skin_preview_motion_blur_shutter_frames(),
                config.skin_preview_motion_blur_sample_count(),
                config.skin_preview_3d_layers_enabled(),
            );
            ScreenOutput::default()
        }
        AppScreen::Settings => {
            settings::render(
                ui,
                text_ui,
                config,
                available_ui_fonts,
                available_themes,
                settings_info,
            );
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
                streamer_mode,
                instances,
                config,
                account_avatars_by_key,
            );
            ScreenOutput {
                instances_changed: output.instances_changed,
                requested_screen: output.requested_screen,
                selected_instance_id: None,
            }
        }
    };

    context_menu::show(ui.ctx());
    output
}
