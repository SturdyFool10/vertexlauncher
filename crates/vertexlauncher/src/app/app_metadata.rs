use launcher_ui::screens::SettingsInfo;
use std::sync::OnceLock;

use super::platform;

#[derive(Debug, Clone)]
struct GraphicsInfo {
    gpu: String,
    driver_name: String,
    driver_version: String,
}

static BASE_SETTINGS_INFO: OnceLock<SettingsInfo> = OnceLock::new();
static GRAPHICS_INFO: OnceLock<GraphicsInfo> = OnceLock::new();

pub fn try_settings_info() -> Option<SettingsInfo> {
    let mut info = BASE_SETTINGS_INFO.get()?.clone();
    apply_graphics_overrides(&mut info);
    Some(info)
}

pub fn preload_settings_info() {
    let _ = BASE_SETTINGS_INFO.get_or_init(build_base_settings_info);
}

fn apply_graphics_overrides(info: &mut SettingsInfo) {
    if let Some(graphics) = GRAPHICS_INFO.get() {
        if !graphics.gpu.is_empty() {
            info.gpu = graphics.gpu.clone();
        }

        let driver_name = clean_value(&graphics.driver_name);
        let driver_version = clean_value(&graphics.driver_version);
        info.graphics_driver = match (driver_name.as_str(), driver_version.as_str()) {
            ("Unknown", "Unknown") => "Unknown".to_owned(),
            ("Unknown", version) => version.to_owned(),
            (name, "Unknown") => name.to_owned(),
            (name, version) => format!("{name} {version}"),
        };
    }
}

pub fn record_graphics_adapter(gpu: &str, driver_name: &str, driver_version: &str) {
    let _ = GRAPHICS_INFO.set(GraphicsInfo {
        gpu: clean_value(gpu),
        driver_name: clean_value(driver_name),
        driver_version: clean_value(driver_version),
    });
}

fn build_base_settings_info() -> SettingsInfo {
    let version = env!("VERTEX_APP_VERSION");
    let revision = env!("VERTEX_GIT_REVISION");

    SettingsInfo {
        cpu: platform::detect_cpu_name(),
        gpu: "Unknown".to_owned(),
        memory: platform::detect_total_memory(),
        graphics_driver: "Unknown".to_owned(),
        app_version: format!("{version} ({revision})"),
    }
}

fn clean_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "Unknown".to_owned()
    } else {
        trimmed.to_owned()
    }
}
