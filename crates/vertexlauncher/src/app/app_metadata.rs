use launcher_ui::screens::{SettingsGraphicsAdapterInfo, SettingsInfo};
use std::sync::OnceLock;
use vertex_3d::AvailableAdapter;

use super::platform;

#[derive(Debug, Clone)]
struct GraphicsInfo {
    gpu: String,
    graphics_api: String,
    driver_name: String,
    driver_version: String,
}

static BASE_SETTINGS_INFO: OnceLock<SettingsInfo> = OnceLock::new();
static GRAPHICS_INFO: OnceLock<GraphicsInfo> = OnceLock::new();
static AVAILABLE_GRAPHICS_ADAPTERS: OnceLock<Vec<SettingsGraphicsAdapterInfo>> = OnceLock::new();

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
        info.graphics_api = clean_value(&graphics.graphics_api);

        let driver_name = clean_value(&graphics.driver_name);
        let driver_version = clean_value(&graphics.driver_version);
        info.graphics_driver = match (driver_name.as_str(), driver_version.as_str()) {
            ("Unknown", "Unknown") => "Unknown".to_owned(),
            ("Unknown", version) => version.to_owned(),
            (name, "Unknown") => name.to_owned(),
            (name, version) => format!("{name} {version}"),
        };
    }
    if let Some(adapters) = AVAILABLE_GRAPHICS_ADAPTERS.get() {
        info.available_graphics_adapters = adapters.clone();
    }
}

pub fn record_graphics_adapter(
    gpu: &str,
    graphics_api: &str,
    driver_name: &str,
    driver_version: &str,
) {
    let _ = GRAPHICS_INFO.set(GraphicsInfo {
        gpu: clean_value(gpu),
        graphics_api: clean_value(graphics_api),
        driver_name: clean_value(driver_name),
        driver_version: clean_value(driver_version),
    });
}

pub fn record_available_graphics_adapters(adapters: &[AvailableAdapter]) {
    let _ = AVAILABLE_GRAPHICS_ADAPTERS.set(
        adapters
            .iter()
            .map(|adapter| SettingsGraphicsAdapterInfo {
                label: graphics_adapter_label(adapter),
                hash: adapter.selection_hash,
            })
            .collect(),
    );
}

fn build_base_settings_info() -> SettingsInfo {
    let version = env!("VERTEX_APP_VERSION");
    let revision = env!("VERTEX_GIT_REVISION");

    SettingsInfo {
        cpu: platform::detect_cpu_name(),
        gpu: "Unknown".to_owned(),
        memory: platform::detect_total_memory(),
        graphics_api: "Unknown".to_owned(),
        graphics_driver: "Unknown".to_owned(),
        app_version: format!("{version} ({revision})"),
        available_graphics_adapters: Vec::new(),
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

fn clean_driver_label(driver_name: &str, driver_info: &str) -> String {
    let driver_name = clean_value(driver_name);
    if driver_name != "Unknown" {
        return driver_name;
    }

    let driver_info = clean_value(driver_info);
    if driver_info != "Unknown" {
        return driver_info;
    }

    "Unknown Driver".to_owned()
}

fn graphics_adapter_label(adapter: &AvailableAdapter) -> String {
    let name = clean_value(&adapter.name);
    let driver = clean_driver_label(&adapter.driver, &adapter.driver_info);
    if driver == "Unknown Driver" || name_already_contains_driver(&name, &driver) {
        return name;
    }

    format!("{name} ({driver})")
}

fn name_already_contains_driver(name: &str, driver: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let driver = driver.to_ascii_lowercase();
    name.contains(&driver)
}
