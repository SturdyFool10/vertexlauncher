use super::*;

#[inline]
pub(crate) fn texture_options_for_sampling(sampling: TextAtlasSampling) -> TextureOptions {
    match sampling {
        TextAtlasSampling::Linear => TextureOptions::LINEAR,
        TextAtlasSampling::Nearest => TextureOptions::NEAREST,
    }
}

#[inline]
pub(crate) fn glyph_content_mode_from_rasterization(mode: TextGlyphRasterMode) -> GlyphContentMode {
    match mode {
        TextGlyphRasterMode::Auto => GlyphContentMode::AlphaMask,
        TextGlyphRasterMode::AlphaMask => GlyphContentMode::AlphaMask,
        TextGlyphRasterMode::Sdf => GlyphContentMode::Sdf,
        TextGlyphRasterMode::Msdf => GlyphContentMode::Msdf,
    }
}

pub(crate) fn egui_key_from_text(key: TextKey) -> Key {
    match key {
        TextKey::A => Key::A,
        TextKey::B => Key::B,
        TextKey::Backspace => Key::Backspace,
        TextKey::Delete => Key::Delete,
        TextKey::Down => Key::ArrowDown,
        TextKey::E => Key::E,
        TextKey::End => Key::End,
        TextKey::Enter => Key::Enter,
        TextKey::Escape => Key::Escape,
        TextKey::F => Key::F,
        TextKey::H => Key::H,
        TextKey::Home => Key::Home,
        TextKey::K => Key::K,
        TextKey::Left => Key::ArrowLeft,
        TextKey::N => Key::N,
        TextKey::P => Key::P,
        TextKey::PageDown => Key::PageDown,
        TextKey::PageUp => Key::PageUp,
        TextKey::Right => Key::ArrowRight,
        TextKey::Tab => Key::Tab,
        TextKey::U => Key::U,
        TextKey::Up => Key::ArrowUp,
        TextKey::W => Key::W,
        TextKey::Y => Key::Y,
        TextKey::Z => Key::Z,
    }
}

pub(crate) fn egui_modifiers_from_text(modifiers: TextModifiers) -> egui::Modifiers {
    egui::Modifiers {
        alt: modifiers.alt,
        ctrl: modifiers.ctrl,
        shift: modifiers.shift,
        mac_cmd: modifiers.mac_cmd,
        command: modifiers.command,
    }
}

pub(crate) fn core_label_options(options: &TextLabelOptions) -> LabelOptions {
    LabelOptions {
        font_size: options.font_size,
        line_height: options.line_height,
        color: options.color.into(),
        wrap: options.wrap,
        monospace: options.monospace,
        weight: options.weight,
        italic: options.italic,
        padding: options.padding.into(),
        fundamentals: options.fundamentals.clone(),
        ellipsis: options.ellipsis.clone(),
    }
}

#[inline]
pub(crate) fn wgpu_filter_mode_for_sampling(sampling: TextAtlasSampling) -> wgpu::FilterMode {
    match sampling {
        TextAtlasSampling::Linear => wgpu::FilterMode::Linear,
        TextAtlasSampling::Nearest => wgpu::FilterMode::Nearest,
    }
}

pub fn wgpu_backends_for_text_graphics_api(api: TextGraphicsApi) -> wgpu::Backends {
    match api {
        TextGraphicsApi::Auto => wgpu::Backends::PRIMARY,
        TextGraphicsApi::Vulkan => wgpu::Backends::VULKAN,
        TextGraphicsApi::Metal => wgpu::Backends::METAL,
        TextGraphicsApi::Dx12 => wgpu::Backends::DX12,
        TextGraphicsApi::Gl => wgpu::Backends::GL,
    }
}

pub fn wgpu_power_preference_for_text_gpu_preference(
    preference: TextGpuPowerPreference,
) -> wgpu::PowerPreference {
    match preference {
        TextGpuPowerPreference::Auto => wgpu::PowerPreference::default(),
        TextGpuPowerPreference::LowPower => wgpu::PowerPreference::LowPower,
        TextGpuPowerPreference::HighPerformance => wgpu::PowerPreference::HighPerformance,
    }
}

pub(crate) fn multiply_color32(a: Color32, b: Color32) -> Color32 {
    Color32::from_rgba_premultiplied(
        ((u16::from(a.r()) * u16::from(b.r())) / 255) as u8,
        ((u16::from(a.g()) * u16::from(b.g())) / 255) as u8,
        ((u16::from(a.b()) * u16::from(b.b())) / 255) as u8,
        ((u16::from(a.a()) * u16::from(b.a())) / 255) as u8,
    )
}

pub(crate) fn to_cosmic_color(color: Color32) -> Color {
    Color::rgba(color.r(), color.g(), color.b(), color.a())
}

pub(crate) fn to_cosmic_text_color(color: TextColor) -> Color {
    to_cosmic_color(color.into())
}

pub(crate) fn cosmic_to_egui_color(color: Color) -> Color32 {
    Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), color.a())
}
