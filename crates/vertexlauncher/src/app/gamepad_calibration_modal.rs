use config::GamepadCalibration;
use eframe::egui;
use textui::{LabelOptions, TextUi};
use ui_foundation::{DialogPreset, dialog_options, primary_button, secondary_button, show_dialog};

use crate::app::gamepad::{GamepadDeviceIdentity, GamepadStickSample};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CalibrationStep {
    Neutral,
    Up,
    Down,
    Left,
    Right,
    Review,
}

#[derive(Debug, Clone, Default)]
pub struct GamepadCalibrationState {
    pub device: Option<GamepadDeviceIdentity>,
    step: Option<CalibrationStep>,
    neutral: Option<GamepadStickSample>,
    up: Option<GamepadStickSample>,
    down: Option<GamepadStickSample>,
    left: Option<GamepadStickSample>,
    right: Option<GamepadStickSample>,
    error: Option<String>,
}

impl GamepadCalibrationState {
    pub fn start(&mut self, device: GamepadDeviceIdentity) {
        self.device = Some(device);
        self.step = Some(CalibrationStep::Neutral);
        self.neutral = None;
        self.up = None;
        self.down = None;
        self.left = None;
        self.right = None;
        self.error = None;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn device_key(&self) -> Option<&str> {
        self.device.as_ref().map(|device| device.key.as_str())
    }
}

#[derive(Debug, Clone)]
pub enum ModalAction {
    None,
    Cancel,
    Save {
        device_key: String,
        calibration: GamepadCalibration,
    },
}

pub fn render(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut GamepadCalibrationState,
    live_sample: Option<GamepadStickSample>,
) -> ModalAction {
    let Some(device) = state.device.clone() else {
        return ModalAction::None;
    };
    let Some(step) = state.step else {
        return ModalAction::None;
    };

    let mut action = ModalAction::None;
    let response = show_dialog(
        ctx,
        dialog_options("gamepad_calibration_modal_window", DialogPreset::Form),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

            let heading = LabelOptions {
                font_size: 30.0,
                line_height: 34.0,
                weight: 700,
                color: ui.visuals().text_color(),
                wrap: false,
                ..LabelOptions::default()
            };
            let body = LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            };

            let _ = text_ui.label(
                ui,
                "gamepad_calibration_heading",
                "Calibrate Gamepad",
                &heading,
            );
            let device_label = format!("Device: {}", device.name);
            let _ = text_ui.label(
                ui,
                "gamepad_calibration_device",
                device_label.as_str(),
                &body,
            );
            let _ = text_ui.label(
                ui,
                "gamepad_calibration_step",
                step_description(step),
                &body,
            );

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            if let Some(sample) = live_sample {
                let live_label = format!("Live left stick: x={:.3}, y={:.3}", sample.x, sample.y);
                let _ = text_ui.label(ui, "gamepad_calibration_live", live_label.as_str(), &body);
            } else {
                let _ = text_ui.label(
                    ui,
                    "gamepad_calibration_live_missing",
                    "No live left-stick sample is available from this device yet.",
                    &body,
                );
            }

            if let Some(error) = state.error.as_deref() {
                let mut error_style = body.clone();
                error_style.color = ui.visuals().error_fg_color;
                let _ = text_ui.label(ui, "gamepad_calibration_error", error, &error_style);
            }

            ui.add_space(8.0);
            let capture_enabled = live_sample.is_some();
            let capture_label = match step {
                CalibrationStep::Neutral => "Use current neutral",
                CalibrationStep::Review => "Save calibration",
                _ => "Capture current position",
            };

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let primary_clicked = text_ui
                    .button(
                        ui,
                        "gamepad_calibration_primary",
                        capture_label,
                        &primary_button(ui, egui::vec2(170.0, 34.0)),
                    )
                    .clicked();
                let cancel_clicked = text_ui
                    .button(
                        ui,
                        "gamepad_calibration_cancel",
                        "Cancel",
                        &secondary_button(ui, egui::vec2(100.0, 34.0)),
                    )
                    .clicked();

                if cancel_clicked {
                    action = ModalAction::Cancel;
                    return;
                }

                if !primary_clicked {
                    return;
                }

                match step {
                    CalibrationStep::Review => {
                        if let Some(calibration) = build_calibration(state) {
                            action = ModalAction::Save {
                                device_key: device.key.clone(),
                                calibration,
                            };
                        } else {
                            state.error = Some(
                                "Calibration is incomplete or the captured values are inconsistent."
                                    .to_owned(),
                            );
                        }
                    }
                    _ if !capture_enabled => {
                        state.error =
                            Some("Move the stick so Vertex can see live input first.".to_owned());
                    }
                    _ => {
                        capture_step(state, step, live_sample.expect("capture_enabled checked"));
                    }
                }
            });
        },
    );

    if response.close_requested && matches!(action, ModalAction::None) {
        action = ModalAction::Cancel;
    }

    action
}

fn step_description(step: CalibrationStep) -> &'static str {
    match step {
        CalibrationStep::Neutral => {
            "Leave the left stick untouched, then capture its neutral resting position."
        }
        CalibrationStep::Up => "Push the left stick upward and capture it.",
        CalibrationStep::Down => "Push the left stick downward and capture it.",
        CalibrationStep::Left => "Push the left stick left and capture it.",
        CalibrationStep::Right => "Push the left stick right and capture it.",
        CalibrationStep::Review => {
            "Review complete. Save the calibration to use this controller automatically next time."
        }
    }
}

fn capture_step(
    state: &mut GamepadCalibrationState,
    step: CalibrationStep,
    sample: GamepadStickSample,
) {
    state.error = None;
    match step {
        CalibrationStep::Neutral => {
            state.neutral = Some(sample);
            state.step = Some(CalibrationStep::Up);
        }
        CalibrationStep::Up => {
            state.up = Some(sample);
            state.step = Some(CalibrationStep::Down);
        }
        CalibrationStep::Down => {
            state.down = Some(sample);
            state.step = Some(CalibrationStep::Left);
        }
        CalibrationStep::Left => {
            state.left = Some(sample);
            state.step = Some(CalibrationStep::Right);
        }
        CalibrationStep::Right => {
            state.right = Some(sample);
            state.step = Some(CalibrationStep::Review);
        }
        CalibrationStep::Review => {}
    }
}

fn build_calibration(state: &GamepadCalibrationState) -> Option<GamepadCalibration> {
    let neutral = state.neutral?;
    let up = state.up?;
    let down = state.down?;
    let left = state.left?;
    let right = state.right?;

    let right_dx = right.x - neutral.x;
    let left_dx = left.x - neutral.x;
    let up_dy = up.y - neutral.y;
    let down_dy = down.y - neutral.y;

    if right_dx.abs() < 0.2 || left_dx.abs() < 0.2 || up_dy.abs() < 0.2 || down_dy.abs() < 0.2 {
        return None;
    }

    let x_extent = right_dx.abs().min(left_dx.abs());
    let y_extent = up_dy.abs().min(down_dy.abs());
    let deadzone_x = (x_extent * 0.3).clamp(0.12, 0.45);
    let deadzone_y = (y_extent * 0.3).clamp(0.12, 0.45);
    let threshold_x = (x_extent * 0.65).clamp(deadzone_x + 0.08, 0.92);
    let threshold_y = (y_extent * 0.65).clamp(deadzone_y + 0.08, 0.92);

    let mut calibration = GamepadCalibration {
        center_x: neutral.x,
        center_y: neutral.y,
        deadzone_x,
        deadzone_y,
        threshold_x,
        threshold_y,
        x_forward_sign: if right_dx >= 0.0 { 1 } else { -1 },
        y_backward_sign: if up_dy >= 0.0 { 1 } else { -1 },
    };
    calibration.normalize();
    Some(calibration)
}
