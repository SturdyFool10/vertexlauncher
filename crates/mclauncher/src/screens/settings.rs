use config::{Config, DropdownSettingId, UiFontFamily};
use egui::Ui;
use textui::TextUi;

use crate::ui::{components::settings_widgets, theme::Theme};

pub fn render(
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
}
