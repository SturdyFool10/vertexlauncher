use config::Config;
use eframe::{self, egui, egui_wgpu::wgpu};
use launcher_ui::window_effects;
use std::fmt::Write as _;
use std::sync::Arc;
use vertex_3d::{
    AdapterPreference, AdapterSelector, describe_adapter_slice, select_adapter_from_slice,
};
use vertex_constants::branding::DESKTOP_APP_ID;

use super::{app_icon, app_metadata, platform};

fn adapter_preference_for_profile(profile: config::GraphicsAdapterProfile) -> AdapterPreference {
    match profile {
        config::GraphicsAdapterProfile::Default => AdapterPreference::Default,
        config::GraphicsAdapterProfile::HighPerformance => AdapterPreference::HighPerformance,
        config::GraphicsAdapterProfile::LowPower => AdapterPreference::LowPower,
        config::GraphicsAdapterProfile::DiscreteOnly => AdapterPreference::DiscreteOnly,
        config::GraphicsAdapterProfile::IntegratedOnly => AdapterPreference::IntegratedOnly,
    }
}

pub fn build(startup_config: &Config) -> eframe::NativeOptions {
    let startup_power_preference = startup_config.graphics_adapter_profile();
    let startup_power_preference =
        adapter_preference_for_profile(startup_power_preference).power_preference();
    let blur_enabled =
        startup_config.window_blur_enabled() && window_effects::platform_supports_blur();
    let transparent_viewport = blur_enabled;
    let startup_graphics = platform::startup_graphics_config(
        transparent_viewport,
        startup_config.graphics_api_preference(),
    );
    let renderer = startup_graphics.renderer;
    let hardware_acceleration = startup_graphics.hardware_acceleration;

    platform::log_startup_graphics_choice(startup_graphics);

    let mut wgpu_setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    wgpu_setup.instance_descriptor.backends = startup_graphics.backends;
    wgpu_setup.instance_descriptor.backend_options =
        transparent_backend_options(transparent_viewport);
    wgpu_setup.power_preference = startup_power_preference;
    let startup_adapter_selector = match startup_config.graphics_adapter_preference_type() {
        config::GraphicsAdapterPreferenceType::PerformanceProfile => AdapterSelector::Preference(
            adapter_preference_for_profile(startup_config.graphics_adapter_profile()),
        ),
        config::GraphicsAdapterPreferenceType::ExplicitAdapter => startup_config
            .graphics_adapter_explicit_hash()
            .map(AdapterSelector::Hashed)
            .unwrap_or(AdapterSelector::Preference(
                AdapterPreference::HighPerformance,
            )),
    };
    wgpu_setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
        select_adapter_or_diagnose(adapters, surface, startup_adapter_selector)
    }));
    wgpu_setup.device_descriptor = Arc::new(|adapter| {
        let info = adapter.get_info();
        let graphics_api = graphics_api_label(&format!("{:?}", info.backend));
        app_metadata::record_graphics_adapter(
            &info.name,
            &graphics_api,
            &info.driver,
            &info.driver_info,
        );
        tracing::info!(
            target: "vertexlauncher/app/graphics",
            "Selected graphics adapter: {} backend={:?} type={:?} vendor=0x{:04x} device=0x{:04x}",
            info.name,
            info.backend,
            info.device_type,
            info.vendor,
            info.device
        );

        let base_limits = if info.backend == wgpu::Backend::Gl {
            wgpu::Limits::downlevel_webgl2_defaults()
        } else {
            wgpu::Limits::default()
        };
        let adapter_limits = adapter.limits();

        wgpu::DeviceDescriptor {
            label: Some("egui wgpu device"),
            required_limits: base_limits.using_resolution(adapter_limits),
            ..Default::default()
        }
    });

    eframe::NativeOptions {
        viewport: egui::ViewportBuilder {
            title: Some("Vertex Launcher".into()),
            app_id: Some(DESKTOP_APP_ID.into()),
            inner_size: Some(egui::vec2(1280.0, 800.0)),
            min_inner_size: Some(egui::vec2(900.0, 460.0)),
            resizable: Some(true),
            decorations: Some(false),
            transparent: Some(transparent_viewport),
            icon: app_icon::egui_icon(),
            ..Default::default()
        },
        renderer,
        hardware_acceleration,
        vsync: true,
        multisampling: 4,
        depth_buffer: 32,
        stencil_buffer: 0,
        dithering: false,
        centered: false,
        persist_window: false,
        event_loop_builder: None,
        window_builder: None,
        run_and_return: false,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: Some(2),
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(wgpu_setup),
            on_surface_status: std::sync::Arc::new(|_| {
                eframe::egui_wgpu::SurfaceErrorAction::RecreateSurface
            }),
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Adapter selection with diagnostics
// ---------------------------------------------------------------------------

/// Selects the best hardware adapter whose surface compatibility is confirmed,
/// or emits a detailed diagnostic and returns an error.
///
/// Software renderers (DeviceType::Cpu) are never returned; the launcher
/// requires hardware acceleration. There is no OpenGL or software fallback.
fn select_adapter_or_diagnose(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
    selector: AdapterSelector,
) -> Result<wgpu::Adapter, String> {
    tracing::info!(
        target: "vertexlauncher/app/graphics",
        "Evaluating {} GPU adapter(s) for hardware + surface compatibility.",
        adapters.len()
    );

    let described = describe_adapter_slice(adapters, surface);
    app_metadata::record_available_graphics_adapters(&described);

    for adapter in &described {
        let info = &adapters[adapter.slot].get_info();

        tracing::info!(
            target: "vertexlauncher/app/graphics",
            "  [{i}] {:?} | backend={:?} | type={:?} | vendor=0x{:04x} | device=0x{:04x} | driver={:?} | surface_ok={}",
            info.name, info.backend, info.device_type,
            info.vendor, info.device, info.driver, adapter.surface_supported,
            i = adapter.slot
        );
    }

    let fallback_selector = AdapterSelector::Preference(AdapterPreference::HighPerformance);
    let selected = select_adapter_from_slice(adapters, surface, selector)
        .or_else(|| matches!(selector, AdapterSelector::Hashed(_)).then(|| {
            tracing::warn!(
                target: "vertexlauncher/app/graphics",
                "Explicit graphics adapter selection could not be resolved; falling back to High Performance profile."
            );
            select_adapter_from_slice(adapters, surface, fallback_selector)
        }).flatten());

    if let Some(adapter) = selected {
        let info = adapter.get_info();
        let i = described
            .iter()
            .find(|candidate| {
                candidate.name == info.name
                    && candidate.backend == info.backend
                    && candidate.vendor == info.vendor
                    && candidate.device == info.device
            })
            .map(|candidate| candidate.slot)
            .unwrap_or_default();
        tracing::info!(
            target: "vertexlauncher/app/graphics",
            "Chose adapter [{i}] {:?} (backend={:?} type={:?})",
            info.name, info.backend, info.device_type
        );
        return Ok(adapter);
    }

    // Nothing usable — build and emit the diagnostic before returning the error.
    let diag = build_adapter_diagnostic(adapters, surface);
    eprintln!("{diag}");
    tracing::error!(target: "vertexlauncher/app/graphics", "{diag}");
    Err(diag)
}

fn build_adapter_diagnostic(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
) -> String {
    let mut out = String::new();

    writeln!(out).ok();
    writeln!(
        out,
        "╔══════════════════════════════════════════════════════╗"
    )
    .ok();
    writeln!(
        out,
        "║  Vertex Launcher — GPU initialisation failed         ║"
    )
    .ok();
    writeln!(
        out,
        "╚══════════════════════════════════════════════════════╝"
    )
    .ok();
    writeln!(out).ok();
    writeln!(
        out,
        "Hardware GPU acceleration is required. Software rendering and OpenGL are not supported."
    )
    .ok();
    writeln!(out).ok();

    if adapters.is_empty() {
        writeln!(
            out,
            "No GPU adapters were enumerated for the active backends."
        )
        .ok();
        writeln!(out).ok();
        writeln!(out, "Likely causes:").ok();
        writeln!(
            out,
            "  • No Vulkan-capable GPU or Vulkan driver is installed."
        )
        .ok();
        writeln!(out, "  • The GPU device nodes are not accessible:").ok();

        #[cfg(target_os = "linux")]
        emit_dri_status(&mut out);

        writeln!(out).ok();
        writeln!(
            out,
            "  • You are running inside a container or Flatpak without --device=all."
        )
        .ok();
        writeln!(
            out,
            "  • The WGPU_BACKEND environment variable is forcing a backend with no adapters."
        )
        .ok();
    } else {
        let all_software = adapters
            .iter()
            .all(|a| matches!(a.get_info().device_type, wgpu::DeviceType::Cpu));

        if all_software {
            writeln!(
                out,
                "Only software/CPU renderers were found. Hardware GPU acceleration is required."
            )
            .ok();
        } else {
            writeln!(
                out,
                "Hardware GPU adapter(s) were found but none can present to the display surface."
            )
            .ok();
        }
        writeln!(out).ok();

        for (i, adapter) in adapters.iter().enumerate() {
            let info = adapter.get_info();
            let is_software = matches!(info.device_type, wgpu::DeviceType::Cpu);
            let surface_ok = surface.map_or(true, |s| adapter.is_surface_supported(s));

            writeln!(out, "  Adapter [{i}]: {:?}", info.name).ok();
            writeln!(out, "    Backend  : {:?}", info.backend).ok();
            writeln!(out, "    Type     : {:?}", info.device_type).ok();
            writeln!(out, "    Vendor   : 0x{:04x}", info.vendor).ok();
            writeln!(out, "    Device   : 0x{:04x}", info.device).ok();
            writeln!(out, "    Driver   : {} ({})", info.driver, info.driver_info).ok();
            writeln!(
                out,
                "    Surface  : {}",
                if surface_ok {
                    "compatible"
                } else {
                    "NOT compatible with this window surface"
                }
            )
            .ok();

            if !is_software && !surface_ok {
                writeln!(
                    out,
                    "    Why      : {}",
                    surface_incompatibility_reason(&info)
                )
                .ok();
            } else if is_software {
                writeln!(
                    out,
                    "    Why      : software renderer — hardware acceleration required."
                )
                .ok();
            }
            writeln!(out).ok();
        }
    }

    writeln!(out, "Environment:").ok();
    for var in &[
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "WGPU_BACKEND",
        "WGPU_POWER_PREF",
        "XDG_SESSION_TYPE",
        "GDK_BACKEND",
    ] {
        let val = std::env::var(var).unwrap_or_else(|_| "(not set)".into());
        writeln!(out, "  {var:<22} = {val}").ok();
    }

    out
}

/// Returns a human-readable explanation for why a hardware adapter cannot
/// present to the window surface, based on the backend and platform.
fn surface_incompatibility_reason(info: &wgpu::AdapterInfo) -> &'static str {
    match info.backend {
        wgpu::Backend::Vulkan => {
            // The DMA-buf import error is the most common cause on Wayland.
            // NVIDIA proprietary drivers frequently fail here when paired with
            // compositors that do not support the GPU's preferred DMA-buf
            // format modifiers.
            "Vulkan adapter cannot import the Wayland surface buffers. \
             This usually means the DMA-buf format or modifier negotiation \
             between the Vulkan driver and the Wayland compositor failed. \
             Common on NVIDIA proprietary drivers with certain compositors. \
             Ensure Mesa or the NVIDIA driver is up to date, or try running \
             under an Xwayland session (DISPLAY set, WAYLAND_DISPLAY unset)."
        }
        wgpu::Backend::Gl => "OpenGL adapter — the launcher does not use the OpenGL backend.",
        wgpu::Backend::Metal => {
            "Metal adapter found on a non-macOS platform — this should not happen."
        }
        wgpu::Backend::Dx12 => {
            "DirectX 12 adapter found on a non-Windows platform — this should not happen."
        }
        _ => "Adapter cannot present to this surface for an unknown reason.",
    }
}

#[cfg(target_os = "linux")]
fn emit_dri_status(out: &mut String) {
    match std::fs::read_dir("/dev/dri") {
        Ok(entries) => {
            let nodes: Vec<_> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            if nodes.is_empty() {
                writeln!(out, "    /dev/dri  : exists but contains no device nodes.").ok();
            } else {
                writeln!(out, "    /dev/dri  : {}", nodes.join(", ")).ok();
            }
        }
        Err(e) => {
            writeln!(
                out,
                "    /dev/dri  : not accessible ({e}) — \
                 GPU passthrough or --device=all may be missing."
            )
            .ok();
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers (unchanged)
// ---------------------------------------------------------------------------

fn graphics_api_label(backend_name: &str) -> String {
    let backend_name = match backend_name {
        "Dx12" => "DX12",
        "Dx11" => "DX11",
        "Gl" => "OpenGL",
        other => other,
    };
    format!("WGPU({backend_name})")
}

fn transparent_backend_options(transparent_viewport: bool) -> wgpu::BackendOptions {
    #[cfg(target_os = "windows")]
    {
        let mut options = wgpu::BackendOptions::default();
        if transparent_viewport {
            options.dx12.presentation_system = wgpu::wgt::Dx12SwapchainKind::DxgiFromVisual;
        }
        return options;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = transparent_viewport;
        wgpu::BackendOptions::default()
    }
}
