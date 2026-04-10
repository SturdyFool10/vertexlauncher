use std::{collections::HashMap, env};

use config::{Config, UiEmojiFontFamily, UiFontFamily};
use curseforge::set_api_key_override as set_curseforge_api_key_override;
use eframe::egui_wgpu::wgpu;
use egui::Ui;
use instances::InstanceStore;
use textui::TextUi;

use crate::ui::{context_menu, modal, theme::Theme};

#[path = "screens/app_screen.rs"]
mod app_screen;
mod console;
mod content_browser;
mod discover;
mod home;
mod instance;
#[path = "screens/launch_auth_context.rs"]
mod launch_auth_context;
mod legal;
mod library;
#[path = "screens/menu_presence_context.rs"]
mod menu_presence_context;
mod platform;
#[path = "screens/quick_launch_command_mode.rs"]
mod quick_launch_command_mode;
#[path = "screens/screen_output.rs"]
mod screen_output;
mod settings;
#[path = "screens/settings_info.rs"]
mod settings_info;
mod skins;

pub use app_screen::AppScreen;
pub use console::console_log_scroll_id;
pub use console::request_console_tab_focus;
pub use content_browser::ContentBrowserState;
pub use content_browser::loader_dropdown_id as content_browser_loader_dropdown_id;
pub use content_browser::scope_dropdown_id as content_browser_scope_dropdown_id;
pub use content_browser::sort_dropdown_id as content_browser_sort_dropdown_id;
pub use content_browser::version_dropdown_id as content_browser_version_dropdown_id;
pub use discover::{DiscoverInstallRequest, DiscoverInstallSource, DiscoverState};
pub use home::HomePresenceSection;
pub use home::purge_inactive_state as purge_inactive_home_state;
pub use home::purge_screenshot_state as purge_home_screenshot_state;
pub use home::set_gamepad_screenshot_viewer_input as set_home_screenshot_viewer_gamepad_input;
pub use instance::InstancePresenceSection;
pub use instance::instance_content_resource_packs_tab_id;
pub use instance::instance_content_shader_packs_tab_id;
pub use instance::instance_top_content_tab_id;
pub use instance::instance_top_logs_tab_id;
pub use instance::instance_top_screenshots_tab_id;
pub use instance::purge_inactive_state as purge_inactive_instance_state;
pub use instance::purge_screenshot_state as purge_instance_screenshot_state;
pub use instance::set_gamepad_screenshot_viewer_input as set_instance_screenshot_viewer_gamepad_input;
pub use launch_auth_context::LaunchAuthContext;
pub use library::{
    purge_inactive_state as purge_inactive_library_state, render_global_overlays,
    request_delete_instance,
};
pub use menu_presence_context::MenuPresenceContext;
pub use quick_launch_command_mode::QuickLaunchCommandMode;
pub use screen_output::ScreenOutput;
pub use settings::request_theme_focus as request_settings_theme_focus;
pub use settings_info::SettingsInfo;
pub use skins::classic_model_button_id as skins_classic_model_button_id;
pub use skins::purge_inactive_state as purge_inactive_skins_state;
pub use skins::request_model_focus as request_skins_model_focus;
pub use skins::request_motion_focus as request_skins_motion_focus;
pub use skins::set_gamepad_orbit_input as set_skins_gamepad_orbit_input;
pub use skins::slim_model_button_id as skins_slim_model_button_id;

#[derive(Debug, Clone)]
pub(crate) struct PendingLaunchIntent {
    pub nonce: u64,
    pub instance_id: String,
    pub quick_play_singleplayer: Option<String>,
    pub quick_play_multiplayer: Option<String>,
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
    available_emoji_fonts: &[UiEmojiFontFamily],
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
                available_emoji_fonts,
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
