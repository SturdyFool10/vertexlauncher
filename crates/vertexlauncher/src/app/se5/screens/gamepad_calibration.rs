//! The controller calibration dialog: capture the left stick at rest and pushed up, down, left and right, then save
//! the thresholds so navigation matches this particular controller.

use vertex_i18n::tr;
use vertex_shell::sturdy::ui::{Sizing, TextStyle, TextWrap, Ui};
use vertex_shell::sturdy::ui_kit::ModalStyle;

use super::super::super::gamepad_calibration_modal::ModalAction;
use super::super::AppState;
use super::super::kit::{self, ButtonKind, Kit, TextRole};

/// Returns false once the dialog should close.
pub fn dialog(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) -> bool {
    let AppState {
        calibration,
        gamepad,
        config,
        ..
    } = state;
    let Some(device) = calibration.device.clone() else {
        return false;
    };
    let Some(instructions) = calibration.instructions() else {
        return false;
    };
    let live = gamepad.navigator().and_then(|navigator| {
        calibration
            .device_key()
            .and_then(|key| navigator.current_left_stick(key))
    });
    // The stick moves while the dialog is open; keep sampling it.

    let style = ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(520.0),
        ..ModalStyle::default()
    };
    let mut cancel = false;
    let mut action = ModalAction::None;
    let response = kit::modal(ui, "gamepad-calibration", &style, true, |ui| {
        let _column = kit.column(ui, kit::SPACE_SM);
        kit.text(ui, TextRole::Heading, &tr!("Calibrate Gamepad"));
        kit::label(
            ui,
            &tr!("Device: {name}", name = device.name),
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
        kit::label(
            ui,
            &instructions,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
        kit.separator(ui);
        match live {
            Some(sample) => kit.text(
                ui,
                TextRole::Muted,
                &tr!("Live left stick: x={x:.3}, y={y:.3}", x = sample.x, y = sample.y),
            ),
            None => kit.text(
                ui,
                TextRole::Muted,
                &tr!("No live left-stick sample is available from this device yet."),
            ),
        }
        if let Some(error) = calibration.error() {
            kit::label(
                ui,
                error,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Danger)
                },
            );
        }
        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        cancel = kit.button(
            ui,
            "gamepad-calibration::cancel",
            &tr!("Cancel"),
            ButtonKind::Neutral,
            true,
        );
        if kit.button(
            ui,
            "gamepad-calibration::primary",
            &calibration.primary_label(),
            ButtonKind::Primary,
            true,
        ) {
            action = calibration.primary(live);
        }
    });
    let mut done = false;
    if let ModalAction::Save {
        device_key,
        calibration: saved,
    } = action
    {
        config.set_gamepad_calibration(device_key.clone(), saved);
        if let Some(navigator) = gamepad.navigator_mut() {
            navigator.reset_navigation_state(&device_key);
        }
        launcher_ui::notification::info!("gamepad", "Saved gamepad calibration.");
        done = true;
    }
    if cancel || (response.dismissed || kit.back_pressed()) {
        done = true;
    }
    if done {
        calibration.reset();
        kit.forget("gamepad-calibration");
    }
    !done
}
