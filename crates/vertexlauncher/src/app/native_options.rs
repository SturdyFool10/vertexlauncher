use config::Config;
use eframe::{self, egui};
use launcher_ui::window_effects;
use std::sync::Arc;

use super::{app_icon, app_metadata, platform};

#[cfg(target_os = "linux")]
const LINUX_APP_ID: &str = "io.github.SturdyFool10.VertexLauncher";
#[cfg(not(target_os = "linux"))]
const LINUX_APP_ID: &str = "vertexlauncher";

pub fn build(startup_config: &Config) -> eframe::NativeOptions {
    let startup_power_preference = if startup_config.low_power_gpu_preferred() {
        eframe::egui_wgpu::wgpu::PowerPreference::LowPower
    } else {
        eframe::egui_wgpu::wgpu::PowerPreference::HighPerformance
    };
    let blur_enabled =
        startup_config.window_blur_enabled() && window_effects::platform_supports_blur();
    let transparent_viewport = blur_enabled;
    let startup_graphics = platform::startup_graphics_config(transparent_viewport);
    let renderer = startup_graphics.renderer;
    let hardware_acceleration = startup_graphics.hardware_acceleration;

    platform::log_startup_graphics_choice(startup_graphics);

    eframe::NativeOptions {
        viewport: egui::ViewportBuilder {
            title: Some("Vertex Launcher".into()),
            app_id: Some(LINUX_APP_ID.into()),
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
        vsync: false,
        multisampling: 4,
        depth_buffer: 32,
        stencil_buffer: 0,
        dithering: false,
        centered: false,
        persist_window: false,
        event_loop_builder: None,
        window_builder: None,
        shader_version: None,
        run_and_return: false,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            present_mode: eframe::egui_wgpu::wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: Some(1),
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
                eframe::egui_wgpu::WgpuSetupCreateNew {
                    instance_descriptor: eframe::egui_wgpu::wgpu::InstanceDescriptor {
                        backends: startup_graphics.backends,
                        backend_options: transparent_backend_options(transparent_viewport),
                        ..Default::default()
                    },
                    power_preference: startup_power_preference,
                    native_adapter_selector: None,
                    device_descriptor: Arc::new(|adapter| {
                        let info = adapter.get_info();
                        app_metadata::record_graphics_adapter(
                            &info.name,
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

                        let base_limits = if info.backend == eframe::egui_wgpu::wgpu::Backend::Gl {
                            eframe::egui_wgpu::wgpu::Limits::downlevel_webgl2_defaults()
                        } else {
                            eframe::egui_wgpu::wgpu::Limits::default()
                        };
                        let adapter_limits = adapter.limits();

                        eframe::egui_wgpu::wgpu::DeviceDescriptor {
                            label: Some("egui wgpu device"),
                            required_limits: base_limits.using_resolution(adapter_limits),
                            ..Default::default()
                        }
                    }),
                },
            ),
            on_surface_error: std::sync::Arc::new(|_| {
                eframe::egui_wgpu::SurfaceErrorAction::RecreateSurface
            }),
        },
        ..Default::default()
    }
}

fn transparent_backend_options(
    transparent_viewport: bool,
) -> eframe::egui_wgpu::wgpu::BackendOptions {
    #[cfg(target_os = "windows")]
    {
        let mut options = eframe::egui_wgpu::wgpu::BackendOptions::default();
        if transparent_viewport {
            options.dx12.presentation_system =
                eframe::egui_wgpu::wgpu::wgt::Dx12SwapchainKind::DxgiFromVisual;
        }
        return options;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = transparent_viewport;
        eframe::egui_wgpu::wgpu::BackendOptions::default()
    }
}
