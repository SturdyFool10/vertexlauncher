use std::{
    collections::BTreeMap,
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
use std::{fs, io};

use config::GamepadCalibration;
use gilrs::{Axis, Button, EventType, Gamepad, GamepadId, Gilrs};
use launcher_ui::notification;
use launcher_ui::screens::AppScreen;

/// How long to wait after the first press before starting to repeat navigation.
const INITIAL_REPEAT_DELAY: Duration = Duration::from_millis(350);
/// How quickly to repeat while a direction is held.
const REPEAT_INTERVAL: Duration = Duration::from_millis(110);
/// Points scrolled per second at full right-stick deflection.
const RIGHT_STICK_SCROLL_SPEED: f32 = 1100.0;
/// Minimum right-stick deflection before scrolling begins.
const RIGHT_STICK_SCROLL_DEADZONE: f32 = 0.15;

/// Direction to move focus in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDir {
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

/// A plain 2D vector, so the poll result carries no UI toolkit type.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Vec2f {
    pub x: f32,
    pub y: f32,
}

impl Vec2f {
    const ZERO: Self = Self { x: 0.0, y: 0.0 };
}

/// What the controllers asked for this frame.
#[derive(Debug, Default, Clone)]
pub struct GamepadPoll {
    /// Focus moves, in order: presses and held-direction repeats.
    pub nav: Vec<NavDir>,
    /// A (South) was pressed.
    pub south: bool,
    /// B (East) was pressed.
    pub back: bool,
    /// X (West) was pressed: open the focused item's context menu.
    pub context_menu: bool,
    /// Left stick click: jump focus to the sidebar.
    pub home_focus: bool,
    /// Net slider steps (left/right while a slider has focus).
    pub slider_step: i32,
    /// Scroll to apply this frame, already scaled by the frame time (x = horizontal, y = vertical).
    pub scroll: Vec2f,
    pub skins_preview_orbit: f32,
    pub screenshot_viewer_pan: Vec2f,
    pub screenshot_viewer_zoom: f32,
    /// Any controller input was seen (so the UI can show focus rings).
    pub input_seen: bool,
    pub calibration_requested: Option<GamepadDeviceIdentity>,
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
    #[cfg(target_os = "linux")]
    linux_access_warning_emitted: bool,
    #[cfg(target_os = "linux")]
    linux_probe_logged: bool,
    /// Why no controller can be opened, worked out when the navigator is made (off the UI thread): finding out opens
    /// every input device, which takes a couple of hundred milliseconds.
    #[cfg(target_os = "linux")]
    linux_access_warning: Option<String>,
}

impl GamepadNavigator {
    /// Try to create a navigator. Returns `None` if no gamepad subsystem is available.
    pub fn new() -> Option<Self> {
        // The diagnostics below open and describe every input device, so they run here, where the navigator is made
        // (a worker), and not on the first frame of the UI.
        #[cfg(target_os = "linux")]
        log_linux_input_probe();
        #[cfg(target_os = "linux")]
        let linux_access_warning = linux_input_access_warning();
        match Gilrs::new() {
            Ok(gilrs) => Some(Self {
                gilrs,
                device_states: HashMap::new(),
                last_scroll_update: Instant::now(),
                known_gamepads: HashSet::new(),
                prompted_uncalibrated: HashSet::new(),
                pending_calibration_request: None,
                startup_scan_complete: false,
                #[cfg(target_os = "linux")]
                linux_access_warning_emitted: false,
                #[cfg(target_os = "linux")]
                linux_probe_logged: true,
                #[cfg(target_os = "linux")]
                linux_access_warning,
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

    /// Reads the controllers and says what the user asked for this frame. Knows nothing about any UI toolkit:
    /// `slider_focused` says the focused control is a slider, which takes left/right as value steps.
    pub fn poll(
        &mut self,
        calibrations: &BTreeMap<String, GamepadCalibration>,
        active_screen: AppScreen,
        slider_focused: bool,
    ) -> GamepadPoll {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_scroll_update)
            .as_secs_f32()
            .clamp(1.0 / 240.0, 0.05);
        self.last_scroll_update = now;
        self.detect_startup_gamepads(calibrations);
        self.maybe_log_linux_input_probe();
        self.maybe_warn_about_linux_input_access();

        let mut out = GamepadPoll::default();
        let mut scroll_delta: f32 = 0.0;
        let mut h_scroll_delta: f32 = 0.0;
        let mut nav_actions = Vec::new();

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
                        out.input_seen = true;
                        nav_actions.push((ev.id, NavDir::Up, HoldSource::Dpad))
                    }
                    Button::DPadRight => {
                        out.input_seen = true;
                        if slider_focused {
                            out.slider_step += 1;
                        } else {
                            nav_actions.push((ev.id, NavDir::Right, HoldSource::Dpad))
                        }
                    }
                    Button::DPadDown => {
                        out.input_seen = true;
                        nav_actions.push((ev.id, NavDir::Down, HoldSource::Dpad))
                    }
                    Button::DPadLeft => {
                        out.input_seen = true;
                        if slider_focused {
                            out.slider_step -= 1;
                        } else {
                            nav_actions.push((ev.id, NavDir::Left, HoldSource::Dpad))
                        }
                    }
                    Button::South => {
                        out.input_seen = true;
                        out.south = true;
                    }
                    Button::East => {
                        out.input_seen = true;
                        out.back = true
                    }
                    // X opens the focused item's context menu (the keyboard Shift+F10 chord).
                    Button::West => {
                        out.input_seen = true;
                        out.context_menu = true;
                    }
                    Button::LeftThumb => {
                        out.input_seen = true;
                        out.home_focus = true;
                    }
                    // Bumpers: coarse scroll
                    Button::LeftTrigger => {
                        out.input_seen = true;
                        scroll_delta += 200.0
                    }
                    Button::RightTrigger => {
                        out.input_seen = true;
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
            out.input_seen = true;
        }
        if slider_focused {
            if let Some(dir) = nav_actions
                .iter()
                .rev()
                .find_map(|(_, dir, source)| (*source == HoldSource::Analog).then_some(*dir))
            {
                match dir {
                    NavDir::Left => out.slider_step -= 1,
                    NavDir::Right => out.slider_step += 1,
                    NavDir::Up | NavDir::Down => {}
                }
            }
            nav_actions.retain(|(_, dir, _)| !matches!(dir, NavDir::Left | NavDir::Right));
        }

        // --- Hold-to-navigate ---
        for (id, dir, source) in nav_actions {
            out.nav.push(dir);
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
                        out.nav.push(dir);
                        state.last_repeat = Some(now);
                    }
                }
            }
        }

        // --- Right stick continuous scroll ---
        let mut skins_preview_orbit: f32 = 0.0;
        let mut screenshot_viewer_pan = Vec2f::ZERO;
        let mut screenshot_viewer_zoom: f32 = 0.0;
        for state in self.device_states.values() {
            if state.right_stick_y.abs() > RIGHT_STICK_SCROLL_DEADZONE {
                scroll_delta += state.right_stick_y * RIGHT_STICK_SCROLL_SPEED;
                out.input_seen = true;
            }
            if active_screen == AppScreen::Skins {
                if state.right_stick_x.abs() > RIGHT_STICK_SCROLL_DEADZONE {
                    skins_preview_orbit += state.right_stick_x;
                    out.input_seen = true;
                }
            } else if matches!(active_screen, AppScreen::Home | AppScreen::Instance) {
                if state.right_stick_x.abs() > RIGHT_STICK_SCROLL_DEADZONE
                    || state.right_stick_y.abs() > RIGHT_STICK_SCROLL_DEADZONE
                {
                    screenshot_viewer_pan.x += state.right_stick_x;
                    screenshot_viewer_pan.y += -state.right_stick_y;
                    out.input_seen = true;
                }
                let trigger_zoom = state.right_trigger - state.left_trigger;
                if trigger_zoom.abs() > 0.05 {
                    screenshot_viewer_zoom += trigger_zoom;
                    out.input_seen = true;
                }
            } else if state.right_stick_x.abs() > RIGHT_STICK_SCROLL_DEADZONE {
                h_scroll_delta -= state.right_stick_x * RIGHT_STICK_SCROLL_SPEED;
                out.input_seen = true;
            }
        }

        out.scroll = Vec2f {
            x: h_scroll_delta * dt,
            y: scroll_delta * dt,
        };
        let pan_len_sq = screenshot_viewer_pan.x * screenshot_viewer_pan.x
            + screenshot_viewer_pan.y * screenshot_viewer_pan.y;
        out.screenshot_viewer_pan = if pan_len_sq > 1.0 {
            let len = pan_len_sq.sqrt();
            Vec2f {
                x: screenshot_viewer_pan.x / len,
                y: screenshot_viewer_pan.y / len,
            }
        } else {
            screenshot_viewer_pan
        };
        out.skins_preview_orbit = skins_preview_orbit.clamp(-1.0, 1.0);
        out.screenshot_viewer_zoom = screenshot_viewer_zoom.clamp(-1.0, 1.0);
        out.calibration_requested = self.pending_calibration_request.take();
        out
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

    /// How many tracked controllers are connected.
    pub fn connected(&self) -> usize {
        self.known_gamepads.len()
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
                && let Some(message) = self.linux_access_warning.clone()
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
