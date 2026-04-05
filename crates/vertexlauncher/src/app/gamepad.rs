use std::{
    collections::{HashMap, HashSet},
    fs,
    io,
    time::{Duration, Instant},
};

use egui::FocusDirection;
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use launcher_ui::notification;

/// How long to wait after the first press before starting to repeat navigation.
const INITIAL_REPEAT_DELAY: Duration = Duration::from_millis(350);
/// How quickly to repeat while a direction is held.
const REPEAT_INTERVAL: Duration = Duration::from_millis(110);
/// Analog stick deflection required to trigger navigation (left stick).
const STICK_THRESHOLD: f32 = 0.5;
/// How far the stick must return toward center before re-triggering (left stick).
const STICK_DEADZONE: f32 = 0.25;
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

#[derive(Debug, Clone)]
struct DeviceState {
    /// Currently held navigation direction (None = nothing held).
    held_dir: Option<NavDir>,
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
    pub fn update(&mut self, ctx: &egui::Context) {
        self.detect_startup_gamepads();
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
                    let name = self.gamepad_name(ev.id);
                    if self.known_gamepads.insert(ev.id) {
                        tracing::info!(
                            target: "vertexlauncher/gamepad",
                            gamepad_name = %name,
                            "Gamepad connected."
                        );
                        notification::info!("gamepad", "Gamepad connected: {name}");
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
                        nav_actions.push((ev.id, NavDir::Backward))
                    }
                    Button::DPadDown | Button::DPadRight => {
                        nav_actions.push((ev.id, NavDir::Forward))
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
                EventType::AxisChanged(axis, value, _) => {
                    self.handle_axis(ev.id, axis, value, &mut nav_actions);
                }
                _ => {}
            }
        }

        // --- Hold-to-navigate ---
        let now = Instant::now();

        for (id, dir) in nav_actions {
            Self::fire_nav(ctx, dir);
            if let Some(state) = self.device_states.get_mut(&id) {
                state.held_dir = Some(dir);
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
    }

    /// Handle a left-stick or right-stick axis change.
    fn handle_axis(
        &mut self,
        gamepad_id: GamepadId,
        axis: Axis,
        value: f32,
        nav_actions: &mut Vec<(GamepadId, NavDir)>,
    ) {
        let Some(state) = self.device_states.get_mut(&gamepad_id) else {
            return;
        };
        match axis {
            Axis::LeftStickX => {
                if !state.stick_x_seen {
                    state.stick_x_seen = true;
                    state.stick_x = value;
                    state.stick_x_armed = value.abs() < STICK_DEADZONE;
                    return;
                }

                let was_right = state.stick_x > STICK_THRESHOLD;
                let was_left = state.stick_x < -STICK_THRESHOLD;
                state.stick_x = value;
                let now_right = value > STICK_THRESHOLD;
                let now_left = value < -STICK_THRESHOLD;
                let released = was_right || was_left;
                let center = value.abs() < STICK_DEADZONE;

                if center {
                    state.stick_x_armed = true;
                }

                if state.stick_x_armed && !was_right && now_right {
                    nav_actions.push((gamepad_id, NavDir::Forward));
                    state.stick_x_armed = false;
                } else if state.stick_x_armed && !was_left && now_left {
                    nav_actions.push((gamepad_id, NavDir::Backward));
                    state.stick_x_armed = false;
                } else if center && released {
                    Self::clear_hold(state);
                }
            }
            Axis::LeftStickY => {
                if !state.stick_y_seen {
                    state.stick_y_seen = true;
                    state.stick_y = value;
                    state.stick_y_armed = value.abs() < STICK_DEADZONE;
                    return;
                }

                // gilrs Y: positive = up on most controllers → backward in tab order
                let was_up = state.stick_y > STICK_THRESHOLD;
                let was_down = state.stick_y < -STICK_THRESHOLD;
                state.stick_y = value;
                let now_up = value > STICK_THRESHOLD;
                let now_down = value < -STICK_THRESHOLD;
                let released = was_up || was_down;
                let center = value.abs() < STICK_DEADZONE;

                if center {
                    state.stick_y_armed = true;
                }

                if state.stick_y_armed && !was_up && now_up {
                    nav_actions.push((gamepad_id, NavDir::Backward));
                    state.stick_y_armed = false;
                } else if state.stick_y_armed && !was_down && now_down {
                    nav_actions.push((gamepad_id, NavDir::Forward));
                    state.stick_y_armed = false;
                } else if center && released {
                    Self::clear_hold(state);
                }
            }
            Axis::RightStickY => {
                state.right_stick_y = value;
            }
            Axis::RightStickX => {
                state.right_stick_x = value;
            }
            _ => {}
        }
    }

    fn clear_hold(state: &mut DeviceState) {
        state.held_dir = None;
        state.held_since = None;
        state.last_repeat = None;
    }

    fn detect_startup_gamepads(&mut self) {
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
                        let name = self.gamepad_name(*id);
                        tracing::info!(
                            target: "vertexlauncher/gamepad",
                            gamepad_name = %name,
                            "Detected connected controller at startup."
                        );
                        notification::info!("gamepad", "Detected gamepad: {name}");
                    }
                }
            }
            self.startup_scan_complete = true;
            return;
        }

        for id in connected_ids {
            if self.known_gamepads.insert(id) {
                let name = self.gamepad_name(id);
                tracing::info!(
                    target: "vertexlauncher/gamepad",
                    gamepad_name = %name,
                    "Detected connected controller after startup."
                );
                notification::info!("gamepad", "Detected gamepad: {name}");
            }
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
        self.gilrs
            .connected_gamepad(id)
            .map(|gamepad| gamepad.name().trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("Controller {id:?}"))
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
