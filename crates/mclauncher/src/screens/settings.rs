use config::{
    Config, DropdownSettingId, INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN,
    INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP, JavaRuntimeVersion, UiFontFamily,
};
use egui::Ui;
use installation::purge_cache as purge_installation_cache;
use std::sync::OnceLock;
use textui::{ButtonOptions, LabelOptions, TextUi};

use crate::ui::{components::settings_widgets, theme::Theme};

const RESERVED_SYSTEM_MEMORY_MIB: u128 = 4 * 1024;
const FALLBACK_TOTAL_MEMORY_MIB: u128 = 20 * 1024;

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    available_ui_fonts: &[UiFontFamily],
    available_themes: &[Theme],
) {
    egui::ScrollArea::vertical()
        .id_salt("settings_page_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            render_settings_contents(ui, text_ui, config, available_ui_fonts, available_themes);
        });
}

fn render_settings_contents(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    available_ui_fonts: &[UiFontFamily],
    available_themes: &[Theme],
) {
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(10.0);

    config.for_each_toggle_mut(|setting, value| {
        ui.push_id(setting.id, |ui| {
            settings_widgets::toggle_row(text_ui, ui, setting.label, setting.info_tooltip, value);
        });
        ui.add_space(8.0);
    });

    if !available_themes.is_empty() {
        let mut selected_theme_index = available_themes
            .iter()
            .position(|theme| theme.id == config.theme_id())
            .unwrap_or(0);
        let theme_labels: Vec<&str> = available_themes
            .iter()
            .map(|theme| theme.name.as_str())
            .collect();
        let response = settings_widgets::dropdown_row(
            text_ui,
            ui,
            "theme_selector",
            "Theme",
            Some("Themes are loaded from the themes/ folder at startup."),
            &mut selected_theme_index,
            &theme_labels,
        );
        if response.changed() {
            if let Some(theme) = available_themes.get(selected_theme_index) {
                config.set_theme_id(theme.id.clone());
            }
        }
        ui.add_space(8.0);
    }

    config.for_each_dropdown_mut(|setting, value| {
        ui.push_id(setting.id, |ui| {
            let options: &[UiFontFamily] = match setting.id {
                DropdownSettingId::UiFontFamily => available_ui_fonts,
            };
            if options.is_empty() {
                return;
            }

            if !options.contains(value) {
                *value = options[0];
            }

            let option_labels: Vec<&str> = options
                .iter()
                .map(|option| option.settings_label())
                .collect();
            let mut selected_index = options
                .iter()
                .position(|option| *option == *value)
                .unwrap_or(0);

            let response = settings_widgets::dropdown_row(
                text_ui,
                ui,
                setting.id,
                setting.label,
                setting.info_tooltip,
                &mut selected_index,
                &option_labels,
            );

            if response.changed() {
                if let Some(next_value) = options.get(selected_index).copied() {
                    *value = next_value;
                }
            }
        });
        ui.add_space(8.0);
    });

    config.for_each_float_mut(|setting, value| {
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
        ui.add_space(8.0);
    });

    config.for_each_int_mut(|setting, value| {
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
        ui.add_space(8.0);
    });

    config.for_each_text_mut(|setting, value| {
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
        ui.add_space(8.0);
    });

    render_instance_defaults_section(ui, text_ui, config);
}

fn render_instance_defaults_section(ui: &mut Ui, text_ui: &mut TextUi, config: &mut Config) {
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(10.0);

    let heading_style = LabelOptions {
        font_size: 20.0,
        line_height: 24.0,
        weight: 700,
        color: ui.visuals().text_color(),
        wrap: false,
        ..LabelOptions::default()
    };
    let mut body_style = LabelOptions::default();
    body_style.color = ui.visuals().weak_text_color();
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
    ui.add_space(8.0);

    let mut installations_root = config.minecraft_installations_root().to_owned();
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
    ui.add_space(8.0);

    let cache_button_style = ButtonOptions {
        min_size: egui::vec2(220.0, 32.0),
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..ButtonOptions::default()
    };
    let cache_status_id = ui.make_persistent_id("settings_cache_status_message");
    let mut cache_status = ui.ctx().data_mut(|d| d.get_temp::<String>(cache_status_id));
    if text_ui
        .button(
            ui,
            "instance_defaults_purge_cache",
            "Purge metadata cache",
            &cache_button_style,
        )
        .clicked()
    {
        cache_status = Some(match purge_installation_cache() {
            Ok(()) => {
                "Purged local metadata cache. Version lists will be re-fetched on next refresh."
                    .to_owned()
            }
            Err(err) => format!("Failed to purge metadata cache: {err}"),
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
    ui.add_space(8.0);

    let mut default_memory = config.default_instance_max_memory_mib();
    let max_memory_mib = memory_slider_max_mib();
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

fn render_java_runtime_path_row(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    config: &mut Config,
    runtime: JavaRuntimeVersion,
) {
    let mut path_value = config
        .java_runtime_path(runtime)
        .unwrap_or_default()
        .to_owned();
    let response = settings_widgets::full_width_text_input_row(
        text_ui,
        ui,
        ("instance_java_path", runtime.major()),
        runtime.label(),
        Some(runtime.info_tooltip()),
        &mut path_value,
    );
    if response.changed() {
        config.set_java_runtime_path(runtime, normalize_optional_input(&path_value));
    }
    ui.add_space(8.0);
}

fn normalize_optional_input(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn memory_slider_max_mib() -> u128 {
    static CACHED: OnceLock<u128> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let total_mib = detect_total_memory_mib().unwrap_or(FALLBACK_TOTAL_MEMORY_MIB);
        total_mib
            .saturating_sub(RESERVED_SYSTEM_MEMORY_MIB)
            .max(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN)
    })
}

#[cfg(target_os = "linux")]
fn detect_total_memory_mib() -> Option<u128> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u128>().ok()?;
    Some(kib / 1024)
}

#[cfg(target_os = "windows")]
fn detect_total_memory_mib() -> Option<u128> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..unsafe { std::mem::zeroed() }
    };

    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return None;
    }

    Some((status.ullTotalPhys as u128) / (1024 * 1024))
}

#[cfg(target_os = "macos")]
fn detect_total_memory_mib() -> Option<u128> {
    let output = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let bytes = String::from_utf8(output.stdout).ok()?;
    let bytes = bytes.trim().parse::<u128>().ok()?;
    Some(bytes / (1024 * 1024))
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn detect_total_memory_mib() -> Option<u128> {
    None
}
