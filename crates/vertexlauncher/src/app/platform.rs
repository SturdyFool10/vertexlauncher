use eframe::{self, egui_wgpu::wgpu};

#[derive(Clone, Copy)]
pub(crate) struct StartupGraphicsConfig {
    pub renderer: eframe::Renderer,
    pub hardware_acceleration: eframe::HardwareAcceleration,
    pub backends: wgpu::Backends,
}

pub(crate) fn startup_graphics_config() -> StartupGraphicsConfig {
    StartupGraphicsConfig {
        renderer: eframe::Renderer::Wgpu,
        hardware_acceleration: eframe::HardwareAcceleration::Required,
        backends: startup_backends(),
    }
}

pub(crate) fn log_startup_graphics_choice(config: StartupGraphicsConfig) {
    let _ = config;
}

pub(crate) fn detect_cpu_name() -> String {
    #[cfg(target_os = "linux")]
    {
        use std::fs;

        return fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|contents| {
                contents
                    .lines()
                    .find_map(|line| line.strip_prefix("model name\t: ").map(str::to_owned))
            })
            .unwrap_or_else(|| "Unknown".to_owned());
    }

    #[cfg(target_os = "windows")]
    {
        return detect_windows_cpu_name()
            .or_else(|| std::env::var("PROCESSOR_IDENTIFIER").ok())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Unknown".to_owned());
    }

    #[allow(unreachable_code)]
    "Unknown".to_owned()
}

#[cfg(target_os = "windows")]
fn detect_windows_cpu_name() -> Option<String> {
    use std::process::Command;

    let output = Command::new("reg")
        .args([
            "query",
            r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "/v",
            "ProcessorNameString",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    stdout.lines().find_map(|line| {
        if !line.contains("ProcessorNameString") {
            return None;
        }

        let mut parts = line.split_whitespace();
        let _value_name = parts.next()?;
        let value_type = parts.next()?;
        if !value_type.eq_ignore_ascii_case("REG_SZ") {
            return None;
        }
        let value = parts.collect::<Vec<_>>().join(" ");
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

pub(crate) fn detect_total_memory() -> String {
    #[cfg(target_os = "linux")]
    {
        use std::fs;

        return fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|contents| {
                contents.lines().find_map(|line| {
                    let value = line.strip_prefix("MemTotal:")?.trim();
                    let kib = value.split_whitespace().next()?.parse::<u64>().ok()?;
                    Some(format_memory_from_bytes(kib.saturating_mul(1024)))
                })
            })
            .unwrap_or_else(|| "Unknown".to_owned());
    }

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

        let mut memory_status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };

        return unsafe {
            if GlobalMemoryStatusEx(&mut memory_status).as_bool() {
                format_memory_from_bytes(memory_status.ullTotalPhys)
            } else {
                "Unknown".to_owned()
            }
        };
    }

    #[allow(unreachable_code)]
    "Unknown".to_owned()
}

fn startup_backends() -> wgpu::Backends {
    #[cfg(target_os = "macos")]
    {
        return wgpu::Backends::METAL;
    }

    #[cfg(not(target_os = "macos"))]
    {
        return wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12;
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn format_memory_from_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    format!("{:.1} GiB", bytes as f64 / GIB)
}
