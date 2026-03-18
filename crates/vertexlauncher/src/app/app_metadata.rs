use launcher_ui::screens::SettingsInfo;
use std::sync::OnceLock;

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
    let commit_hash = env!("VERTEX_GIT_COMMIT_HASH");

    SettingsInfo {
        cpu: detect_cpu_name(),
        gpu: "Unknown".to_owned(),
        memory: detect_total_memory(),
        graphics_driver: "Unknown".to_owned(),
        app_version: format!("{version} ({commit_hash})"),
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

#[cfg(target_os = "linux")]
fn detect_cpu_name() -> String {
    use std::fs;

    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find_map(|line| line.strip_prefix("model name\t: ").map(str::to_owned))
        })
        .unwrap_or_else(|| "Unknown".to_owned())
}

#[cfg(target_os = "windows")]
fn detect_cpu_name() -> String {
    std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "Unknown".to_owned())
}

#[cfg(target_os = "macos")]
fn detect_cpu_name() -> String {
    "Unknown".to_owned()
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn detect_cpu_name() -> String {
    "Unknown".to_owned()
}

#[cfg(target_os = "linux")]
fn detect_total_memory() -> String {
    use std::fs;

    fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                let value = line.strip_prefix("MemTotal:")?.trim();
                let kib = value.split_whitespace().next()?.parse::<u64>().ok()?;
                Some(format_memory_from_bytes(kib.saturating_mul(1024)))
            })
        })
        .unwrap_or_else(|| "Unknown".to_owned())
}

#[cfg(target_os = "windows")]
fn detect_total_memory() -> String {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut memory_status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };

    unsafe {
        if GlobalMemoryStatusEx(&mut memory_status).is_ok() {
            format_memory_from_bytes(memory_status.ullTotalPhys)
        } else {
            "Unknown".to_owned()
        }
    }
}

#[cfg(target_os = "macos")]
fn detect_total_memory() -> String {
    "Unknown".to_owned()
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn detect_total_memory() -> String {
    "Unknown".to_owned()
}

fn format_memory_from_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    format!("{:.1} GiB", bytes as f64 / GIB)
}
