use config::Config;
use egui::Ui;
use instances::InstanceRecord;
use textui::TextUi;
use textui_egui::prelude::*;

#[cfg(target_os = "linux")]
use crate::ui::components::settings_widgets;

use super::super::platform as screen_platform;
use super::InstanceScreenState;

pub(super) fn effective_linux_graphics_settings_for_state(
    state: &InstanceScreenState,
    config: &Config,
) -> (bool, bool) {
    #[cfg(target_os = "linux")]
    {
        return effective_linux_graphics_settings_for_flags(
            state.linux_set_opengl_driver,
            state.linux_use_zink_driver,
            config.linux_set_opengl_driver(),
            config.linux_use_zink_driver(),
        );
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = config;
        return (state.linux_set_opengl_driver, state.linux_use_zink_driver);
    }
}

pub(super) fn linux_instance_driver_settings_for_save(
    state: &InstanceScreenState,
    existing: Option<&InstanceRecord>,
) -> (Option<bool>, Option<bool>) {
    #[cfg(target_os = "linux")]
    {
        let _ = existing;
        return linux_instance_driver_override_for_save(
            state.linux_set_opengl_driver,
            state.linux_use_zink_driver,
        );
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = state;
        return existing
            .map(|instance| {
                (
                    instance.linux_set_opengl_driver,
                    instance.linux_use_zink_driver,
                )
            })
            .unwrap_or((None, None));
    }
}

pub(super) fn render_platform_specific_instance_settings_section(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut InstanceScreenState,
    instance_id: &str,
    section_style: &LabelOptions,
    muted_style: &LabelOptions,
) {
    let Some(section) = screen_platform::current_platform_specific_section() else {
        return;
    };

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);

    let _ = text_ui.label(
        ui,
        (
            "instance_platform_settings_heading",
            instance_id,
            section.id,
        ),
        section.heading,
        section_style,
    );
    let _ = text_ui.label(
        ui,
        (
            "instance_platform_settings_description",
            instance_id,
            section.id,
        ),
        section.instance_description,
        muted_style,
    );
    ui.add_space(8.0);

    #[cfg(target_os = "linux")]
    {
        let _ = ui
            .push_id(("instance_linux_set_opengl_driver", instance_id), |ui| {
                settings_widgets::toggle_row(
                    text_ui,
                    ui,
                    "Set Linux OpenGL Driver",
                    Some(
                        "Linux-only. When enabled, this instance uses the Zink toggle below instead of the launcher-wide Linux OpenGL driver behavior. When disabled, this instance falls back to the launcher-wide Linux graphics settings. Versions using Vulkan directly should ignore it.",
                    ),
                    &mut state.linux_set_opengl_driver,
                )
            })
            .inner;
        ui.add_space(6.0);

        let _ = ui.add_enabled_ui(state.linux_set_opengl_driver, |ui| {
            ui.push_id(("instance_linux_use_zink_driver", instance_id), |ui| {
                settings_widgets::toggle_row(
                    text_ui,
                    ui,
                    "Use Zink Driver (Experimental)",
                    Some(
                        "Linux-only. Experimental. When the setting above is enabled, this toggle decides whether this instance forces Mesa Zink so OpenGL runs over Vulkan. When the setting above is disabled, the launcher-wide Zink setting is used instead. Versions using Vulkan directly should ignore it.",
                    ),
                    &mut state.linux_use_zink_driver,
                )
            })
            .inner
        });
        ui.add_space(8.0);
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = state;
    }
}

#[cfg(target_os = "linux")]
fn effective_linux_graphics_settings_for_flags(
    override_enabled: bool,
    use_zink_driver: bool,
    global_set_opengl_driver: bool,
    global_use_zink_driver: bool,
) -> (bool, bool) {
    if override_enabled {
        (true, use_zink_driver)
    } else {
        (global_set_opengl_driver, global_use_zink_driver)
    }
}

#[cfg(target_os = "linux")]
fn linux_instance_driver_override_for_save(
    override_enabled: bool,
    use_zink_driver: bool,
) -> (Option<bool>, Option<bool>) {
    if override_enabled {
        (Some(true), Some(use_zink_driver))
    } else {
        (None, None)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{
        effective_linux_graphics_settings_for_flags, linux_instance_driver_override_for_save,
    };

    #[test]
    fn linux_driver_override_is_cleared_when_instance_override_is_disabled() {
        assert_eq!(
            linux_instance_driver_override_for_save(false, false),
            (None, None)
        );
        assert_eq!(
            linux_instance_driver_override_for_save(false, true),
            (None, None)
        );
    }

    #[test]
    fn linux_driver_override_is_saved_when_instance_override_is_enabled() {
        assert_eq!(
            linux_instance_driver_override_for_save(true, false),
            (Some(true), Some(false))
        );
        assert_eq!(
            linux_instance_driver_override_for_save(true, true),
            (Some(true), Some(true))
        );
    }

    #[test]
    fn effective_linux_graphics_settings_use_global_when_instance_override_is_disabled() {
        assert_eq!(
            effective_linux_graphics_settings_for_flags(false, false, true, true),
            (true, true)
        );
    }

    #[test]
    fn effective_linux_graphics_settings_use_instance_zink_when_override_is_enabled() {
        assert_eq!(
            effective_linux_graphics_settings_for_flags(true, true, false, false),
            (true, true)
        );
    }
}
