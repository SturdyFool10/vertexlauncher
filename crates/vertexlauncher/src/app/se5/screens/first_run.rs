//! First launch: choose the config file format.

use vertex_i18n::tr;
use config::{ConfigFormat, create_default_config};
use vertex_shell::sturdy::ui::{Sizing, TextStyle, TextWrap, Ui};
use vertex_shell::sturdy::ui_kit::ModalStyle;

use super::super::kit::{self, ButtonKind, Kit, TextRole};
use super::super::{AppState, Dialog};

const FORMATS: [ConfigFormat; 2] = [ConfigFormat::Json, ConfigFormat::Toml];

/// Returns false once the dialog should close.
pub fn config_format(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    state: &mut AppState,
    selected: ConfigFormat,
) -> bool {
    let style = ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(480.0),
        ..ModalStyle::default()
    };
    let mut choice = selected;
    let mut create = false;
    let response = kit::modal(ui, "first-run::format", &style, true, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text(ui, TextRole::Heading, &tr!("Welcome to Vertex"));
        kit::label(
            ui,
            &tr!("Vertex keeps its settings in one file. Choose its format; you can edit it by hand later."),
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Body)
            },
        );
        if let Some(next) = kit.choice(
            ui,
            "first-run::format::choice",
            choice,
            &FORMATS,
            |f| f.label().to_owned(),
            260.0,
        ) {
            choice = next;
        }
        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        create = kit.button(
            ui,
            "first-run::format::create",
            &tr!("Create settings file"),
            ButtonKind::Primary,
            true,
        );
    });
    if choice != selected {
        state.dialog = Some(Dialog::ConfigFormat(choice));
    }
    if !(create || response.dismissed) {
        return true;
    }
    match create_default_config(choice) {
        Ok(mut config) => {
            config.normalize();
            state.config = config;
            false
        }
        Err(err) => {
            launcher_ui::notification::error!("config", "Failed to create the settings file: {err}", err = err);
            true
        }
    }
}
