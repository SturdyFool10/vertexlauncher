use super::*;

pub(super) fn sleep_precise(duration: Duration) {
    let coarse = Duration::from_millis(1);
    let tail = Duration::from_micros(250);
    if duration > coarse + tail {
        std::thread::sleep(duration - tail);
    }
    let deadline = Instant::now() + tail.min(duration);
    while Instant::now() < deadline {
        std::hint::spin_loop();
        std::thread::yield_now();
    }
}

pub(super) fn build_text_graphics_config(
    config: &Config,
    startup_graphics: platform::StartupGraphicsConfig,
) -> textui::TextGraphicsConfig {
    let mut graphics_config = textui::TextGraphicsConfig {
        renderer_backend: textui::TextRendererBackend::Auto,
        graphics_api: text_graphics_api_for_startup_config(startup_graphics),
        gpu_power_preference: text_gpu_power_preference_for_config(config),
        ..textui::TextGraphicsConfig::default()
    };
    graphics_config.rasterization.glyph_raster_mode = match config.text_rendering_path() {
        config::TextRenderingPath::Auto => textui::TextGlyphRasterMode::Auto,
        config::TextRenderingPath::AlphaMask => textui::TextGlyphRasterMode::AlphaMask,
        config::TextRenderingPath::Sdf => textui::TextGlyphRasterMode::Sdf,
        config::TextRenderingPath::Msdf => textui::TextGlyphRasterMode::Msdf,
    };

    graphics_config
}

fn text_graphics_api_for_startup_config(
    startup_graphics: platform::StartupGraphicsConfig,
) -> textui::TextGraphicsApi {
    match startup_graphics.graphics_api_preference {
        config::GraphicsApiPreference::Vulkan => textui::TextGraphicsApi::Vulkan,
        config::GraphicsApiPreference::Metal => textui::TextGraphicsApi::Metal,
        config::GraphicsApiPreference::Dx12 => textui::TextGraphicsApi::Dx12,
        config::GraphicsApiPreference::Auto => {
            preferred_text_graphics_api(startup_graphics.backends)
        }
    }
}

fn text_gpu_power_preference_for_config(config: &Config) -> textui::TextGpuPowerPreference {
    match config.graphics_adapter_preference_type() {
        config::GraphicsAdapterPreferenceType::PerformanceProfile => {
            match config.graphics_adapter_profile() {
                config::GraphicsAdapterProfile::Default => textui::TextGpuPowerPreference::Auto,
                config::GraphicsAdapterProfile::HighPerformance => {
                    textui::TextGpuPowerPreference::HighPerformance
                }
                config::GraphicsAdapterProfile::LowPower => {
                    textui::TextGpuPowerPreference::LowPower
                }
                config::GraphicsAdapterProfile::DiscreteOnly => {
                    textui::TextGpuPowerPreference::HighPerformance
                }
                config::GraphicsAdapterProfile::IntegratedOnly => {
                    textui::TextGpuPowerPreference::LowPower
                }
            }
        }
        config::GraphicsAdapterPreferenceType::ExplicitAdapter => {
            textui::TextGpuPowerPreference::Auto
        }
    }
}

pub(super) fn preferred_text_graphics_api(backends: wgpu::Backends) -> textui::TextGraphicsApi {
    if cfg!(target_os = "macos") && backends.contains(wgpu::Backends::METAL) {
        return textui::TextGraphicsApi::Metal;
    }
    if cfg!(target_os = "windows") && backends.contains(wgpu::Backends::DX12) {
        return textui::TextGraphicsApi::Dx12;
    }
    if backends.contains(wgpu::Backends::VULKAN) {
        return textui::TextGraphicsApi::Vulkan;
    }
    if backends.contains(wgpu::Backends::METAL) {
        return textui::TextGraphicsApi::Metal;
    }
    if backends.contains(wgpu::Backends::DX12) {
        return textui::TextGraphicsApi::Dx12;
    }
    if backends.contains(wgpu::Backends::GL) {
        return textui::TextGraphicsApi::Gl;
    }
    textui::TextGraphicsApi::Auto
}

pub(super) fn effective_window_blur_enabled(config: &Config) -> bool {
    if !config.window_blur_enabled() || !window_effects::platform_supports_blur() {
        return false;
    }

    #[cfg(target_os = "windows")]
    {
        return true;
    }

    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

pub(super) fn transparent_viewport_enabled(config: &Config) -> bool {
    effective_window_blur_enabled(config)
}

pub(super) fn effective_ui_opacity_percent(config: &Config) -> u8 {
    if effective_window_blur_enabled(config) {
        config.ui_opacity_percent()
    } else {
        100
    }
}

pub(super) fn disable_window_blur_for_startup(
    cc: &eframe::CreationContext<'_>,
    config: &mut Config,
    config_loaded_from_disk: bool,
    message: String,
    save_context: &'static str,
) {
    if !config.window_blur_enabled() {
        return;
    }

    config.set_window_blur_enabled(false);
    cc.egui_ctx
        .send_viewport_cmd(egui::ViewportCommand::Transparent(
            transparent_viewport_enabled(config),
        ));
    notification::warn!("window_blur", "{message}");

    if !config_loaded_from_disk {
        return;
    }

    let config_to_save = config.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let result = save_config(&config_to_save).map_err(|err| err.to_string());
        if let Err(save_error) = result {
            notification::warn!(
                "config",
                "Failed to persist disabled blur setting after {save_context}: {save_error}"
            );
        }
    });
}
