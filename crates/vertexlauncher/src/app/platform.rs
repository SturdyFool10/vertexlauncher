use config::GraphicsApiPreference;
use eframe::{self, egui_wgpu::wgpu};
#[cfg(target_os = "macos")]
use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) struct StartupGraphicsConfig {
    pub renderer: eframe::Renderer,
    pub hardware_acceleration: eframe::HardwareAcceleration,
    pub backends: wgpu::Backends,
    pub graphics_api_preference: GraphicsApiPreference,
}

pub(crate) fn startup_graphics_config(
    transparent_viewport: bool,
    graphics_api_preference: GraphicsApiPreference,
) -> StartupGraphicsConfig {
    let resolved_graphics_api_preference =
        resolve_graphics_api_preference(graphics_api_preference, transparent_viewport);
    StartupGraphicsConfig {
        renderer: eframe::Renderer::Wgpu,
        hardware_acceleration: eframe::HardwareAcceleration::Required,
        backends: startup_backends(
            transparent_viewport,
            graphics_api_preference,
            resolved_graphics_api_preference,
        ),
        graphics_api_preference: resolved_graphics_api_preference,
    }
}

pub(crate) fn log_startup_graphics_choice(config: StartupGraphicsConfig) {
    tracing::info!(
        target: "vertexlauncher/app/graphics",
        renderer = ?config.renderer,
        hardware_acceleration = ?config.hardware_acceleration,
        backends = ?config.backends,
        graphics_api_preference = ?config.graphics_api_preference,
        "Startup graphics configuration selected."
    );
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

pub(crate) fn resolve_graphics_api_preference(
    preference: GraphicsApiPreference,
    transparent_viewport: bool,
) -> GraphicsApiPreference {
    match preference {
        GraphicsApiPreference::Auto => auto_graphics_api_preference(transparent_viewport),
        GraphicsApiPreference::Metal if cfg!(target_os = "macos") => GraphicsApiPreference::Metal,
        GraphicsApiPreference::Dx12 if cfg!(target_os = "windows") => GraphicsApiPreference::Dx12,
        GraphicsApiPreference::Vulkan if cfg!(target_os = "windows") => {
            if transparent_viewport {
                auto_graphics_api_preference(transparent_viewport)
            } else {
                GraphicsApiPreference::Vulkan
            }
        }
        GraphicsApiPreference::Vulkan if !cfg!(target_os = "macos") => {
            GraphicsApiPreference::Vulkan
        }
        _ => auto_graphics_api_preference(transparent_viewport),
    }
}

fn startup_backends(
    transparent_viewport: bool,
    graphics_api_preference: GraphicsApiPreference,
    resolved_graphics_api_preference: GraphicsApiPreference,
) -> wgpu::Backends {
    if graphics_api_preference == GraphicsApiPreference::Auto {
        return auto_graphics_api_backends(transparent_viewport, resolved_graphics_api_preference);
    }

    match resolved_graphics_api_preference {
        GraphicsApiPreference::Vulkan => return wgpu::Backends::VULKAN,
        GraphicsApiPreference::Metal => return wgpu::Backends::METAL,
        GraphicsApiPreference::Dx12 => return wgpu::Backends::DX12,
        GraphicsApiPreference::Auto => {}
    }

    #[cfg(target_os = "macos")]
    {
        let _ = transparent_viewport;
        return wgpu::Backends::METAL;
    }

    #[cfg(target_os = "windows")]
    {
        if transparent_viewport {
            return wgpu::Backends::DX12;
        }

        return wgpu::Backends::DX12 | wgpu::Backends::VULKAN;
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let _ = transparent_viewport;
        return wgpu::Backends::VULKAN;
    }
}

fn auto_graphics_api_preference(transparent_viewport: bool) -> GraphicsApiPreference {
    #[cfg(target_os = "windows")]
    {
        let _ = transparent_viewport;
        return GraphicsApiPreference::Dx12;
    }

    #[cfg(target_os = "macos")]
    {
        let _ = transparent_viewport;
        return if moltenvk_present() {
            GraphicsApiPreference::Vulkan
        } else {
            GraphicsApiPreference::Metal
        };
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let _ = transparent_viewport;
        GraphicsApiPreference::Vulkan
    }
}

fn auto_graphics_api_backends(
    transparent_viewport: bool,
    resolved_graphics_api_preference: GraphicsApiPreference,
) -> wgpu::Backends {
    #[cfg(target_os = "windows")]
    {
        let _ = transparent_viewport;
        let _ = resolved_graphics_api_preference;
        return wgpu::Backends::DX12 | wgpu::Backends::VULKAN;
    }

    #[cfg(target_os = "macos")]
    {
        let _ = transparent_viewport;
        return match resolved_graphics_api_preference {
            GraphicsApiPreference::Vulkan => wgpu::Backends::VULKAN | wgpu::Backends::METAL,
            GraphicsApiPreference::Metal => wgpu::Backends::METAL,
            GraphicsApiPreference::Dx12 | GraphicsApiPreference::Auto => wgpu::Backends::METAL,
        };
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let _ = transparent_viewport;
        let _ = resolved_graphics_api_preference;
        wgpu::Backends::VULKAN
    }
}

#[cfg(target_os = "macos")]
fn moltenvk_present() -> bool {
    if std::env::var_os("VK_ICD_FILENAMES").is_some()
        || std::env::var_os("VK_DRIVER_FILES").is_some()
    {
        return true;
    }

    [
        "/usr/local/lib/libMoltenVK.dylib",
        "/opt/homebrew/lib/libMoltenVK.dylib",
        "/Library/Frameworks/MoltenVK.framework/MoltenVK",
        "/usr/local/share/vulkan/icd.d/MoltenVK_icd.json",
        "/opt/homebrew/share/vulkan/icd.d/MoltenVK_icd.json",
    ]
    .into_iter()
    .any(|path| Path::new(path).exists())
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn format_memory_from_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    format!("{:.1} GiB", bytes as f64 / GIB)
}
