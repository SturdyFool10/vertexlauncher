use std::{
    collections::BTreeMap,
    collections::{HashMap, HashSet},
    fs,
    io,
    time::{Duration, Instant},
};

use config::GamepadCalibration;
use egui::FocusDirection;
use gilrs::{Axis, Button, EventType, Gamepad, GamepadId, Gilrs};
use launcher_ui::notification;

/// How long to wait after the first press before starting to repeat navigation.
const INITIAL_REPEAT_DELAY: Duration = Duration::from_millis(350);
/// How quickly to repeat while a direction is held.
const REPEAT_INTERVAL: Duration = Duration::from_millis(110);
/// Points scrolled per frame at full right-stick deflection (~8 pts @ 60 fps ≈ 480 pts/s).
const RIGHT_STICK_SCROLL_SPEED: f32 = 8.0;
/// Minimum right-stick deflection before scrolling begins.
const RIGHT_STICK_SCROLL_DEADZONE: f32 = 0.15;

/// Whether to tab forward or backward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavDir {
    Forward,
    Backward,
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
        }
    }
}

/// Polls connected gamepads and translates their input into egui navigation events.
///
/// Call [`GamepadNavigator::update`] once per frame (before widgets are built).
pub struct GamepadNavigator {
    gilrs: Gilrs,
    device_states: HashMap<GamepadId, DeviceState>,
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
    ) -> GamepadUpdate {
        self.detect_startup_gamepads(calibrations);
        self.maybe_log_linux_input_probe();
        self.maybe_warn_about_linux_input_access();

        let mut activate = false;
        let mut back = false;
        let mut scroll_delta: f32 = 0.0;
        let mut h_scroll_delta: f32 = 0.0;
        let mut nav_actions = Vec::new();

        // Drain all pending events from gilrs.
        while let Some(ev) = self.gilrs.next_event() {
            if ev.is_dropped() {
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
                    // D-pad up/left = tab backward; down/right = tab forward.
                    Button::DPadUp | Button::DPadLeft => {
                        nav_actions.push((ev.id, NavDir::Backward, HoldSource::Dpad))
                    }
                    Button::DPadDown | Button::DPadRight => {
                        nav_actions.push((ev.id, NavDir::Forward, HoldSource::Dpad))
                    }
                    Button::South => activate = true,
                    Button::East => back = true,
                    // Bumpers: coarse scroll
                    Button::LeftTrigger => scroll_delta += 200.0,
                    Button::RightTrigger => scroll_delta -= 200.0,
                    // Triggers: fine scroll
                    Button::LeftTrigger2 => scroll_delta += 80.0,
                    Button::RightTrigger2 => scroll_delta -= 80.0,
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

        // --- Hold-to-navigate ---
        let now = Instant::now();

        for (id, dir, source) in nav_actions {
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
                        Self::fire_nav(ctx, dir);
                        state.last_repeat = Some(now);
                    }
                }
            }
        }

        // --- Right stick continuous scroll ---
        for state in self.device_states.values() {
            if state.right_stick_y.abs() > RIGHT_STICK_SCROLL_DEADZONE {
                scroll_delta += state.right_stick_y * RIGHT_STICK_SCROLL_SPEED;
            }
            if state.right_stick_x.abs() > RIGHT_STICK_SCROLL_DEADZONE {
                h_scroll_delta -= state.right_stick_x * RIGHT_STICK_SCROLL_SPEED;
            }
        }

        // --- Activate (A / South button) ---
        if activate {
            Self::inject_key(ctx, egui::Key::Enter, egui::Modifiers::default());
        }

        // --- Back (B / East button) ---
        if back {
            Self::inject_key(ctx, egui::Key::Escape, egui::Modifiers::default());
        }

        // --- Scroll ---
        if scroll_delta != 0.0 || h_scroll_delta != 0.0 {
            ctx.input_mut(|i| {
                i.smooth_scroll_delta.y += scroll_delta;
                i.smooth_scroll_delta.x += h_scroll_delta;
            });
        }

        GamepadUpdate {
            calibration_requested: self.pending_calibration_request.take(),
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
            .map(|(id, gamepad)| {
                let identity = Self::identity_from_gamepad(gamepad);
                (
                    id,
                    calibrations.get(identity.key.as_str()).cloned(),
                    gamepad.value(Axis::LeftStickX),
                    gamepad.value(Axis::LeftStickY),
                    gamepad.value(Axis::RightStickX),
                    gamepad.value(Axis::RightStickY),
                )
            })
            .collect::<Vec<_>>();

        for (gamepad_id, calibration, left_x, left_y, right_x, right_y) in snapshots {
            let state = self.device_states.entry(gamepad_id).or_default();
            state.right_stick_x = right_x;
            state.right_stick_y = right_y;

            let Some(calibration) = calibration else {
                state.stick_x = left_x;
                state.stick_y = left_y;
                state.stick_x_seen = true;
                state.stick_y_seen = true;
                continue;
            };

            Self::update_polled_axis_x(state, gamepad_id, left_x, &calibration, nav_actions);
            Self::update_polled_axis_y(state, gamepad_id, left_y, &calibration, nav_actions);

            let centered_x = Self::normalized_x(state.stick_x, &calibration).abs() < calibration.deadzone_x;
            let centered_y = Self::normalized_y(state.stick_y, &calibration).abs() < calibration.deadzone_y;
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
            nav_actions.push((gamepad_id, NavDir::Forward, HoldSource::Analog));
            state.stick_x_armed = false;
        } else if state.stick_x_armed && !was_left && now_left {
            nav_actions.push((gamepad_id, NavDir::Backward, HoldSource::Analog));
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
            nav_actions.push((gamepad_id, NavDir::Backward, HoldSource::Analog));
            state.stick_y_armed = false;
        } else if state.stick_y_armed && !was_down && now_down {
            nav_actions.push((gamepad_id, NavDir::Forward, HoldSource::Analog));
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
        self.gilrs.connected_gamepad(id).map(Self::identity_from_gamepad)
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
            vendor_id.map(|value| format!("{value:04x}")).unwrap_or_default(),
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
            NavDir::Forward => FocusDirection::Next,
            NavDir::Backward => FocusDirection::Previous,
        };
        ctx.memory_mut(|memory| memory.move_focus(direction));
        ctx.request_repaint();
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
