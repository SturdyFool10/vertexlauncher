use config::{
    Config, DOWNLOAD_CONCURRENCY_MAX, DOWNLOAD_CONCURRENCY_MIN, DropdownSettingId, FloatSettingId,
    GraphicsAdapterPreferenceType, GraphicsAdapterProfile, GraphicsApiPreference,
    INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP, IntSettingId,
    JavaRuntimeVersion, SkinPreviewAaMode, SkinPreviewTexelAaMode, SvgAaMode, TextRenderingPath,
    UiEmojiFontFamily, UiFontFamily, parse_bitrate_to_bps,
};
use egui::Ui;
use installation::{ensure_openjdk_runtime_async, purge_cache_async as purge_installation_cache};
use launcher_runtime as tokio_runtime;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;

struct PurgeCacheState {
    rx: Option<mpsc::Receiver<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JavaRuntimeActionKind {
    Download,
    Validate,
}

struct JavaRuntimeActionResult {
    kind: JavaRuntimeActionKind,
    outcome: Result<JavaRuntimeActionOutcome, String>,
}

enum JavaRuntimeActionOutcome {
    Downloaded(PathBuf),
    Validated(String),
}

#[derive(Default)]
struct JavaRuntimeActionState {
    in_flight: Option<JavaRuntimeActionKind>,
    rx: Option<mpsc::Receiver<JavaRuntimeActionResult>>,
    message: Option<String>,
}
use textui::TextUi;
use textui_egui::{gamepad_scroll, prelude::*};

use super::{SettingsInfo, platform};
use crate::{
    ui::{components::settings_widgets, style, theme::Theme},
    window_effects,
};

const RESERVED_SYSTEM_MEMORY_MIB: u128 = 4 * 1024;
const FALLBACK_TOTAL_MEMORY_MIB: u128 = 20 * 1024;
const FORCE_THEME_FOCUS_ID: &str = "settings_force_theme_focus";

#[path = "settings/memory_slider_max_state.rs"]
mod memory_slider_max_state;

use self::memory_slider_max_state::MemorySliderMaxState;

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    available_ui_fonts: &[UiFontFamily],
    available_ui_font_labels: &[String],
    available_emoji_fonts: &[UiEmojiFontFamily],
    available_emoji_font_labels: &[String],
    available_themes: &[Theme],
    available_theme_labels: &[String],
    settings_info: &SettingsInfo,
) {
    gamepad_scroll(
        egui::ScrollArea::vertical()
            .id_salt("settings_page_scroll")
            .auto_shrink([false, false]),
        ui,
        |ui| {
            render_settings_contents(
                ui,
                text_ui,
                config,
                available_ui_fonts,
                available_ui_font_labels,
                available_emoji_fonts,
                available_emoji_font_labels,
                available_themes,
                available_theme_labels,
                settings_info,
            );
        },
    );
}

pub fn prewarm(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    config: &Config,
    available_ui_fonts: &[UiFontFamily],
    available_ui_font_labels: &[String],
    available_emoji_fonts: &[UiEmojiFontFamily],
    available_emoji_font_labels: &[String],
    available_themes: &[Theme],
    available_theme_labels: &[String],
    settings_info: &SettingsInfo,
) {
    let mut prewarm_config = config.clone();
    let width = ctx.content_rect().width().max(960.0);
    egui::Area::new(egui::Id::new("settings_screen_prewarm"))
        .fixed_pos(egui::pos2(-20_000.0, -20_000.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.set_min_width(width);
            render_settings_contents(
                ui,
                text_ui,
                &mut prewarm_config,
                available_ui_fonts,
                available_ui_font_labels,
                available_emoji_fonts,
                available_emoji_font_labels,
                available_themes,
                available_theme_labels,
                settings_info,
            );
        });
}

pub fn request_theme_focus(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(FORCE_THEME_FOCUS_ID), true));
    settings_widgets::request_default_focus(ctx, true);
}

fn render_settings_contents(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    available_ui_fonts: &[UiFontFamily],
    available_ui_font_labels: &[String],
    available_emoji_fonts: &[UiEmojiFontFamily],
    available_emoji_font_labels: &[String],
    available_themes: &[Theme],
    available_theme_labels: &[String],
    settings_info: &SettingsInfo,
) {
    render_settings_section(
        ui,
        text_ui,
        "Appearance & Privacy",
        "Theme, launcher chrome, and on-stream safety settings.",
        |ui, text_ui| {
            render_theme_setting(
                ui,
                text_ui,
                config,
                available_themes,
                available_theme_labels,
            );
            render_skin_preview_setting(ui, text_ui, config);
            render_svg_aa_setting(ui, text_ui, config);
            render_window_blur_setting(ui, text_ui, config);
            render_ui_opacity_setting(ui, text_ui, config);
            render_selected_toggles(
                ui,
                text_ui,
                config,
                &[
                    config::ToggleSettingId::StreamerModeEnabled,
                    config::ToggleSettingId::NotificationExpiryBarsEmptyLeft,
                    config::ToggleSettingId::SkinPreviewFreshFormatEnabled,
                    config::ToggleSettingId::SkinPreview3dLayersEnabled,
                ],
            );
        },
    );

    render_settings_section(
        ui,
        text_ui,
        "Text",
        "Font, shaping, and glyph rendering settings that affect launcher text.",
        |ui, text_ui| {
            render_ui_font_settings(
                ui,
                text_ui,
                config,
                available_ui_fonts,
                available_ui_font_labels,
            );
            render_emoji_font_settings(
                ui,
                text_ui,
                config,
                available_emoji_fonts,
                available_emoji_font_labels,
            );
            render_text_rendering_path_setting(ui, text_ui, config);
            render_selected_toggles(
                ui,
                text_ui,
                config,
                &[config::ToggleSettingId::OpenTypeFeaturesEnabled],
            );
            render_selected_text_settings(
                ui,
                text_ui,
                config,
                &[config::TextSettingId::OpenTypeFeaturesToEnable],
            );
        },
    );

    render_settings_section(
        ui,
        text_ui,
        "Social & Presence",
        "Discord integration and launcher-owned activity reporting.",
        |ui, text_ui| {
            render_selected_toggles(
                ui,
                text_ui,
                config,
                &[config::ToggleSettingId::DiscordRichPresenceEnabled],
            );
        },
    );

    render_settings_section(
        ui,
        text_ui,
        "Graphics & Performance",
        "GPU, skin preview rendering, frame pacing, and download throughput behavior.",
        |ui, text_ui| {
            render_skin_preview_motion_blur_settings(ui, text_ui, config);
            render_graphics_adapter_settings(ui, text_ui, config, settings_info);
            render_graphics_api_setting(ui, text_ui, config);
            render_selected_toggles(
                ui,
                text_ui,
                config,
                &[config::ToggleSettingId::FrameLimiterEnabled],
            );
            render_download_settings(ui, text_ui, config);
        },
    );

    render_settings_section(
        ui,
        text_ui,
        "Minecraft & Java",
        "Version catalog behavior and shared runtime configuration.",
        |ui, text_ui| {
            render_settings_subgroup(
                ui,
                text_ui,
                "Minecraft Version Groups",
                "Controls which non-default Minecraft version categories appear in version pickers.",
            );
            render_selected_toggles(
                ui,
                text_ui,
                config,
                &[
                    config::ToggleSettingId::SnapshotsAndBetasEnabled,
                    config::ToggleSettingId::AlphaVersionsEnabled,
                    config::ToggleSettingId::ExperimentalVersionsEnabled,
                ],
            );
            render_settings_subgroup(
                ui,
                text_ui,
                "Java Runtime",
                "Shared Java behavior for launches that use managed runtime selection.",
            );
            render_selected_toggles(
                ui,
                text_ui,
                config,
                &[config::ToggleSettingId::ForceJava21Minimum],
            );
            render_java_runtime_settings(ui, text_ui, config);
            render_curseforge_settings(ui, text_ui, config);
        },
    );

    render_instance_defaults_section(ui, text_ui, config);
    render_platform_specific_settings_section(ui, text_ui, config);
    render_info_section(ui, text_ui, settings_info);
}

fn render_info_section(ui: &mut Ui, text_ui: &mut TextUi, settings_info: &SettingsInfo) {
    render_settings_section(
        ui,
        text_ui,
        "Info",
        "Runtime and hardware information for this installation.",
        |ui, text_ui| {
            let mut key_style = style::muted(ui);
            key_style.weight = 700;
            key_style.wrap = false;

            let mut value_style = style::body(ui);
            value_style.monospace = true;

            egui::Grid::new("settings_info_grid")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    render_info_row(
                        text_ui,
                        ui,
                        "CPU",
                        &settings_info.cpu,
                        &key_style,
                        &value_style,
                    );
                    render_info_row(
                        text_ui,
                        ui,
                        "GPU",
                        &settings_info.gpu,
                        &key_style,
                        &value_style,
                    );
                    render_info_row(
                        text_ui,
                        ui,
                        "Memory",
                        &settings_info.memory,
                        &key_style,
                        &value_style,
                    );
                    render_info_row(
                        text_ui,
                        ui,
                        "Graphics API",
                        &settings_info.graphics_api,
                        &key_style,
                        &value_style,
                    );
                    render_info_row(
                        text_ui,
                        ui,
                        "Graphics Driver + Version",
                        &settings_info.graphics_driver,
                        &key_style,
                        &value_style,
                    );
                    render_info_row(
                        text_ui,
                        ui,
                        "App Version + Commit Hash",
                        &settings_info.app_version,
                        &key_style,
                        &value_style,
                    );
                });
        },
    );
}

fn render_platform_specific_settings_section(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
) {
    let Some(section) = platform::current_platform_specific_section() else {
        return;
    };

    render_settings_section(
        ui,
        text_ui,
        section.heading,
        section.launcher_description,
        |ui, text_ui| platform::render_launcher_platform_settings(ui, text_ui, config),
    );
}

fn render_info_row(
    text_ui: &mut TextUi,
    ui: &mut Ui,
    label: &str,
    value: &str,
    key_style: &LabelOptions,
    value_style: &LabelOptions,
) {
    let _ = text_ui.label(ui, ("settings_info_label", label), label, key_style);
    let _ = text_ui.label(ui, ("settings_info_value", label), value, value_style);
    ui.end_row();
}

fn render_settings_section(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    heading: &str,
    description: &str,
    render_body: impl FnOnce(&mut Ui, &mut TextUi),
) {
    let heading_style = style::section_heading(ui);
    let mut body_style = style::muted(ui);
    body_style.wrap = true;

    ui.add_space(style::SPACE_XL);
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_MD))
        .inner_margin(egui::Margin::same(style::SPACE_XL as i8))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = style::SPACE_MD;
            let _ = text_ui.label(ui, ("settings_heading", heading), heading, &heading_style);
            let _ = text_ui.label(
                ui,
                ("settings_description", heading),
                description,
                &body_style,
            );
            ui.add_space(style::SPACE_MD);
            render_body(ui, text_ui);
        });
}

fn render_settings_subgroup(ui: &mut Ui, text_ui: &mut TextUi, heading: &str, description: &str) {
    let mut heading_style = style::body(ui);
    heading_style.weight = 700;
    heading_style.wrap = false;

    let mut description_style = style::muted(ui);
    description_style.wrap = true;

    let _ = text_ui.label(
        ui,
        ("settings_subgroup_heading", heading),
        heading,
        &heading_style,
    );
    let _ = text_ui.label(
        ui,
        ("settings_subgroup_description", heading),
        description,
        &description_style,
    );
    ui.add_space(style::SPACE_MD);
}

fn render_theme_setting(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    available_themes: &[Theme],
    available_theme_labels: &[String],
) {
    if available_themes.is_empty() {
        return;
    }
    let mut selected_theme_index = available_themes
        .iter()
        .position(|theme| theme.id == config.theme_id())
        .unwrap_or(0);
    let theme_labels: Vec<&str> = available_theme_labels.iter().map(String::as_str).collect();
    let theme_tooltip = format!(
        "Themes are loaded from {} at startup.",
        app_paths::themes_dir().display()
    );
    let response = settings_widgets::dropdown_row(
        text_ui,
        ui,
        "theme_selector",
        "Theme",
        Some(theme_tooltip.as_str()),
        &mut selected_theme_index,
        &theme_labels,
    );
    let should_force_focus = ui.ctx().data_mut(|data| {
        data.get_temp::<bool>(egui::Id::new(FORCE_THEME_FOCUS_ID))
            .unwrap_or(false)
    });
    if should_force_focus || ui.ctx().memory(|memory| memory.focused().is_none()) {
        response.request_focus();
        ui.ctx()
            .data_mut(|data| data.remove::<bool>(egui::Id::new(FORCE_THEME_FOCUS_ID)));
    }
    if response.changed() {
        if let Some(theme) = available_themes.get(selected_theme_index) {
            config.set_theme_id(theme.id.clone());
        }
    }
    ui.add_space(style::SPACE_MD);
}

fn render_ui_font_settings(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    available_ui_fonts: &[UiFontFamily],
    available_ui_font_labels: &[String],
) {
    config.for_each_dropdown_mut(|setting, value| {
        if setting.id != DropdownSettingId::UiFontFamily {
            return;
        }
        ui.push_id(setting.id, |ui| {
            let options: &[UiFontFamily] = available_ui_fonts;
            if options.is_empty() {
                return;
            }
            let selected_option_index = options
                .iter()
                .position(|option| option.matches(value))
                .unwrap_or(0);
            if options[selected_option_index] != *value {
                *value = options[selected_option_index].clone();
            }
            let option_label_refs: Vec<&str> = available_ui_font_labels
                .iter()
                .map(String::as_str)
                .collect();
            let mut selected_index = selected_option_index;
            let response = settings_widgets::searchable_dropdown_row(
                text_ui,
                ui,
                setting.id,
                setting.label,
                setting.info_tooltip,
                &mut selected_index,
                &option_label_refs,
            );
            if response.changed() {
                if let Some(next_value) = options.get(selected_index) {
                    *value = next_value.clone();
                }
            }
        });
        ui.add_space(style::SPACE_MD);
    });

    config.for_each_float_mut(|setting, value| {
        if matches!(
            setting.id,
            FloatSettingId::SkinPreviewMotionBlurAmount
                | FloatSettingId::SkinPreviewMotionBlurShutterFrames
        ) {
            return;
        }
        ui.push_id(setting.id, |ui| {
            settings_widgets::float_stepper_row(
                text_ui,
                ui,
                setting.id,
                setting.label,
                setting.info_tooltip,
                value,
                setting.min,
                setting.max,
                setting.step,
            );
        });
        ui.add_space(style::SPACE_MD);
    });

    render_selected_int_settings(ui, text_ui, config, &[IntSettingId::UiFontWeight]);
}

fn render_graphics_adapter_settings(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    settings_info: &SettingsInfo,
) {
    let type_setting = DropdownSettingId::GraphicsAdapterPreferenceType.spec();
    let type_options = GraphicsAdapterPreferenceType::ALL
        .iter()
        .map(|option| option.settings_label())
        .collect::<Vec<_>>();
    let mut selected_type_index = GraphicsAdapterPreferenceType::ALL
        .iter()
        .position(|option| *option == config.graphics_adapter_preference_type())
        .unwrap_or(0);
    let type_response = settings_widgets::dropdown_row(
        text_ui,
        ui,
        "graphics_adapter_preference_type",
        type_setting.label,
        type_setting.info_tooltip,
        &mut selected_type_index,
        &type_options,
    );
    if type_response.changed() {
        if let Some(next_type) = GraphicsAdapterPreferenceType::ALL.get(selected_type_index) {
            config.set_graphics_adapter_preference_type(*next_type);
        }
    }
    ui.add_space(style::SPACE_MD);

    let preference_setting = DropdownSettingId::GraphicsAdapterPreference.spec();
    match config.graphics_adapter_preference_type() {
        GraphicsAdapterPreferenceType::PerformanceProfile => {
            let profile_options = GraphicsAdapterProfile::ALL
                .iter()
                .map(|option| option.settings_label())
                .collect::<Vec<_>>();
            let mut selected_profile_index = GraphicsAdapterProfile::ALL
                .iter()
                .position(|option| *option == config.graphics_adapter_profile())
                .unwrap_or(0);
            let response = settings_widgets::dropdown_row(
                text_ui,
                ui,
                "graphics_adapter_profile",
                preference_setting.label,
                preference_setting.info_tooltip,
                &mut selected_profile_index,
                &profile_options,
            );
            if response.changed() {
                if let Some(next_profile) = GraphicsAdapterProfile::ALL.get(selected_profile_index)
                {
                    config.set_graphics_adapter_profile(*next_profile);
                }
            }
        }
        GraphicsAdapterPreferenceType::ExplicitAdapter => {
            let mut labels = settings_info
                .available_graphics_adapters
                .iter()
                .map(|adapter| adapter.label.as_str())
                .collect::<Vec<_>>();
            let explicit_hash = config.graphics_adapter_explicit_hash();
            let mut missing_label = None::<String>;
            let mut selected_index = settings_info
                .available_graphics_adapters
                .iter()
                .position(|adapter| Some(adapter.hash) == explicit_hash)
                .unwrap_or_else(|| {
                    if let Some(hash) = explicit_hash {
                        let _ = hash;
                        missing_label =
                            Some("Missing adapter (falls back to High Performance)".to_owned());
                        labels.insert(0, missing_label.as_ref().unwrap().as_str());
                        0
                    } else {
                        0
                    }
                });

            if labels.is_empty() {
                labels.push("No graphics adapters detected");
                selected_index = 0;
            }

            let response = settings_widgets::dropdown_row(
                text_ui,
                ui,
                "graphics_adapter_explicit",
                preference_setting.label,
                preference_setting.info_tooltip,
                &mut selected_index,
                &labels,
            );
            if response.changed() {
                if missing_label.is_some() && selected_index == 0 {
                    config.set_graphics_adapter_explicit_hash(explicit_hash);
                } else {
                    let actual_index =
                        selected_index.saturating_sub(usize::from(missing_label.is_some()));
                    let selected_hash = settings_info
                        .available_graphics_adapters
                        .get(actual_index)
                        .map(|adapter| adapter.hash);
                    config.set_graphics_adapter_explicit_hash(selected_hash);
                }
            }
        }
    }
    ui.add_space(style::SPACE_MD);
}

fn supported_graphics_api_preferences(config: &Config) -> Vec<GraphicsApiPreference> {
    let transparent_viewport =
        config.window_blur_enabled() && window_effects::platform_supports_blur();

    GraphicsApiPreference::ALL
        .into_iter()
        .filter(|preference| match preference {
            GraphicsApiPreference::Auto => true,
            GraphicsApiPreference::Metal => cfg!(target_os = "macos"),
            GraphicsApiPreference::Dx12 => cfg!(target_os = "windows"),
            GraphicsApiPreference::Vulkan => {
                if cfg!(target_os = "windows") {
                    !transparent_viewport
                } else {
                    !cfg!(target_os = "macos")
                }
            }
        })
        .collect()
}

fn render_graphics_api_setting(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    let setting = DropdownSettingId::GraphicsApiPreference.spec();
    let options = supported_graphics_api_preferences(config);
    let labels = options
        .iter()
        .map(|option| option.settings_label())
        .collect::<Vec<_>>();
    let mut selected_index = options
        .iter()
        .position(|option| *option == config.graphics_api_preference())
        .unwrap_or(0);
    let response = settings_widgets::dropdown_row(
        text_ui,
        ui,
        "graphics_api_preference",
        setting.label,
        setting.info_tooltip,
        &mut selected_index,
        &labels,
    );
    if response.changed() {
        if let Some(next_preference) = options.get(selected_index) {
            config.set_graphics_api_preference(*next_preference);
        }
    }
    ui.add_space(style::SPACE_MD);
}

fn render_emoji_font_settings(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    available_emoji_fonts: &[UiEmojiFontFamily],
    available_emoji_font_labels: &[String],
) {
    if available_emoji_fonts.is_empty() {
        return;
    }
    let current = config.ui_emoji_font_family();
    let selected_option_index = available_emoji_fonts
        .iter()
        .position(|option| option.matches(&current))
        .unwrap_or(0);
    let option_label_refs: Vec<&str> = available_emoji_font_labels
        .iter()
        .map(String::as_str)
        .collect();
    let mut selected_index = selected_option_index;
    let response = settings_widgets::searchable_dropdown_row(
        text_ui,
        ui,
        egui::Id::new("emoji_font_family_dropdown"),
        "Emoji Font",
        Some(
            "Font used for emoji characters. Noto Color Emoji is included and selected by default.",
        ),
        &mut selected_index,
        &option_label_refs,
    );
    if response.changed() {
        if let Some(next_value) = available_emoji_fonts.get(selected_index) {
            config.set_ui_emoji_font_family(next_value.clone());
        }
    }
    ui.add_space(style::SPACE_MD);
}

fn render_text_rendering_path_setting(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    let options = TextRenderingPath::ALL;
    let option_labels: Vec<&str> = options.iter().map(|option| option.label()).collect();
    let selected_index = options
        .iter()
        .position(|option| *option == config.text_rendering_path())
        .unwrap_or(0);
    let mut selected_index = selected_index;
    let response = settings_widgets::full_width_dropdown_row(
        text_ui,
        ui,
        "text_rendering_path",
        "Text Rendering Path",
        Some(
            "Controls how glyphs are rasterized for the launcher UI. Auto keeps the default path. SDF/MSDF are mainly useful for testing the new text pipeline.",
        ),
        &mut selected_index,
        &option_labels,
    );
    if response.changed() {
        if let Some(next) = options.get(selected_index).copied() {
            config.set_text_rendering_path(next);
        }
    }
    ui.add_space(style::SPACE_MD);
}

fn render_svg_aa_setting(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    let mut selected = SvgAaMode::ALL
        .iter()
        .position(|mode| *mode == config.svg_aa_mode())
        .unwrap_or(0);
    let labels: Vec<&str> = SvgAaMode::ALL.iter().map(|mode| mode.label()).collect();

    let response = settings_widgets::dropdown_row(
        text_ui,
        ui,
        "svg_aa_mode",
        "SVG Anti-Aliasing",
        Some(
            "Controls supersampled SVG rasterization for launcher icons. Changes apply immediately.",
        ),
        &mut selected,
        &labels,
    );

    if response.changed() {
        if let Some(next) = SvgAaMode::ALL.get(selected).copied() {
            config.set_svg_aa_mode(next);
        }
    }
}

fn render_skin_preview_setting(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    let mut texel_selected = SkinPreviewTexelAaMode::ALL
        .iter()
        .position(|mode| *mode == config.skin_preview_texel_aa_mode())
        .unwrap_or(0);
    let texel_labels: Vec<&str> = SkinPreviewTexelAaMode::ALL
        .iter()
        .map(|mode| mode.label())
        .collect();
    let texel_response = settings_widgets::dropdown_row(
        text_ui,
        ui,
        "skins_preview_texel_aa_mode",
        "Skin Preview Texel Edge AA",
        Some("Controls texel-boundary smoothing in the skin shader itself."),
        &mut texel_selected,
        &texel_labels,
    );
    if texel_response.changed() {
        if let Some(next) = SkinPreviewTexelAaMode::ALL.get(texel_selected).copied() {
            config.set_skin_preview_texel_aa_mode(next);
        }
    }

    ui.add_space(style::SPACE_MD);

    let mut selected = SkinPreviewAaMode::ALL
        .iter()
        .position(|mode| *mode == config.skin_preview_aa_mode())
        .unwrap_or(0);
    let labels: Vec<&str> = SkinPreviewAaMode::ALL
        .iter()
        .map(|mode| mode.label())
        .collect();
    let response = settings_widgets::dropdown_row(
        text_ui,
        ui,
        "skins_preview_aa_mode",
        "Skin Preview Post Anti-Aliasing",
        Some(
            "MSAA, SMAA, FXAA, TAA, and FXAA + TAA all run on the GPU after the scene is rendered. Changes apply immediately.",
        ),
        &mut selected,
        &labels,
    );
    if response.changed() {
        if let Some(next) = SkinPreviewAaMode::ALL.get(selected).copied() {
            config.set_skin_preview_aa_mode(next);
        }
    }

    ui.add_space(style::SPACE_MD);
}

fn render_skin_preview_motion_blur_settings(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
) {
    let mut enabled = config.skin_preview_motion_blur_enabled();
    let response = settings_widgets::toggle_row(
        text_ui,
        ui,
        "Enable Skin Preview Motion Blur",
        Some(
            "Uses multi-sample temporal shutter accumulation in the 3D skin preview. Applies immediately.",
        ),
        &mut enabled,
    );
    if response.changed() {
        config.set_skin_preview_motion_blur_enabled(enabled);
    }
    ui.add_space(style::SPACE_MD);

    if !config.skin_preview_motion_blur_enabled() {
        return;
    }

    let spec = FloatSettingId::SkinPreviewMotionBlurAmount.spec();
    let mut amount = config.skin_preview_motion_blur_amount();

    let response = settings_widgets::float_slider_row(
        text_ui,
        ui,
        spec.id,
        spec.label,
        Some(
            "Longer shutter intervals produce stronger blur but more trailing. Best paired with SMAA, FXAA, or TAA.",
        ),
        &mut amount,
        spec.min,
        spec.max,
        true,
    );

    if response.changed()
        || (amount - config.skin_preview_motion_blur_amount()).abs() > f32::EPSILON
    {
        config.set_skin_preview_motion_blur_amount(amount);
    }
    ui.add_space(style::SPACE_MD);

    let shutter_spec = FloatSettingId::SkinPreviewMotionBlurShutterFrames.spec();
    let mut shutter_frames = config.skin_preview_motion_blur_shutter_frames();
    let shutter_response = settings_widgets::float_stepper_row(
        text_ui,
        ui,
        shutter_spec.id,
        shutter_spec.label,
        Some(
            "Total shutter interval measured in 60 FPS frames. Higher values create longer streaks.",
        ),
        &mut shutter_frames,
        shutter_spec.min,
        shutter_spec.max,
        shutter_spec.step,
    );
    if shutter_response.changed()
        || (shutter_frames - config.skin_preview_motion_blur_shutter_frames()).abs() > f32::EPSILON
    {
        config.set_skin_preview_motion_blur_shutter_frames(shutter_frames);
    }
    ui.add_space(style::SPACE_MD);

    let sample_spec = IntSettingId::SkinPreviewMotionBlurSampleCount.spec();
    let mut sample_count = config.skin_preview_motion_blur_sample_count();
    let sample_response = settings_widgets::int_stepper_row(
        text_ui,
        ui,
        sample_spec.id,
        sample_spec.label,
        Some("Higher sample counts smooth the blur at the cost of more GPU work in the preview."),
        &mut sample_count,
        sample_spec.min,
        sample_spec.max,
        sample_spec.step,
    );
    if sample_response.changed() || sample_count != config.skin_preview_motion_blur_sample_count() {
        config.set_skin_preview_motion_blur_sample_count(sample_count);
    }
    ui.add_space(style::SPACE_MD);
}

fn render_window_blur_setting(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    let setting = config::ToggleSettingId::WindowBlurEnabled.spec();

    #[cfg(target_os = "macos")]
    {
        if config.window_blur_enabled() {
            config.set_window_blur_enabled(false);
        }

        let mut value = false;
        let _ = ui.add_enabled_ui(false, |ui| {
            settings_widgets::toggle_row(
                text_ui,
                ui,
                setting.label,
                setting.info_tooltip,
                &mut value,
            )
        });

        let note_style = style::muted(ui);
        let _ = text_ui.label(
            ui,
            "window_blur_macos_note",
            "Temporarily disabled on macOS to keep startup on the stable path.",
            &note_style,
        );
        ui.add_space(style::SPACE_MD);
        return;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut value = config.window_blur_enabled();
        let response = settings_widgets::toggle_row(
            text_ui,
            ui,
            setting.label,
            setting.info_tooltip,
            &mut value,
        );
        if response.changed() {
            config.set_window_blur_enabled(value);
        }
        ui.add_space(style::SPACE_MD);
    }
}

fn render_ui_opacity_setting(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    let opacity_active = ui_opacity_setting_active(config);
    let mut opacity_percent = config.ui_opacity_percent() as u128;
    let response = ui
        .add_enabled_ui(opacity_active, |ui| {
            settings_widgets::u128_slider_with_input_row(
                text_ui,
                ui,
                "ui_opacity_percent",
                "UI Opacity",
                Some(
                    "Only active while native window blur is supported and enabled. 100% is fully opaque and 0% is fully transparent.",
                ),
                &mut opacity_percent,
                0,
                100,
                1,
            )
        })
        .inner;
    if response.changed() {
        config.set_ui_opacity_percent(opacity_percent as u8);
    }
    ui.add_space(style::SPACE_MD);
}

fn ui_opacity_setting_active(config: &Config) -> bool {
    if !config.window_blur_enabled() || !crate::window_effects::platform_supports_blur() {
        return false;
    }

    #[cfg(target_os = "windows")]
    {
        true
    }

    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

fn render_selected_toggles(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    selected_ids: &[config::ToggleSettingId],
) {
    let mut frame_limit_fps = config.frame_limit_fps();
    config.for_each_toggle_mut(|setting, value| {
        if !selected_ids.contains(&setting.id) {
            return;
        }
        ui.push_id(setting.id, |ui| {
            settings_widgets::toggle_row(text_ui, ui, setting.label, setting.info_tooltip, value);
        });
        if setting.id == config::ToggleSettingId::FrameLimiterEnabled && *value {
            let spec = IntSettingId::FrameLimitFps.spec();
            ui.push_id(spec.id, |ui| {
                let _ = settings_widgets::int_stepper_row(
                    text_ui,
                    ui,
                    spec.id,
                    spec.label,
                    spec.info_tooltip,
                    &mut frame_limit_fps,
                    spec.min,
                    spec.max,
                    spec.step,
                );
            });
        }
        ui.add_space(style::SPACE_MD);
    });
    config.set_frame_limit_fps(frame_limit_fps);
}

fn render_selected_int_settings(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    selected_ids: &[IntSettingId],
) {
    config.for_each_int_mut(|setting, value| {
        if !selected_ids.contains(&setting.id) {
            return;
        }
        ui.push_id(setting.id, |ui| {
            settings_widgets::int_stepper_row(
                text_ui,
                ui,
                setting.id,
                setting.label,
                setting.info_tooltip,
                value,
                setting.min,
                setting.max,
                setting.step,
            );
        });
        ui.add_space(style::SPACE_MD);
    });
}

fn render_selected_text_settings(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    selected_ids: &[config::TextSettingId],
) {
    config.for_each_text_mut(|setting, value| {
        if !selected_ids.contains(&setting.id) {
            return;
        }
        ui.push_id(setting.id, |ui| {
            settings_widgets::text_input_row(
                text_ui,
                ui,
                setting.id,
                setting.label,
                setting.info_tooltip,
                value,
            );
        });
        ui.add_space(style::SPACE_MD);
    });
}

fn render_download_settings(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    let concurrency_spec = IntSettingId::FrameLimitFps; // placeholder to satisfy formatting block removal
    let _ = concurrency_spec;

    let mut max_concurrent = config.download_max_concurrent() as i32;
    let response = settings_widgets::int_stepper_row(
        text_ui,
        ui,
        "download_max_concurrent",
        "Max Concurrent Downloads",
        Some("Maximum number of parallel downloads used by the launcher."),
        &mut max_concurrent,
        DOWNLOAD_CONCURRENCY_MIN as i32,
        DOWNLOAD_CONCURRENCY_MAX as i32,
        1,
    );
    if response.changed() {
        config.set_download_max_concurrent(max_concurrent.max(1) as u32);
    }
    ui.add_space(style::SPACE_MD);

    let mut speed_limit_enabled = config.download_speed_limit_enabled();
    settings_widgets::toggle_row(
        text_ui,
        ui,
        "Enable Download Speed Limit",
        Some("Caps launcher download bandwidth using the value below."),
        &mut speed_limit_enabled,
    );
    config.set_download_speed_limit_enabled(speed_limit_enabled);
    ui.add_space(style::SPACE_MD);

    let _ = settings_widgets::text_input_row(
        text_ui,
        ui,
        "download_speed_limit",
        "Download Speed Limit",
        Some("Examples: 10mbps, 500kbps, 1gbps."),
        config.download_speed_limit_mut(),
    );
    ui.add_space(style::SPACE_MD);
}

fn render_java_runtime_settings(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    let heading_style = style::section_heading(ui);
    let mut body_style = style::muted(ui);
    body_style.wrap = false;

    let _ = text_ui.label(
        ui,
        "java_paths_heading",
        "Java JVM Paths (Optional)",
        &heading_style,
    );
    let _ = text_ui.label(
        ui,
        "java_paths_description",
        "Leave blank for None. Configure only versions you actually use.",
        &body_style,
    );
    ui.add_space(8.0);

    for runtime in JavaRuntimeVersion::ALL {
        render_java_runtime_path_row(ui, text_ui, config, runtime);
    }
}

fn render_curseforge_settings(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    let _ = settings_widgets::text_input_row(
        text_ui,
        ui,
        "curseforge_api_key",
        "CurseForge API Key",
        Some(
            "Required for CurseForge modpack zip imports. Overrides VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY and enables CurseForge browsing/download metadata.",
        ),
        config.curseforge_api_key_mut(),
    );
    ui.add_space(style::SPACE_MD);
}

fn render_instance_defaults_section(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    ui.add_space(style::SPACE_LG);
    ui.separator();
    ui.add_space(style::SPACE_LG);

    let heading_style = style::section_heading(ui);
    let mut body_style = style::muted(ui);
    body_style.wrap = false;

    let _ = text_ui.label(
        ui,
        "instance_defaults_heading",
        "Instance Defaults",
        &heading_style,
    );
    let _ = text_ui.label(
        ui,
        "instance_defaults_description",
        "Used when creating new instances. You can still override values per instance.",
        &body_style,
    );
    ui.add_space(style::SPACE_MD);

    let mut installations_root = config
        .minecraft_installations_root_path()
        .as_os_str()
        .to_string_lossy()
        .into_owned();
    let installations_root_response = settings_widgets::full_width_text_input_row(
        text_ui,
        ui,
        "instance_defaults_installations_root",
        "Minecraft installations folder",
        Some(
            "New instances are created under this folder as <folder>/<instance.minecraft_root>. Relative paths are resolved from the launcher working directory.",
        ),
        &mut installations_root,
    );
    if installations_root_response.changed() {
        config.set_minecraft_installations_root(installations_root);
    }
    ui.add_space(style::SPACE_MD);

    let cache_button_style = ButtonOptions {
        min_size: egui::vec2(220.0, style::CONTROL_HEIGHT_LG),
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..ButtonOptions::default()
    };
    static PURGE_CACHE_STATE: OnceLock<Mutex<PurgeCacheState>> = OnceLock::new();
    let purge_state = PURGE_CACHE_STATE.get_or_init(|| Mutex::new(PurgeCacheState { rx: None }));

    let cache_status_id = ui.make_persistent_id("settings_cache_status_message");
    let mut cache_status = ui.ctx().data_mut(|d| d.get_temp::<String>(cache_status_id));

    if let Ok(mut state) = purge_state.lock() {
        if let Some(rx) = state.rx.as_ref() {
            match rx.try_recv() {
                Ok(msg) => {
                    cache_status = Some(msg);
                    state.rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(50));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    state.rx = None;
                }
            }
        }
    }

    let purge_in_flight = purge_state.lock().is_ok_and(|state| state.rx.is_some());
    let purge_button_enabled = !purge_in_flight;
    let purge_button_label = if purge_in_flight {
        "Purging..."
    } else {
        "Purge metadata cache"
    };
    if ui
        .add_enabled_ui(purge_button_enabled, |ui| {
            text_ui.button(
                ui,
                "instance_defaults_purge_cache",
                purge_button_label,
                &cache_button_style,
            )
        })
        .inner
        .clicked()
    {
        let (tx, rx) = mpsc::channel::<String>();
        if let Ok(mut state) = purge_state.lock() {
            state.rx = Some(rx);
        }
        tokio_runtime::spawn_detached(async move {
            let msg = match purge_installation_cache().await {
                Ok(()) => {
                    "Purged local metadata cache. Version lists will be re-fetched on next refresh."
                        .to_owned()
                }
                Err(err) => format!("Failed to purge metadata cache: {err}"),
            };
            let _ = tx.send(msg);
        });
    }
    if let Some(message) = cache_status.as_deref() {
        let mut status_style = body_style.clone();
        status_style.wrap = true;
        let _ = text_ui.label(ui, "instance_defaults_cache_status", message, &status_style);
    }
    if let Some(message) = cache_status {
        ui.ctx()
            .data_mut(|d| d.insert_temp(cache_status_id, message));
    }
    ui.add_space(style::SPACE_MD);

    let mut max_concurrent_downloads = config.download_max_concurrent() as i32;
    let starts_response = settings_widgets::int_stepper_row(
        text_ui,
        ui,
        "download_max_concurrent",
        "Max concurrent downloads",
        Some("Global cap on simultaneous download jobs. Default: 8."),
        &mut max_concurrent_downloads,
        DOWNLOAD_CONCURRENCY_MIN as i32,
        DOWNLOAD_CONCURRENCY_MAX as i32,
        1,
    );
    if starts_response.changed() {
        config.set_download_max_concurrent(max_concurrent_downloads.max(1) as u32);
    }
    ui.add_space(style::SPACE_MD);

    let mut speed_limit_enabled = config.download_speed_limit_enabled();
    let speed_toggle_response = settings_widgets::toggle_row(
        text_ui,
        ui,
        "Enable download speed limiter",
        Some("When disabled, no bandwidth cap is applied."),
        &mut speed_limit_enabled,
    );
    if speed_toggle_response.changed() {
        config.set_download_speed_limit_enabled(speed_limit_enabled);
    }
    ui.add_space(style::SPACE_MD);

    let mut speed_limit = config.download_speed_limit().to_owned();
    let speed_response = settings_widgets::full_width_text_input_row(
        text_ui,
        ui,
        "download_speed_limit",
        "Download speed limit",
        Some("Format: <number><unit> where unit is Kbps, Mbps, Gbps, or Tbps (example: 250Mbps)."),
        &mut speed_limit,
    );
    if speed_response.changed() {
        *config.download_speed_limit_mut() = speed_limit;
    }
    if config.download_speed_limit_enabled() {
        let current_value = config.download_speed_limit().trim();
        let mut validation_style = body_style.clone();
        validation_style.wrap = true;
        if current_value.is_empty() {
            validation_style.color = ui.visuals().weak_text_color();
            let _ = text_ui.label(
                ui,
                "download_speed_limit_hint",
                "Speed limiter enabled, but no value set. Enter something like 250Mbps.",
                &validation_style,
            );
        } else if let Some(bps) = parse_bitrate_to_bps(current_value) {
            let _ = text_ui.label(
                ui,
                "download_speed_limit_ok",
                &format!("Speed limiter active at {bps} bps."),
                &validation_style,
            );
        } else {
            validation_style.color = ui.visuals().error_fg_color;
            let _ = text_ui.label(
                ui,
                "download_speed_limit_invalid",
                "Invalid speed format. Use Kbps, Mbps, Gbps, or Tbps.",
                &validation_style,
            );
        }
        ui.add_space(style::SPACE_MD);
    }

    let mut default_memory = config.default_instance_max_memory_mib();
    let (max_memory_mib, memory_slider_pending) = memory_slider_max_mib();
    if memory_slider_pending {
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }
    if default_memory > max_memory_mib {
        default_memory = max_memory_mib;
        config.set_default_instance_max_memory_mib(default_memory);
    }

    let memory_response = settings_widgets::u128_slider_with_input_row(
        text_ui,
        ui,
        "instance_defaults_memory_mib",
        "Default max memory allocation (MiB)",
        Some("Amount of RAM allocated by default to new instances."),
        &mut default_memory,
        INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN,
        max_memory_mib,
        INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP,
    );
    if memory_response.changed() {
        config.set_default_instance_max_memory_mib(default_memory);
    }
    ui.add_space(8.0);

    settings_widgets::full_width_text_input_row(
        text_ui,
        ui,
        "instance_defaults_cli_args",
        "Default CLI args",
        Some("Extra JVM arguments applied to new instances (for example: -XX:+UseG1GC)."),
        config.default_instance_cli_args_mut(),
    );
    ui.add_space(10.0);
}

fn render_java_runtime_path_row(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    runtime: JavaRuntimeVersion,
) {
    update_java_runtime_action_state(ui, config, runtime);

    let mut path_value = config
        .java_runtime_path_ref(runtime)
        .map(|path| path.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default();
    let response = settings_widgets::full_width_text_input_row(
        text_ui,
        ui,
        ("instance_java_path", runtime.major()),
        runtime.label(),
        Some(runtime.info_tooltip()),
        &mut path_value,
    );
    if response.changed() {
        config.set_java_runtime_path_ref(
            runtime,
            normalize_optional_input(&path_value)
                .as_deref()
                .map(Path::new),
        );
    }

    render_java_runtime_path_actions(ui, text_ui, config, runtime, path_value.as_str());
    ui.add_space(8.0);
}

fn java_runtime_action_states() -> &'static Mutex<HashMap<u8, JavaRuntimeActionState>> {
    static STATES: OnceLock<Mutex<HashMap<u8, JavaRuntimeActionState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn update_java_runtime_action_state(ui: &mut Ui, config: &mut Config, runtime: JavaRuntimeVersion) {
    let mut completed = None;
    if let Ok(mut states) = java_runtime_action_states().lock() {
        let state = states.entry(runtime.major()).or_default();
        if let Some(rx) = state.rx.as_ref() {
            match rx.try_recv() {
                Ok(result) => {
                    state.in_flight = None;
                    state.rx = None;
                    completed = Some(result);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(50));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    state.in_flight = None;
                    state.rx = None;
                    state.message =
                        Some(format!("{} action worker disconnected.", runtime.label()));
                }
            }
        }
    }

    let Some(result) = completed else {
        return;
    };

    let message = match result.outcome {
        Ok(JavaRuntimeActionOutcome::Downloaded(path)) => {
            config.set_java_runtime_path_ref(runtime, Some(path.as_path()));
            format!(
                "Downloaded {} and selected {}.",
                runtime.label(),
                path.display()
            )
        }
        Ok(JavaRuntimeActionOutcome::Validated(version)) => {
            format!("Java validation passed: {version}")
        }
        Err(err) => match result.kind {
            JavaRuntimeActionKind::Download => {
                format!("Failed to download {}: {err}", runtime.label())
            }
            JavaRuntimeActionKind::Validate => format!("Java validation failed: {err}"),
        },
    };

    if let Ok(mut states) = java_runtime_action_states().lock() {
        states.entry(runtime.major()).or_default().message = Some(message);
    }
}

fn render_java_runtime_path_actions(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &Config,
    runtime: JavaRuntimeVersion,
    edited_path_value: &str,
) {
    let (in_flight, message) = java_runtime_action_states()
        .lock()
        .ok()
        .and_then(|states| {
            states
                .get(&runtime.major())
                .map(|state| (state.in_flight, state.message.clone()))
        })
        .unwrap_or((None, None));

    let button_style = ButtonOptions {
        min_size: egui::vec2(180.0, 30.0),
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().widgets.active.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..ButtonOptions::default()
    };

    let current_path = normalize_optional_input(edited_path_value).or_else(|| {
        config
            .java_runtime_path_ref(runtime)
            .map(|path| path.as_os_str().to_string_lossy().into_owned())
    });
    let busy = in_flight.is_some();
    let download_label = match in_flight {
        Some(JavaRuntimeActionKind::Download) => "Downloading...".to_owned(),
        _ => format!("Download OpenJDK {}", runtime.major()),
    };
    let validate_label = match in_flight {
        Some(JavaRuntimeActionKind::Validate) => "Testing...".to_owned(),
        _ => "Test Java".to_owned(),
    };

    let has_current_path = current_path.is_some();
    ui.horizontal_wrapped(|ui| {
        let download_clicked = ui
            .add_enabled_ui(!busy, |ui| {
                text_ui.button(
                    ui,
                    ("settings_java_download", runtime.major()),
                    download_label.as_str(),
                    &button_style,
                )
            })
            .inner
            .clicked();
        if download_clicked {
            request_java_runtime_download(runtime);
        }

        let validate_clicked = ui
            .add_enabled_ui(!busy && has_current_path, |ui| {
                text_ui.button(
                    ui,
                    ("settings_java_validate", runtime.major()),
                    validate_label.as_str(),
                    &button_style,
                )
            })
            .inner
            .clicked();
        if validate_clicked {
            if let Some(path) = current_path.clone() {
                request_java_runtime_validation(runtime, path);
            }
        }
    });

    if !has_current_path && in_flight != Some(JavaRuntimeActionKind::Download) {
        let mut hint_style = style::muted(ui);
        hint_style.wrap = true;
        let _ = text_ui.label(
            ui,
            ("settings_java_validate_hint", runtime.major()),
            "Set a Java executable path before testing, or download OpenJDK.",
            &hint_style,
        );
    }

    if let Some(message) = message {
        let mut status_style = style::muted(ui);
        status_style.wrap = true;
        let _ = text_ui.label(
            ui,
            ("settings_java_action_status", runtime.major()),
            message.as_str(),
            &status_style,
        );
    }
}

fn request_java_runtime_download(runtime: JavaRuntimeVersion) {
    let (tx, rx) = mpsc::channel();
    if let Ok(mut states) = java_runtime_action_states().lock() {
        let state = states.entry(runtime.major()).or_default();
        state.in_flight = Some(JavaRuntimeActionKind::Download);
        state.rx = Some(rx);
        state.message = Some(format!("Downloading {}...", runtime.label()));
    }

    let runtime_major = runtime.major();
    tokio_runtime::spawn_detached(async move {
        let outcome = ensure_openjdk_runtime_async(runtime_major)
            .await
            .map(JavaRuntimeActionOutcome::Downloaded)
            .map_err(|err| err.to_string());
        let _ = tx.send(JavaRuntimeActionResult {
            kind: JavaRuntimeActionKind::Download,
            outcome,
        });
    });
}

fn request_java_runtime_validation(runtime: JavaRuntimeVersion, path: String) {
    let (tx, rx) = mpsc::channel();
    if let Ok(mut states) = java_runtime_action_states().lock() {
        let state = states.entry(runtime.major()).or_default();
        state.in_flight = Some(JavaRuntimeActionKind::Validate);
        state.rx = Some(rx);
        state.message = Some(format!("Testing {}...", path));
    }

    tokio_runtime::spawn_detached(async move {
        let outcome = validate_java_executable(path.as_str())
            .await
            .map(JavaRuntimeActionOutcome::Validated);
        let _ = tx.send(JavaRuntimeActionResult {
            kind: JavaRuntimeActionKind::Validate,
            outcome,
        });
    });
}

async fn validate_java_executable(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("No Java executable path is configured.".to_owned());
    }
    let path = Path::new(trimmed);
    tokio::fs::metadata(path)
        .await
        .map_err(|err| format!("Path is not readable: {trimmed}: {err}"))?;

    let output = tokio::process::Command::new(path)
        .arg("-version")
        .output()
        .await
        .map_err(|err| format!("Could not run {trimmed}: {err}"))?;
    let stdout = String::from_utf8_lossy(output.stdout.as_slice());
    let stderr = String::from_utf8_lossy(output.stderr.as_slice());
    let combined = format!("{stdout}\n{stderr}");
    let first_line = combined
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("<no version output>");
    let combined_lower = combined.to_ascii_lowercase();

    if !output.status.success() {
        return Err(format!(
            "`{trimmed} -version` exited with {}; output: {first_line}",
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_owned())
        ));
    }

    let looks_like_java = combined_lower.contains("openjdk")
        || combined_lower.contains("java version")
        || combined_lower.contains("runtime environment")
        || combined_lower.contains("java(tm)");
    if !looks_like_java {
        return Err(format!(
            "`{trimmed} -version` ran, but output did not look like Java: {first_line}"
        ));
    }

    Ok(first_line.to_owned())
}

fn normalize_optional_input(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn memory_slider_max_mib() -> (u128, bool) {
    static CACHED: OnceLock<Mutex<MemorySliderMaxState>> = OnceLock::new();
    let cache = CACHED.get_or_init(|| Mutex::new(MemorySliderMaxState::default()));
    let mut total_mib = None;
    let mut pending = false;

    if let Ok(mut state) = cache.lock() {
        if !state.load_complete {
            if let Some(rx) = state.rx.as_ref() {
                match rx.try_recv() {
                    Ok(result) => {
                        state.detected_total_mib = result;
                        state.load_complete = true;
                        state.rx = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        pending = true;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/settings",
                            "Memory slider max probe worker disconnected unexpectedly."
                        );
                        state.load_complete = true;
                        state.rx = None;
                    }
                }
            }

            if !state.load_complete && state.rx.is_none() {
                let (tx, rx) = mpsc::channel::<Option<u128>>();
                state.rx = Some(rx);
                pending = true;
                let _ = tokio_runtime::spawn_blocking_detached(move || {
                    let result = platform::detect_total_memory_mib();
                    if let Err(err) = tx.send(result) {
                        tracing::error!(
                            target: "vertexlauncher/settings",
                            error = %err,
                            "Failed to deliver memory slider max probe result."
                        );
                    }
                });
            }
        }
        total_mib = state.detected_total_mib;
        pending |= !state.load_complete;
    }

    let max_mib = total_mib
        .unwrap_or(FALLBACK_TOTAL_MEMORY_MIB)
        .saturating_sub(RESERVED_SYSTEM_MEMORY_MIB)
        .max(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN);
    (max_mib, pending)
}
