use std::{collections::HashMap, env};

use config::{Config, UiFontFamily};
use curseforge::set_api_key_override as set_curseforge_api_key_override;
use eframe::egui_wgpu::wgpu;
use egui::Ui;
use instances::InstanceStore;
use textui::TextUi;

use crate::ui::{context_menu, modal, theme::Theme};

mod console;
mod content_browser;
mod discover;
mod home;
mod instance;
mod legal;
mod library;
mod platform;
mod settings;
mod skins;

pub use console::console_log_scroll_id;
pub use console::request_console_tab_focus;
pub use content_browser::ContentBrowserState;
pub use discover::{DiscoverInstallRequest, DiscoverInstallSource, DiscoverState};
pub use home::HomePresenceSection;
pub use home::purge_inactive_state as purge_inactive_home_state;
pub use home::purge_screenshot_state as purge_home_screenshot_state;
pub use instance::InstancePresenceSection;
pub use instance::purge_inactive_state as purge_inactive_instance_state;
pub use instance::purge_screenshot_state as purge_instance_screenshot_state;
pub use library::{
    purge_inactive_state as purge_inactive_library_state, render_global_overlays,
    request_delete_instance,
};
pub use settings::request_theme_focus as request_settings_theme_focus;
pub use skins::classic_model_button_id as skins_classic_model_button_id;
pub use skins::purge_inactive_state as purge_inactive_skins_state;
pub use skins::request_model_focus as request_skins_model_focus;
pub use skins::request_motion_focus as request_skins_motion_focus;
pub use skins::set_gamepad_orbit_input as set_skins_gamepad_orbit_input;
pub use skins::slim_model_button_id as skins_slim_model_button_id;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppScreen {
    Home,
    Library,
    Discover,
    DiscoverDetail,
    ContentBrowser,
    Skins,
    Settings,
    Legal,
    Console,
    Instance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuPresenceContext {
    Screen(AppScreen),
    Home(HomePresenceSection),
    Instance(InstancePresenceSection),
}

impl AppScreen {
    pub const FIXED_NAV: [AppScreen; 7] = [
        AppScreen::Home,
        AppScreen::Library,
        AppScreen::Discover,
        AppScreen::Skins,
        AppScreen::Settings,
        AppScreen::Legal,
        AppScreen::Console,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AppScreen::Home => "Home",
            AppScreen::Library => "Library",
            AppScreen::Discover => "Discover",
            AppScreen::DiscoverDetail => "Discover",
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
/// Actions emitted by the active screen for the application shell to handle.
pub struct ScreenOutput {
    pub instances_changed: bool,
    pub requested_screen: Option<AppScreen>,
    pub selected_instance_id: Option<String>,
    pub delete_requested_instance_id: Option<String>,
    pub discover_install_requested: Option<DiscoverInstallRequest>,
    pub menu_presence_context: Option<MenuPresenceContext>,
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
    pub graphics_api: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickLaunchCommandMode {
    Pack,
    World,
    Server,
}

pub fn selected_quick_launch_user(
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
) -> Option<String> {
    active_launch_auth
        .map(|auth| auth.player_uuid.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            active_username
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
}

pub fn build_quick_launch_command(
    mode: QuickLaunchCommandMode,
    instance: &str,
    user: &str,
    world: Option<&str>,
    server: Option<&str>,
) -> String {
    let executable = env::current_exe()
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "vertexlauncher".to_owned());
    let mut args = vec![shell_escape(executable.as_str())];
    args.extend(build_quick_launch_args(mode, instance, user, world, server));
    args.join(" ")
}

pub fn build_quick_launch_steam_options(
    mode: QuickLaunchCommandMode,
    instance: &str,
    user: &str,
    world: Option<&str>,
    server: Option<&str>,
) -> String {
    build_quick_launch_args(mode, instance, user, world, server).join(" ")
}

fn build_quick_launch_args(
    mode: QuickLaunchCommandMode,
    instance: &str,
    user: &str,
    world: Option<&str>,
    server: Option<&str>,
) -> Vec<String> {
    let mut args = vec![match mode {
        QuickLaunchCommandMode::Pack => "--quick-launch-pack".to_owned(),
        QuickLaunchCommandMode::World => "--quick-launch-world".to_owned(),
        QuickLaunchCommandMode::Server => "--quick-launch-server".to_owned(),
    }];
    args.push("--instance".to_owned());
    args.push(shell_escape(instance));
    args.push("--user".to_owned());
    args.push(shell_escape(user));
    if let Some(world) = world.filter(|value| !value.trim().is_empty()) {
        args.push("--world".to_owned());
        args.push(shell_escape(world));
    }
    if let Some(server) = server.filter(|value| !value.trim().is_empty()) {
        args.push("--server".to_owned());
        args.push(shell_escape(server));
    }
    args
}

fn shell_escape(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '/' | '\\'))
    {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('"', "\\\""))
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
    if modal::close_top(ctx) {
        return true;
    }
    let output = match screen {
        AppScreen::Home => home::handle_escape(ctx),
        AppScreen::Library => library::handle_escape(ctx),
        AppScreen::Instance => instance::handle_escape(ctx, selected_instance_id),
        _ => false,
    };
    output
}

pub fn menu_presence_context(
    ctx: &egui::Context,
    screen: AppScreen,
    selected_instance_id: Option<&str>,
) -> MenuPresenceContext {
    match screen {
        AppScreen::Home => MenuPresenceContext::Home(home::presence_section(ctx)),
        AppScreen::Instance => {
            MenuPresenceContext::Instance(instance::presence_section(ctx, selected_instance_id))
        }
        _ => MenuPresenceContext::Screen(screen),
    }
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
    discover_state: &mut DiscoverState,
    text_ui: &mut TextUi,
) -> ScreenOutput {
    modal::begin_frame(ui.ctx());
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
            let output = home::render(
                ui,
                text_ui,
                instances,
                config,
                active_username,
                active_launch_auth,
                streamer_mode,
            );
            ScreenOutput {
                instances_changed: false,
                requested_screen: output.requested_screen,
                selected_instance_id: output.selected_instance_id,
                delete_requested_instance_id: output.delete_requested_instance_id,
                discover_install_requested: None,
                menu_presence_context: Some(MenuPresenceContext::Home(output.presence_section)),
            }
        }
        AppScreen::Library => {
            let installations_root = config.minecraft_installations_root_path().to_path_buf();
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
                delete_requested_instance_id: None,
                discover_install_requested: None,
                menu_presence_context: Some(MenuPresenceContext::Screen(AppScreen::Library)),
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
                delete_requested_instance_id: None,
                discover_install_requested: None,
                menu_presence_context: Some(MenuPresenceContext::Screen(AppScreen::ContentBrowser)),
            }
        }
        AppScreen::Discover | AppScreen::DiscoverDetail => {
            let output = discover::render(
                ui,
                text_ui,
                discover_state,
                screen == AppScreen::DiscoverDetail,
            );
            ScreenOutput {
                requested_screen: output.requested_screen,
                discover_install_requested: output.install_requested,
                menu_presence_context: Some(MenuPresenceContext::Screen(screen)),
                ..ScreenOutput::default()
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
                config.skin_preview_texel_aa_mode(),
                config.skin_preview_motion_blur_enabled(),
                config.skin_preview_motion_blur_amount(),
                config.skin_preview_motion_blur_shutter_frames(),
                config.skin_preview_motion_blur_sample_count(),
                config.skin_preview_3d_layers_enabled(),
                config.skin_preview_fresh_format_enabled(),
            );
            ScreenOutput {
                menu_presence_context: Some(MenuPresenceContext::Screen(AppScreen::Skins)),
                ..ScreenOutput::default()
            }
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
            ScreenOutput {
                menu_presence_context: Some(MenuPresenceContext::Screen(AppScreen::Settings)),
                ..ScreenOutput::default()
            }
        }
        AppScreen::Legal => {
            legal::render(ui, text_ui);
            ScreenOutput {
                menu_presence_context: Some(MenuPresenceContext::Screen(AppScreen::Legal)),
                ..ScreenOutput::default()
            }
        }
        AppScreen::Console => {
            console::render(ui, text_ui);
            ScreenOutput {
                menu_presence_context: Some(MenuPresenceContext::Screen(AppScreen::Console)),
                ..ScreenOutput::default()
            }
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
                delete_requested_instance_id: None,
                discover_install_requested: None,
                menu_presence_context: Some(MenuPresenceContext::Instance(output.presence_section)),
            }
        }
    };

    render_global_overlays(
        ui.ctx(),
        text_ui,
        instances,
        config.minecraft_installations_root_path(),
    );
    context_menu::show(ui.ctx());
    modal::end_frame(ui.ctx());
    output
}
