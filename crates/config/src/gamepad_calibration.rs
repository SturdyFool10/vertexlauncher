use serde::{Deserialize, Serialize};

/// Gamepad calibration parameters for analog stick normalization.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GamepadCalibration {
    pub center_x: f32,
    pub center_y: f32,
    pub deadzone_x: f32,
    pub deadzone_y: f32,
    pub threshold_x: f32,
    pub threshold_y: f32,
    pub x_forward_sign: i8,
    pub y_backward_sign: i8,
}

impl Default for GamepadCalibration {
    fn default() -> Self {
        Self {
            center_x: 0.0,
            center_y: 0.0,
            deadzone_x: 0.25,
            deadzone_y: 0.25,
            threshold_x: 0.5,
            threshold_y: 0.5,
            x_forward_sign: 1,
            y_backward_sign: 1,
        }
    }
}

impl GamepadCalibration {
    /// Clamps all calibration values to valid ranges and ensures sign fields are ±1.
    pub fn normalize(&mut self) {
        self.center_x = self.center_x.clamp(-1.0, 1.0);
        self.center_y = self.center_y.clamp(-1.0, 1.0);
        self.deadzone_x = self.deadzone_x.clamp(0.05, 0.95);
        self.deadzone_y = self.deadzone_y.clamp(0.05, 0.95);
        self.threshold_x = self
            .threshold_x
            .clamp((self.deadzone_x + 0.05).min(0.95), 0.98);
        self.threshold_y = self
            .threshold_y
            .clamp((self.deadzone_y + 0.05).min(0.95), 0.98);
        self.x_forward_sign = if self.x_forward_sign >= 0 { 1 } else { -1 };
        self.y_backward_sign = if self.y_backward_sign >= 0 { 1 } else { -1 };
    }
}
