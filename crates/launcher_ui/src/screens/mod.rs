use std::env;


#[path = "screens/app_screen.rs"]
mod app_screen;
mod content_browser;
mod discover;
mod game_settings;
mod home;
mod instance;
#[path = "screens/launch_auth_context.rs"]
mod launch_auth_context;
mod library;
mod platform;
#[path = "screens/menu_presence_context.rs"]
mod menu_presence_context;
#[path = "screens/quick_launch_command_mode.rs"]
mod quick_launch_command_mode;
#[path = "screens/settings_info.rs"]
mod settings_info;
mod skins;

pub use app_screen::AppScreen;
pub use content_browser::ContentBrowserState;
pub use content_browser::api as content_browser_api;
pub use discover::api as discover_api;
pub use discover::{DiscoverInstallRequest, DiscoverInstallSource, DiscoverState};
pub use game_settings::api as game_settings_api;
pub use home::api as home_api;
pub use instance::api as instance_api;
pub use instance::{DetectedInstanceVersions, detect_instance_versions};
pub use launch_auth_context::LaunchAuthContext;
pub use library::{
    LibraryRuntimeState, poll_library_delete_results, poll_library_runtime_actions, request_library_instance_delete,
    request_library_runtime_launch,
};
pub use home::HomePresenceSection;
pub use instance::InstancePresenceSection;
pub use menu_presence_context::MenuPresenceContext;
pub use quick_launch_command_mode::QuickLaunchCommandMode;
pub use settings_info::{SettingsGraphicsAdapterInfo, SettingsInfo};
pub use skins::api as skins_api;
pub use skins::gpu_scene as skins_gpu_scene;


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
