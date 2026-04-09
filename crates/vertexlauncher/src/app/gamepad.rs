use std::{
    collections::BTreeMap,
    collections::{HashMap, HashSet},
    fs, io,
    time::{Duration, Instant},
};

use auth::MinecraftSkinVariant;
use config::GamepadCalibration;
use egui::FocusDirection;
use gilrs::{Axis, Button, EventType, Gamepad, GamepadId, Gilrs};
use launcher_ui::notification;
use launcher_ui::screens::{self, AppScreen};
use launcher_ui::ui::{components::settings_widgets, sidebar};
use textui_egui::{
    apply_gamepad_scroll_to_focused_target, apply_gamepad_scroll_to_registered_id,
    set_gamepad_scroll_delta,
};

/// How long to wait after the first press before starting to repeat navigation.
const INITIAL_REPEAT_DELAY: Duration = Duration::from_millis(350);
/// How quickly to repeat while a direction is held.
const REPEAT_INTERVAL: Duration = Duration::from_millis(110);
/// Points scrolled per second at full right-stick deflection.
const RIGHT_STICK_SCROLL_SPEED: f32 = 1100.0;
/// Minimum right-stick deflection before scrolling begins.
const RIGHT_STICK_SCROLL_DEADZONE: f32 = 0.15;

/// Direction to move focus within egui's spatial navigation graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavDir {
    Up,
    Right,
    Down,
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldSource {
    Dpad,
    Analog,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GamepadDeviceIdentity {
    pub key: String,
    pub name: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GamepadStickSample {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Default, Clone)]
pub struct GamepadUpdate {
    pub calibration_requested: Option<GamepadDeviceIdentity>,
    pub skins_preview_orbit: f32,
    pub screenshot_viewer_pan: egui::Vec2,
    pub screenshot_viewer_zoom: f32,
}

#[derive(Debug, Clone)]
struct DeviceState {
    /// Currently held navigation direction (None = nothing held).
    held_dir: Option<NavDir>,
    /// Which input source owns the current held direction.
    held_source: Option<HoldSource>,
    /// When the held direction first fired.
    held_since: Option<Instant>,
    /// When the last repeat navigation fired.
    last_repeat: Option<Instant>,
    /// Last sampled left-stick X value (for hysteresis).
    stick_x: f32,
    /// Whether we've seen at least one horizontal stick sample.
    stick_x_seen: bool,
    /// Whether horizontal stick navigation is armed after returning to center.
    stick_x_armed: bool,
    /// Last sampled left-stick Y value (for hysteresis).
    stick_y: f32,
    /// Whether we've seen at least one vertical stick sample.
    stick_y_seen: bool,
    /// Whether vertical stick navigation is armed after returning to center.
    stick_y_armed: bool,
    /// Last sampled right-stick Y value (for continuous vertical scrolling).
    right_stick_y: f32,
    /// Last sampled right-stick X value (for continuous horizontal scrolling).
    right_stick_x: f32,
    /// Last sampled analog left-trigger value.
    left_trigger: f32,
    /// Last sampled analog right-trigger value.
    right_trigger: f32,
}

#[derive(Debug, Clone, Copy)]
struct GamepadCapabilities {
    has_left_stick_x: bool,
    has_left_stick_y: bool,
    has_any_primary_stick: bool,
    has_any_face_button: bool,
    has_any_dpad_button: bool,
    has_any_shoulder_or_menu_button: bool,
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            held_dir: None,
            held_source: None,
            held_since: None,
            last_repeat: None,
            stick_x: 0.0,
            stick_x_seen: false,
            stick_x_armed: false,
            stick_y: 0.0,
            stick_y_seen: false,
            stick_y_armed: false,
            right_stick_y: 0.0,
            right_stick_x: 0.0,
            left_trigger: 0.0,
            right_trigger: 0.0,
        }
    }
}

/// Polls connected gamepads and translates their input into egui navigation events.
///
/// Call [`GamepadNavigator::update`] once per frame (before widgets are built).
pub struct GamepadNavigator {
    gilrs: Gilrs,
    device_states: HashMap<GamepadId, DeviceState>,
    last_scroll_update: Instant,
    known_gamepads: HashSet<GamepadId>,
    prompted_uncalibrated: HashSet<String>,
    pending_calibration_request: Option<GamepadDeviceIdentity>,
    startup_scan_complete: bool,
    linux_access_warning_emitted: bool,
    linux_probe_logged: bool,
}

impl GamepadNavigator {
    /// Try to create a navigator. Returns `None` if no gamepad subsystem is available.
    pub fn new() -> Option<Self> {
        match Gilrs::new() {
            Ok(gilrs) => Some(Self {
                gilrs,
                device_states: HashMap::new(),
                last_scroll_update: Instant::now(),
                known_gamepads: HashSet::new(),
                prompted_uncalibrated: HashSet::new(),
                pending_calibration_request: None,
                startup_scan_complete: false,
                linux_access_warning_emitted: false,
                linux_probe_logged: false,
            }),
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/gamepad",
                    error = %err,
                    "Gamepad subsystem unavailable; controller navigation disabled"
                );
                None
            }
        }
    }

    /// Process all pending gamepad events and inject navigation into egui.
    ///
    /// Should be called at the start of each `update` frame, before any widgets render.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        calibrations: &BTreeMap<String, GamepadCalibration>,
        active_screen: AppScreen,
    ) -> GamepadUpdate {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_scroll_update)
            .as_secs_f32()
            .clamp(1.0 / 240.0, 0.05);
        self.last_scroll_update = now;
        self.detect_startup_gamepads(calibrations);
        self.maybe_log_linux_input_probe();
        self.maybe_warn_about_linux_input_access();
        settings_widgets::set_gamepad_slider_step_delta(ctx, 0);
        settings_widgets::set_gamepad_activate_target(ctx, None);
        sidebar::request_home_focus(ctx, false);
        set_gamepad_scroll_delta(ctx, egui::Vec2::ZERO);

        let mut activate = false;
        let mut back = false;
        let mut scroll_delta: f32 = 0.0;
        let mut h_scroll_delta: f32 = 0.0;
        let mut skins_preview_orbit: f32 = 0.0;
        let mut screenshot_viewer_pan = egui::Vec2::ZERO;
        let mut screenshot_viewer_zoom: f32 = 0.0;
        let mut slider_step_delta: i32 = 0;
        let mut nav_actions = Vec::new();
        let mut controller_input_seen = false;
        let active_slider = settings_widgets::gamepad_active_slider(ctx);
        let focused_id = ctx.memory(|memory| memory.focused());

        // Drain all pending events from gilrs.
        while let Some(ev) = self.gilrs.next_event() {
            if ev.is_dropped() {
                continue;
            }
            if !matches!(ev.event, EventType::Disconnected)
                && self.gamepad_identity(ev.id).is_none()
            {
                continue;
            }
            match ev.event {
                EventType::Connected => {
                    let identity = self.gamepad_identity(ev.id);
                    let name = identity
                        .as_ref()
                        .map(|identity| identity.name.as_str())
                        .unwrap_or("Controller");
                    if self.known_gamepads.insert(ev.id) {
                        tracing::info!(
                            target: "vertexlauncher/gamepad",
                            gamepad_name = %name,
                            "Gamepad connected."
                        );
                        notification::info!("gamepad", "Gamepad connected: {name}");
                    }
                    if let Some(identity) = identity {
                        self.maybe_queue_calibration_prompt(&identity, calibrations);
                    }
                }
                EventType::Disconnected => {
                    let name = self.gamepad_name(ev.id);
                    self.device_states.remove(&ev.id);
                    if self.known_gamepads.remove(&ev.id) {
                        tracing::info!(
                            target: "vertexlauncher/gamepad",
                            gamepad_name = %name,
                            "Gamepad disconnected."
                        );
                        notification::info!("gamepad", "Gamepad disconnected: {name}");
                    }
                }
                EventType::ButtonPressed(button, _) => match button {
                    Button::DPadUp => {
                        controller_input_seen = true;
                        nav_actions.push((ev.id, NavDir::Up, HoldSource::Dpad))
                    }
                    Button::DPadRight => {
                        controller_input_seen = true;
                        if active_slider.is_some() && active_slider == focused_id {
                            slider_step_delta += 1;
                        } else {
                            nav_actions.push((ev.id, NavDir::Right, HoldSource::Dpad))
                        }
                    }
                    Button::DPadDown => {
                        controller_input_seen = true;
                        nav_actions.push((ev.id, NavDir::Down, HoldSource::Dpad))
                    }
                    Button::DPadLeft => {
                        controller_input_seen = true;
                        if active_slider.is_some() && active_slider == focused_id {
                            slider_step_delta -= 1;
                        } else {
                            nav_actions.push((ev.id, NavDir::Left, HoldSource::Dpad))
                        }
                    }
                    Button::South => {
                        controller_input_seen = true;
                        settings_widgets::set_gamepad_activate_target(ctx, focused_id);
                        activate = focused_id
                            .map(|id| !settings_widgets::is_gamepad_custom_activate_id(ctx, id))
                            .unwrap_or(true);
                    }
                    Button::East => {
                        controller_input_seen = true;
                        back = true
                    }
                    Button::LeftThumb => {
                        controller_input_seen = true;
                        settings_widgets::set_gamepad_active_slider(ctx, None);
                        sidebar::request_home_focus(ctx, true);
                    }
                    // Bumpers: coarse scroll
                    Button::LeftTrigger => {
                        controller_input_seen = true;
                        scroll_delta += 200.0
                    }
                    Button::RightTrigger => {
                        controller_input_seen = true;
                        scroll_delta -= 200.0
                    }
                    _ => {}
                },
                EventType::ButtonReleased(button, _) => {
                    if button.is_dpad() {
                        let state = self.device_states.entry(ev.id).or_default();
                        Self::clear_hold(state);
                    }
                }
                EventType::AxisChanged(_, _, _) => {}
                _ => {}
            }
        }

        self.poll_current_axes(calibrations, &mut nav_actions);
        if nav_actions
            .iter()
            .any(|(_, _, source)| *source == HoldSource::Analog)
        {
            controller_input_seen = true;
        }
        if active_slider.is_some() && active_slider == focused_id {
            if let Some(dir) = nav_actions
                .iter()
                .rev()
                .find_map(|(_, dir, source)| (*source == HoldSource::Analog).then_some(*dir))
            {
                match dir {
                    NavDir::Left => slider_step_delta -= 1,
                    NavDir::Right => slider_step_delta += 1,
                    NavDir::Up | NavDir::Down => {}
                }
            }
            nav_actions.retain(|(_, dir, _)| !matches!(dir, NavDir::Left | NavDir::Right));
        }
        settings_widgets::set_gamepad_slider_step_delta(ctx, slider_step_delta);

        // --- Hold-to-navigate ---
        for (id, dir, source) in nav_actions {
            if Self::handle_explicit_screen_focus_bridge(ctx, active_screen, dir) {
                if let Some(state) = self.device_states.get_mut(&id) {
                    state.held_dir = Some(dir);
                    state.held_source = Some(source);
                    state.held_since = Some(now);
                    state.last_repeat = None;
                }
                continue;
            }
            Self::fire_nav(ctx, dir);
            if let Some(state) = self.device_states.get_mut(&id) {
                state.held_dir = Some(dir);
                state.held_source = Some(source);
                state.held_since = Some(now);
                state.last_repeat = None;
            }
        }
        for state in self.device_states.values_mut() {
            if let (Some(dir), Some(held_since)) = (state.held_dir, state.held_since) {
                let elapsed = now.duration_since(held_since);
                if elapsed >= INITIAL_REPEAT_DELAY {
                    let repeat_base = state
                        .last_repeat
                        .unwrap_or(held_since + INITIAL_REPEAT_DELAY);
                    if now.duration_since(repeat_base) >= REPEAT_INTERVAL {
                        if !Self::handle_explicit_screen_focus_bridge(ctx, active_screen, dir) {
                            Self::fire_nav(ctx, dir);
                        }
                        state.last_repeat = Some(now);
                    }
                }
            }
        }

        // --- Right stick continuous scroll ---
        for state in self.device_states.values() {
            if state.right_stick_y.abs() > RIGHT_STICK_SCROLL_DEADZONE {
                scroll_delta += state.right_stick_y * RIGHT_STICK_SCROLL_SPEED;
                controller_input_seen = true;
            }
            if active_screen == AppScreen::Skins {
                if state.right_stick_x.abs() > RIGHT_STICK_SCROLL_DEADZONE {
                    skins_preview_orbit += state.right_stick_x;
                    controller_input_seen = true;
                }
            } else if matches!(active_screen, AppScreen::Home | AppScreen::Instance) {
                if state.right_stick_x.abs() > RIGHT_STICK_SCROLL_DEADZONE
                    || state.right_stick_y.abs() > RIGHT_STICK_SCROLL_DEADZONE
                {
                    screenshot_viewer_pan += egui::vec2(state.right_stick_x, -state.right_stick_y);
                    controller_input_seen = true;
                }
                let trigger_zoom = state.right_trigger - state.left_trigger;
                if trigger_zoom.abs() > 0.05 {
                    screenshot_viewer_zoom += trigger_zoom;
                    controller_input_seen = true;
                }
            } else if state.right_stick_x.abs() > RIGHT_STICK_SCROLL_DEADZONE {
                h_scroll_delta -= state.right_stick_x * RIGHT_STICK_SCROLL_SPEED;
                controller_input_seen = true;
            }
        }

        if controller_input_seen {
            settings_widgets::set_gamepad_input_history(ctx, true);
        }

        let scroll_delta = egui::vec2(h_scroll_delta, scroll_delta) * dt;

        // --- Activate (A / South button) ---
        if activate {
            Self::inject_key(ctx, egui::Key::Enter, egui::Modifiers::default());
        }

        // --- Back (B / East button) ---
        if back {
            Self::inject_key(ctx, egui::Key::Escape, egui::Modifiers::default());
        }

        // --- Scroll ---
        if scroll_delta != egui::Vec2::ZERO {
            let scrolled = if active_screen == AppScreen::Console {
                // Hard-bind right stick to the console log scroll area regardless of focus.
                if let Some(console_id) = screens::console_log_scroll_id(ctx) {
                    apply_gamepad_scroll_to_registered_id(ctx, console_id, scroll_delta)
                } else {
                    apply_gamepad_scroll_to_focused_target(ctx, scroll_delta)
                }
            } else {
                apply_gamepad_scroll_to_focused_target(ctx, scroll_delta)
            };
            if scrolled {
                ctx.request_repaint();
            }
        } else if skins_preview_orbit.abs() > 0.0 {
            ctx.request_repaint();
        }

        let screenshot_viewer_pan = if screenshot_viewer_pan.length_sq() > 1.0 {
            screenshot_viewer_pan.normalized()
        } else {
            screenshot_viewer_pan
        };

        GamepadUpdate {
            calibration_requested: self.pending_calibration_request.take(),
            skins_preview_orbit: skins_preview_orbit.clamp(-1.0, 1.0),
            screenshot_viewer_pan,
            screenshot_viewer_zoom: screenshot_viewer_zoom.clamp(-1.0, 1.0),
        }
    }

    fn poll_current_axes(
        &mut self,
        calibrations: &BTreeMap<String, GamepadCalibration>,
        nav_actions: &mut Vec<(GamepadId, NavDir, HoldSource)>,
    ) {
        let snapshots = self
            .gilrs
            .gamepads()
            .filter(|(_, gamepad)| Self::should_track_gamepad(*gamepad))
            .map(|(id, gamepad)| {
                let identity = Self::identity_from_gamepad(gamepad);
                (
                    id,
                    calibrations.get(identity.key.as_str()).cloned(),
                    gamepad.value(Axis::LeftStickX),
                    gamepad.value(Axis::LeftStickY),
                    gamepad.value(Axis::RightStickX),
                    gamepad.value(Axis::RightStickY),
                    gamepad.value(Axis::LeftZ),
                    gamepad.value(Axis::RightZ),
                )
            })
            .collect::<Vec<_>>();

        for (
            gamepad_id,
            calibration,
            left_x,
            left_y,
            right_x,
            right_y,
            left_trigger,
            right_trigger,
        ) in snapshots
        {
            let state = self.device_states.entry(gamepad_id).or_default();
            state.right_stick_x = right_x;
            state.right_stick_y = right_y;
            state.left_trigger = left_trigger;
            state.right_trigger = right_trigger;

            let Some(calibration) = calibration else {
                state.stick_x = left_x;
                state.stick_y = left_y;
                state.stick_x_seen = true;
                state.stick_y_seen = true;
                continue;
            };

            Self::update_polled_axis_x(state, gamepad_id, left_x, &calibration, nav_actions);
            Self::update_polled_axis_y(state, gamepad_id, left_y, &calibration, nav_actions);

            let centered_x =
                Self::normalized_x(state.stick_x, &calibration).abs() < calibration.deadzone_x;
            let centered_y =
                Self::normalized_y(state.stick_y, &calibration).abs() < calibration.deadzone_y;
            if centered_x && centered_y && matches!(state.held_source, Some(HoldSource::Analog)) {
                Self::clear_hold(state);
            }
        }
    }

    fn update_polled_axis_x(
        state: &mut DeviceState,
        gamepad_id: GamepadId,
        value: f32,
        calibration: &GamepadCalibration,
        nav_actions: &mut Vec<(GamepadId, NavDir, HoldSource)>,
    ) {
        if !state.stick_x_seen {
            state.stick_x_seen = true;
            state.stick_x = value;
            state.stick_x_armed =
                Self::normalized_x(value, calibration).abs() < calibration.deadzone_x;
            return;
        }

        let previous = Self::normalized_x(state.stick_x, calibration);
        let was_right = previous > calibration.threshold_x;
        let was_left = previous < -calibration.threshold_x;
        state.stick_x = value;
        let current = Self::normalized_x(value, calibration);
        let now_right = current > calibration.threshold_x;
        let now_left = current < -calibration.threshold_x;
        let released = was_right || was_left;
        let center = current.abs() < calibration.deadzone_x;

        if center {
            state.stick_x_armed = true;
        }

        if state.stick_x_armed && !was_right && now_right {
            nav_actions.push((gamepad_id, NavDir::Right, HoldSource::Analog));
            state.stick_x_armed = false;
        } else if state.stick_x_armed && !was_left && now_left {
            nav_actions.push((gamepad_id, NavDir::Left, HoldSource::Analog));
            state.stick_x_armed = false;
        } else if center && released {
            Self::clear_hold(state);
        }
    }

    fn update_polled_axis_y(
        state: &mut DeviceState,
        gamepad_id: GamepadId,
        value: f32,
        calibration: &GamepadCalibration,
        nav_actions: &mut Vec<(GamepadId, NavDir, HoldSource)>,
    ) {
        if !state.stick_y_seen {
            state.stick_y_seen = true;
            state.stick_y = value;
            state.stick_y_armed =
                Self::normalized_y(value, calibration).abs() < calibration.deadzone_y;
            return;
        }

        let previous = Self::normalized_y(state.stick_y, calibration);
        let was_up = previous > calibration.threshold_y;
        let was_down = previous < -calibration.threshold_y;
        state.stick_y = value;
        let current = Self::normalized_y(value, calibration);
        let now_up = current > calibration.threshold_y;
        let now_down = current < -calibration.threshold_y;
        let released = was_up || was_down;
        let center = current.abs() < calibration.deadzone_y;

        if center {
            state.stick_y_armed = true;
        }

        if state.stick_y_armed && !was_up && now_up {
            nav_actions.push((gamepad_id, NavDir::Up, HoldSource::Analog));
            state.stick_y_armed = false;
        } else if state.stick_y_armed && !was_down && now_down {
            nav_actions.push((gamepad_id, NavDir::Down, HoldSource::Analog));
            state.stick_y_armed = false;
        } else if center && released {
            Self::clear_hold(state);
        }
    }

    fn clear_hold(state: &mut DeviceState) {
        state.held_dir = None;
        state.held_source = None;
        state.held_since = None;
        state.last_repeat = None;
    }

    fn detect_startup_gamepads(&mut self, calibrations: &BTreeMap<String, GamepadCalibration>) {
        let connected_ids = self
            .gilrs
            .gamepads()
            .filter(|(_, gamepad)| Self::should_track_gamepad(*gamepad))
            .map(|(id, _)| id)
            .collect::<Vec<_>>();

        if !self.startup_scan_complete {
            if connected_ids.is_empty() {
                tracing::info!(
                    target: "vertexlauncher/gamepad",
                    "Gamepad subsystem initialized with no controllers connected."
                );
            } else {
                for id in &connected_ids {
                    if self.known_gamepads.insert(*id) {
                        let identity = self.gamepad_identity(*id);
                        let name = identity
                            .as_ref()
                            .map(|identity| identity.name.as_str())
                            .unwrap_or("Controller");
                        tracing::info!(
                            target: "vertexlauncher/gamepad",
                            gamepad_name = %name,
                            "Detected connected controller at startup."
                        );
                        notification::info!("gamepad", "Detected gamepad: {name}");
                        if let Some(identity) = identity {
                            self.maybe_queue_calibration_prompt(&identity, calibrations);
                        }
                    }
                }
            }
            self.startup_scan_complete = true;
            return;
        }

        for id in connected_ids {
            if self.known_gamepads.insert(id) {
                let identity = self.gamepad_identity(id);
                let name = identity
                    .as_ref()
                    .map(|identity| identity.name.as_str())
                    .unwrap_or("Controller");
                tracing::info!(
                    target: "vertexlauncher/gamepad",
                    gamepad_name = %name,
                    "Detected connected controller after startup."
                );
                notification::info!("gamepad", "Detected gamepad: {name}");
                if let Some(identity) = identity {
                    self.maybe_queue_calibration_prompt(&identity, calibrations);
                }
            }
        }
    }

    fn maybe_queue_calibration_prompt(
        &mut self,
        identity: &GamepadDeviceIdentity,
        calibrations: &BTreeMap<String, GamepadCalibration>,
    ) {
        let Some(gamepad) = self.connected_tracked_gamepad_by_key(identity.key.as_str()) else {
            return;
        };
        if !Self::supports_stick_calibration(gamepad) {
            return;
        }
        if calibrations.contains_key(identity.key.as_str()) {
            return;
        }
        if !self.prompted_uncalibrated.insert(identity.key.clone()) {
            return;
        }
        self.pending_calibration_request = Some(identity.clone());
        notification::info!(
            "gamepad",
            "Calibration needed for {}. Open the calibration modal to finish setup.",
            identity.name
        );
    }

    pub fn current_left_stick(&self, device_key: &str) -> Option<GamepadStickSample> {
        self.gilrs.gamepads().find_map(|(_, gamepad)| {
            if !Self::should_track_gamepad(gamepad) {
                return None;
            }
            let identity = Self::identity_from_gamepad(gamepad);
            (identity.key == device_key).then(|| GamepadStickSample {
                x: gamepad.value(Axis::LeftStickX),
                y: gamepad.value(Axis::LeftStickY),
            })
        })
    }

    pub fn reset_navigation_state(&mut self, device_key: &str) {
        let matching_ids = self
            .gilrs
            .gamepads()
            .filter_map(|(id, gamepad)| {
                if !Self::should_track_gamepad(gamepad) {
                    return None;
                }
                let identity = Self::identity_from_gamepad(gamepad);
                (identity.key == device_key).then_some(id)
            })
            .collect::<Vec<_>>();

        for id in matching_ids {
            if let Some(state) = self.device_states.get_mut(&id) {
                *state = DeviceState::default();
            }
        }
    }

    pub fn gamepad_identity(&self, id: GamepadId) -> Option<GamepadDeviceIdentity> {
        self.gilrs
            .connected_gamepad(id)
            .filter(|gamepad| Self::should_track_gamepad(*gamepad))
            .map(Self::identity_from_gamepad)
    }

    fn connected_tracked_gamepad_by_key(&self, device_key: &str) -> Option<Gamepad<'_>> {
        self.gilrs.gamepads().find_map(|(_, gamepad)| {
            if !Self::should_track_gamepad(gamepad) {
                return None;
            }
            let identity = Self::identity_from_gamepad(gamepad);
            (identity.key == device_key).then_some(gamepad)
        })
    }

    fn should_track_gamepad(gamepad: Gamepad<'_>) -> bool {
        Self::should_track_capabilities(Self::gamepad_capabilities(gamepad))
    }

    fn supports_stick_calibration(gamepad: Gamepad<'_>) -> bool {
        Self::supports_stick_calibration_capabilities(Self::gamepad_capabilities(gamepad))
    }

    fn gamepad_capabilities(gamepad: Gamepad<'_>) -> GamepadCapabilities {
        GamepadCapabilities {
            has_left_stick_x: gamepad.axis_data(Axis::LeftStickX).is_some(),
            has_left_stick_y: gamepad.axis_data(Axis::LeftStickY).is_some(),
            has_any_primary_stick: [
                Axis::LeftStickX,
                Axis::LeftStickY,
                Axis::RightStickX,
                Axis::RightStickY,
                Axis::DPadX,
                Axis::DPadY,
            ]
            .into_iter()
            .any(|axis| gamepad.axis_data(axis).is_some()),
            has_any_face_button: [
                Button::South,
                Button::East,
                Button::North,
                Button::West,
                Button::C,
                Button::Z,
            ]
            .into_iter()
            .any(|button| gamepad.button_data(button).is_some()),
            has_any_dpad_button: [
                Button::DPadUp,
                Button::DPadDown,
                Button::DPadLeft,
                Button::DPadRight,
            ]
            .into_iter()
            .any(|button| gamepad.button_data(button).is_some()),
            has_any_shoulder_or_menu_button: [
                Button::LeftTrigger,
                Button::LeftTrigger2,
                Button::RightTrigger,
                Button::RightTrigger2,
                Button::Select,
                Button::Start,
                Button::Mode,
                Button::LeftThumb,
                Button::RightThumb,
            ]
            .into_iter()
            .any(|button| gamepad.button_data(button).is_some()),
        }
    }

    fn should_track_capabilities(capabilities: GamepadCapabilities) -> bool {
        let has_standard_buttons = capabilities.has_any_face_button
            || capabilities.has_any_dpad_button
            || capabilities.has_any_shoulder_or_menu_button;
        has_standard_buttons && capabilities.has_any_primary_stick
    }

    fn supports_stick_calibration_capabilities(capabilities: GamepadCapabilities) -> bool {
        capabilities.has_left_stick_x
            && capabilities.has_left_stick_y
            && (capabilities.has_any_face_button
                || capabilities.has_any_dpad_button
                || capabilities.has_any_shoulder_or_menu_button)
    }

    fn identity_from_gamepad(gamepad: Gamepad<'_>) -> GamepadDeviceIdentity {
        let name = gamepad.name().trim().to_owned();
        let vendor_id = gamepad.vendor_id();
        let product_id = gamepad.product_id();
        let uuid = gamepad.uuid();
        let uuid = if uuid.iter().all(|byte| *byte == 0) {
            None
        } else {
            Some(
                uuid.iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
            )
        };
        let key = format!(
            "{}:{}:{}:{}",
            name.to_ascii_lowercase(),
            vendor_id
                .map(|value| format!("{value:04x}"))
                .unwrap_or_default(),
            product_id
                .map(|value| format!("{value:04x}"))
                .unwrap_or_default(),
            uuid.clone().unwrap_or_default()
        );
        GamepadDeviceIdentity {
            key,
            name,
            vendor_id,
            product_id,
            uuid,
        }
    }

    fn maybe_warn_about_linux_input_access(&mut self) {
        #[cfg(target_os = "linux")]
        {
            if self.startup_scan_complete
                && self.known_gamepads.is_empty()
                && !self.linux_access_warning_emitted
                && let Some(message) = linux_input_access_warning()
            {
                self.linux_access_warning_emitted = true;
                tracing::warn!(
                    target: "vertexlauncher/gamepad",
                    warning = %message,
                    "Gamepad access diagnostic."
                );
                notification::warn!("gamepad", "{message}");
            }
        }
    }

    fn maybe_log_linux_input_probe(&mut self) {
        #[cfg(target_os = "linux")]
        {
            if self.startup_scan_complete && !self.linux_probe_logged {
                self.linux_probe_logged = true;
                log_linux_input_probe();
            }
        }
    }

    fn gamepad_name(&self, id: GamepadId) -> String {
        self.gamepad_identity(id)
            .map(|identity| identity.name)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("Controller {id:?}"))
    }

    fn normalized_x(value: f32, calibration: &GamepadCalibration) -> f32 {
        (value - calibration.center_x) * calibration.x_forward_sign as f32
    }

    fn normalized_y(value: f32, calibration: &GamepadCalibration) -> f32 {
        (value - calibration.center_y) * calibration.y_backward_sign as f32
    }

    /// Advance egui keyboard focus in the requested direction.
    fn fire_nav(ctx: &egui::Context, dir: NavDir) {
        let direction = match dir {
            NavDir::Up => FocusDirection::Up,
            NavDir::Right => FocusDirection::Right,
            NavDir::Down => FocusDirection::Down,
            NavDir::Left => FocusDirection::Left,
        };
        ctx.memory_mut(|memory| memory.move_focus(direction));
        ctx.request_repaint();
    }

    fn handle_explicit_screen_focus_bridge(
        ctx: &egui::Context,
        active_screen: AppScreen,
        dir: NavDir,
    ) -> bool {
        if active_screen == AppScreen::Settings
            && dir == NavDir::Right
            && Self::focused_widget_is_in_sidebar(ctx)
        {
            if let Some(focused_id) = ctx.memory(|memory| memory.focused()) {
                ctx.memory_mut(|memory| memory.surrender_focus(focused_id));
            }
            screens::request_settings_theme_focus(ctx);
            ctx.request_repaint();
            return true;
        }

        if active_screen == AppScreen::Skins
            && dir == NavDir::Right
            && Self::focused_widget_is_in_sidebar(ctx)
        {
            if let Some(focused_id) = ctx.memory(|memory| memory.focused()) {
                ctx.memory_mut(|memory| memory.surrender_focus(focused_id));
            }
            screens::request_skins_motion_focus(ctx);
            ctx.request_repaint();
            return true;
        }

        if active_screen == AppScreen::Console
            && dir == NavDir::Right
            && Self::focused_widget_is_in_sidebar(ctx)
        {
            if let Some(focused_id) = ctx.memory(|memory| memory.focused()) {
                ctx.memory_mut(|memory| memory.surrender_focus(focused_id));
            }
            screens::request_console_tab_focus(ctx);
            ctx.request_repaint();
            return true;
        }

        if active_screen == AppScreen::Skins {
            let focused_id = ctx.memory(|memory| memory.focused());
            if dir == NavDir::Right && focused_id == screens::skins_classic_model_button_id(ctx) {
                screens::request_skins_model_focus(ctx, MinecraftSkinVariant::Slim);
                ctx.request_repaint();
                return true;
            }
            if dir == NavDir::Left && focused_id == screens::skins_slim_model_button_id(ctx) {
                screens::request_skins_model_focus(ctx, MinecraftSkinVariant::Classic);
                ctx.request_repaint();
                return true;
            }
        }

        if active_screen == AppScreen::Instance {
            let focused_id = ctx.memory(|memory| memory.focused());
            if dir == NavDir::Right
                && focused_id == screens::instance_top_content_tab_id(ctx)
                && let Some(target) = screens::instance_top_screenshots_tab_id(ctx)
            {
                ctx.memory_mut(|memory| {
                    if let Some(focused_id) = focused_id {
                        memory.surrender_focus(focused_id);
                    }
                    memory.request_focus(target);
                });
                ctx.request_repaint();
                return true;
            }
            if dir == NavDir::Right
                && focused_id == screens::instance_top_screenshots_tab_id(ctx)
                && let Some(target) = screens::instance_top_logs_tab_id(ctx)
            {
                ctx.memory_mut(|memory| {
                    if let Some(focused_id) = focused_id {
                        memory.surrender_focus(focused_id);
                    }
                    memory.request_focus(target);
                });
                ctx.request_repaint();
                return true;
            }
            if dir == NavDir::Left
                && focused_id == screens::instance_top_logs_tab_id(ctx)
                && let Some(target) = screens::instance_top_screenshots_tab_id(ctx)
            {
                ctx.memory_mut(|memory| {
                    if let Some(focused_id) = focused_id {
                        memory.surrender_focus(focused_id);
                    }
                    memory.request_focus(target);
                });
                ctx.request_repaint();
                return true;
            }
            if dir == NavDir::Left
                && focused_id == screens::instance_top_screenshots_tab_id(ctx)
                && let Some(target) = screens::instance_top_content_tab_id(ctx)
            {
                ctx.memory_mut(|memory| {
                    if let Some(focused_id) = focused_id {
                        memory.surrender_focus(focused_id);
                    }
                    memory.request_focus(target);
                });
                ctx.request_repaint();
                return true;
            }
            if dir == NavDir::Right
                && focused_id == screens::instance_content_resource_packs_tab_id(ctx)
                && let Some(target) = screens::instance_content_shader_packs_tab_id(ctx)
            {
                ctx.memory_mut(|memory| {
                    if let Some(focused_id) = focused_id {
                        memory.surrender_focus(focused_id);
                    }
                    memory.request_focus(target);
                });
                ctx.request_repaint();
                return true;
            }
            if dir == NavDir::Left
                && focused_id == screens::instance_content_shader_packs_tab_id(ctx)
                && let Some(target) = screens::instance_content_resource_packs_tab_id(ctx)
            {
                ctx.memory_mut(|memory| {
                    if let Some(focused_id) = focused_id {
                        memory.surrender_focus(focused_id);
                    }
                    memory.request_focus(target);
                });
                ctx.request_repaint();
                return true;
            }
        }

        if active_screen == AppScreen::ContentBrowser {
            let focused_id = ctx.memory(|memory| memory.focused());
            let move_focus = |target: Option<egui::Id>| {
                if let Some(target) = target {
                    ctx.memory_mut(|memory| {
                        if let Some(focused_id) = focused_id {
                            memory.surrender_focus(focused_id);
                        }
                        memory.request_focus(target);
                    });
                    ctx.request_repaint();
                    return true;
                }
                false
            };

            if dir == NavDir::Right
                && focused_id == screens::content_browser_version_dropdown_id(ctx)
            {
                return move_focus(screens::content_browser_scope_dropdown_id(ctx));
            }
            if dir == NavDir::Right && focused_id == screens::content_browser_scope_dropdown_id(ctx)
            {
                return move_focus(screens::content_browser_sort_dropdown_id(ctx));
            }
            if dir == NavDir::Right && focused_id == screens::content_browser_sort_dropdown_id(ctx)
            {
                return move_focus(screens::content_browser_loader_dropdown_id(ctx));
            }
            if dir == NavDir::Left && focused_id == screens::content_browser_loader_dropdown_id(ctx)
            {
                return move_focus(screens::content_browser_sort_dropdown_id(ctx));
            }
            if dir == NavDir::Left && focused_id == screens::content_browser_sort_dropdown_id(ctx) {
                return move_focus(screens::content_browser_scope_dropdown_id(ctx));
            }
            if dir == NavDir::Left && focused_id == screens::content_browser_scope_dropdown_id(ctx)
            {
                return move_focus(screens::content_browser_version_dropdown_id(ctx));
            }
        }

        false
    }

    fn focused_widget_is_in_sidebar(ctx: &egui::Context) -> bool {
        let Some(focused_id) = ctx.memory(|memory| memory.focused()) else {
            return false;
        };
        let Some(response) = ctx.read_response(focused_id) else {
            return false;
        };
        let viewport_width = ctx.input(|input| input.content_rect().width());
        let sidebar_boundary = (viewport_width * 0.2).clamp(72.0, 180.0);
        response.rect.center().x <= sidebar_boundary
    }

    /// Inject a key press+release pair.
    fn inject_key(ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
        ctx.input_mut(|i| {
            i.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
            i.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            });
        });
    }
}

#[cfg(target_os = "linux")]
fn linux_input_access_warning() -> Option<String> {
    let input_dir = match fs::read_dir("/dev/input") {
        Ok(dir) => dir,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Some(
                "Linux gamepad input unavailable: /dev/input is missing, so evdev devices cannot be opened."
                    .to_owned(),
            );
        }
        Err(err) => {
            return Some(format!(
                "Linux gamepad input unavailable: failed to read /dev/input ({err})."
            ));
        }
    };

    let event_paths = input_dir
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("event"))
        })
        .collect::<Vec<_>>();

    if event_paths.is_empty() {
        return Some(
            "Linux gamepad input unavailable: no /dev/input/event* devices were found.".to_owned(),
        );
    }

    let writable = event_paths.iter().any(|path| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .is_ok()
    });

    if writable {
        None
    } else {
        Some(
            "Linux gamepad input may be blocked: VertexLauncher could not open any /dev/input/event* device with read/write access."
                .to_owned(),
        )
    }
}

#[cfg(target_os = "linux")]
fn log_linux_input_probe() {
    let input_dir = match fs::read_dir("/dev/input") {
        Ok(dir) => dir,
        Err(err) => {
            tracing::warn!(
                target: "vertexlauncher/gamepad",
                error = %err,
                "Linux input probe could not read /dev/input."
            );
            return;
        }
    };

    let mut event_paths = input_dir
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("event"))
        })
        .collect::<Vec<_>>();
    event_paths.sort();

    tracing::info!(
        target: "vertexlauncher/gamepad",
        event_device_count = event_paths.len(),
        "Linux input probe enumerated event devices."
    );

    for path in event_paths {
        let rw_access = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .is_ok();
        let ro_access = std::fs::OpenOptions::new().read(true).open(&path).is_ok();
        let device_name = linux_event_device_name(&path);
        tracing::info!(
            target: "vertexlauncher/gamepad",
            path = %path.display(),
            device_name = device_name.as_deref().unwrap_or("unknown"),
            read_only = ro_access,
            read_write = rw_access,
            "Linux input probe device access."
        );
    }
}

#[cfg(target_os = "linux")]
fn linux_event_device_name(event_path: &std::path::Path) -> Option<String> {
    let event_name = event_path.file_name()?.to_str()?;
    let sysfs_name_path = std::path::Path::new("/sys/class/input")
        .join(event_name)
        .join("device/name");
    fs::read_to_string(sysfs_name_path)
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{GamepadCapabilities, GamepadNavigator};

    #[test]
    fn tracked_gamepads_need_standard_controls_and_axes() {
        let capabilities = GamepadCapabilities {
            has_left_stick_x: true,
            has_left_stick_y: true,
            has_any_primary_stick: true,
            has_any_face_button: true,
            has_any_dpad_button: false,
            has_any_shoulder_or_menu_button: false,
        };

        assert!(GamepadNavigator::should_track_capabilities(capabilities));
        assert!(GamepadNavigator::supports_stick_calibration_capabilities(
            capabilities
        ));
    }

    #[test]
    fn stick_calibration_requires_both_left_stick_axes() {
        let capabilities = GamepadCapabilities {
            has_left_stick_x: true,
            has_left_stick_y: false,
            has_any_primary_stick: true,
            has_any_face_button: true,
            has_any_dpad_button: false,
            has_any_shoulder_or_menu_button: false,
        };

        assert!(GamepadNavigator::should_track_capabilities(capabilities));
        assert!(!GamepadNavigator::supports_stick_calibration_capabilities(
            capabilities
        ));
    }

    #[test]
    fn pseudo_devices_without_standard_buttons_are_ignored() {
        let capabilities = GamepadCapabilities {
            has_left_stick_x: true,
            has_left_stick_y: true,
            has_any_primary_stick: true,
            has_any_face_button: false,
            has_any_dpad_button: false,
            has_any_shoulder_or_menu_button: false,
        };

        assert!(!GamepadNavigator::should_track_capabilities(capabilities));
    }
}
