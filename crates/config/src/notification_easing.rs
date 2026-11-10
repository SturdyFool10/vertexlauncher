use vertex_i18n::src;
use serde::{Deserialize, Serialize};

/// Easing curve applied to the notification fade-out animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEasing {
    /// Constant rate of change.
    Linear,
    /// Quadratic acceleration then deceleration.
    QuadraticInOut,
    /// Cubic acceleration.
    CubicIn,
    /// Cubic deceleration.
    CubicOut,
    /// Cubic acceleration then deceleration.
    CubicInOut,
    /// Sinusoidal acceleration then deceleration.
    SineInOut,
    /// Exponential acceleration.
    ExponentialIn,
    /// Exponential deceleration.
    ExponentialOut,
    /// Exponential acceleration then deceleration.
    ExponentialInOut,
}

impl NotificationEasing {
    /// Array of all available easing curves in display order.
    pub const ALL: [NotificationEasing; 9] = [
        NotificationEasing::Linear,
        NotificationEasing::QuadraticInOut,
        NotificationEasing::CubicIn,
        NotificationEasing::CubicOut,
        NotificationEasing::CubicInOut,
        NotificationEasing::SineInOut,
        NotificationEasing::ExponentialIn,
        NotificationEasing::ExponentialOut,
        NotificationEasing::ExponentialInOut,
    ];

    /// Human-readable label for UI display.
    pub const fn label(self) -> &'static str {
        match self {
            NotificationEasing::Linear => src!("Linear"),
            NotificationEasing::QuadraticInOut => src!("Quadratic In/Out"),
            NotificationEasing::CubicIn => src!("Cubic In"),
            NotificationEasing::CubicOut => src!("Cubic Out"),
            NotificationEasing::CubicInOut => src!("Cubic In/Out"),
            NotificationEasing::SineInOut => src!("Sine In/Out"),
            NotificationEasing::ExponentialIn => src!("Exponential In"),
            NotificationEasing::ExponentialOut => src!("Exponential Out"),
            NotificationEasing::ExponentialInOut => src!("Exponential In/Out"),
        }
    }

    /// Maps linear progress `t` in `0..=1` onto the eased curve; `0` maps to `0` and `1` to `1`.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            NotificationEasing::Linear => t,
            NotificationEasing::QuadraticInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            NotificationEasing::CubicIn => t * t * t,
            NotificationEasing::CubicOut => 1.0 - (1.0 - t).powi(3),
            NotificationEasing::CubicInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            NotificationEasing::SineInOut => -((std::f32::consts::PI * t).cos() - 1.0) / 2.0,
            NotificationEasing::ExponentialIn => {
                if t <= 0.0 {
                    0.0
                } else {
                    2.0f32.powf(10.0 * t - 10.0)
                }
            }
            NotificationEasing::ExponentialOut => {
                if t >= 1.0 {
                    1.0
                } else {
                    1.0 - 2.0f32.powf(-10.0 * t)
                }
            }
            NotificationEasing::ExponentialInOut => {
                if t <= 0.0 {
                    0.0
                } else if t >= 1.0 {
                    1.0
                } else if t < 0.5 {
                    2.0f32.powf(20.0 * t - 10.0) / 2.0
                } else {
                    (2.0 - 2.0f32.powf(-20.0 * t + 10.0)) / 2.0
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NotificationEasing;

    #[test]
    fn every_curve_starts_at_zero_and_ends_at_one() {
        for easing in NotificationEasing::ALL {
            assert!(easing.apply(0.0).abs() < 1e-3, "{easing:?} start");
            assert!((easing.apply(1.0) - 1.0).abs() < 1e-3, "{easing:?} end");
        }
    }

    #[test]
    fn linear_is_identity_and_input_is_clamped() {
        assert_eq!(NotificationEasing::Linear.apply(0.3), 0.3);
        assert_eq!(NotificationEasing::CubicInOut.apply(2.0), 1.0);
        assert_eq!(NotificationEasing::CubicInOut.apply(-1.0), 0.0);
    }
}
