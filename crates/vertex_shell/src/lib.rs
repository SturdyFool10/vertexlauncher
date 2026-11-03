//! The SturdyEngine5 host for Vertex: opens the window, owns the engine loop and exposes the pieces the rest of the
//! launcher needs (a way to wake the UI from any thread, and the per-frame draw hook).
//!
//! The UI is immediate-mode. The caller hands [`run`] one draw closure that rebuilds the whole frame, and the engine
//! draws continuously ([`sturdy::app::FrameMode::Continuous`]): as fast as the GPU allows, or exactly at the frame
//! limit when one is set. `wake` is therefore only an extra nudge.

mod logging;
pub mod palette;
pub mod settings_apply;
pub mod watchdog;

pub use settings_apply::Applied;
use settings_apply::Appliers;

use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use config::{Config, SettingsDiff, SettingsDomain};

use sturdy::app::{FrameMode, RedrawHandle};
use sturdy::prelude::*;
use sturdy::ui::{self, Ui};

pub use sturdy;

static REDRAW: OnceLock<RedrawHandle> = OnceLock::new();

/// Wakes the UI so it renders another frame. Safe from any thread; a no-op until the engine has started.
pub fn wake() {
    if let Some(handle) = REDRAW.get() {
        handle.request_redraw();
    }
}

/// Wakes the UI after `delay` (the earliest pending request wins).
pub fn wake_after(delay: std::time::Duration) {
    if let Some(handle) = REDRAW.get() {
        handle.request_redraw_after(delay);
    }
}

/// Window-level startup options. Everything here can also change at runtime through the settings appliers.
#[derive(Clone, Debug)]
pub struct ShellConfig {
    pub title: String,
    pub app_id: String,
    pub size: (u32, u32),
    pub decorated: bool,
    pub transparent: bool,
}

impl ShellConfig {
    pub fn new(title: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            app_id: app_id.into(),
            size: (1280, 800),
            decorated: false,
            transparent: true,
        }
    }
}

/// How the UI hands config changes to the engine thread and learns what the appliers made of them.
///
/// `submit` is cheap and callable from any thread (including the draw hook); the next frame applies it. Only the
/// latest pending config matters, so rapid changes coalesce.
#[derive(Default)]
pub struct SettingsBridge {
    pending: Mutex<Option<Config>>,
    results: Mutex<Vec<(SettingsDomain, Applied)>>,
}

impl SettingsBridge {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn submit(&self, config: Config) {
        *lock(&self.pending) = Some(config);
        wake();
    }

    /// Failures (the caller restores the old value and notifies) and degradations (it just notifies), oldest first.
    pub fn take_results(&self) -> Vec<(SettingsDomain, Applied)> {
        std::mem::take(&mut *lock(&self.results))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The mouse cursor the UI wants over the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerCursor {
    /// The ordinary arrow.
    Arrow = 0,
    /// The pointing hand.
    Hand = 1,
    /// The text-editing I-beam.
    Text = 2,
}

/// Whether the window fills the screen, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fullscreen {
    Off = 0,
    Borderless = 1,
    Exclusive = 2,
}

const NO_REQUEST: u8 = u8::MAX;
static WANTED_FULLSCREEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(NO_REQUEST);
static CURRENT_FULLSCREEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Asks for the window to go fullscreen, or back to a window; the shell does it on the next frame.
pub fn request_fullscreen(mode: Fullscreen) {
    WANTED_FULLSCREEN.store(mode as u8, std::sync::atomic::Ordering::Relaxed);
}

/// What the window is now, as of the last frame.
pub fn fullscreen() -> Fullscreen {
    match CURRENT_FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed) {
        1 => Fullscreen::Borderless,
        2 => Fullscreen::Exclusive,
        _ => Fullscreen::Off,
    }
}

static F11_PRESSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ESCAPE_PRESSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether F11 was pressed since this was last asked. Read in the shell's own frame, which runs every engine tick; the
/// UI is only drawn when it has something to show, and a key pressed in a tick it skips would be missed there.
pub fn take_fullscreen_key() -> bool {
    F11_PRESSED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Whether Escape was pressed since this was last asked (see [`take_fullscreen_key`]).
pub fn take_escape_key() -> bool {
    ESCAPE_PRESSED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

static HANDOFF_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static GAME_RUNNING: std::sync::OnceLock<fn() -> bool> = std::sync::OnceLock::new();

/// Installs the question "is a game running?" that the hand-off (see [`set_handoff`]) asks every frame.
pub fn install_game_probe(game_running: fn() -> bool) {
    let _ = GAME_RUNNING.set(game_running);
}

/// Turns the hand-off on or off. While it is on, the window is hidden whenever a game is running and shown again when
/// none is, so the game can take the screen (a Steam Deck's game mode keeps the first window in front until it is gone).
/// It is decided in the shell's own frame, which keeps running while the window is hidden.
pub fn set_handoff(enabled: bool) {
    HANDOFF_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

static WANTED_CURSOR: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Sets the cursor the window should show. The UI engine only works out what it would like; nothing applies that to
/// the window, so the shell does it from here once a frame (the engine skips the call when it is unchanged).
pub fn set_cursor(cursor: PointerCursor) {
    WANTED_CURSOR.store(cursor as u8, std::sync::atomic::Ordering::Relaxed);
}

struct Shell<F> {
    draw: Option<F>,
    bridge: Arc<SettingsBridge>,
    /// The config the engine currently reflects; `None` until the first frame applies the initial one.
    applied: Option<Config>,
    appliers: Option<Appliers>,
    effect_polls_left: u8,
    /// The window is hidden because a game is running.
    handed_off: bool,
}

impl<F> Shell<F> {
    fn apply_pending(&mut self, engine: &mut Engine<'_>, window_id: u64) {
        let appliers = self.appliers.get_or_insert_with(|| Appliers::new(window_id));
        if let Some(new) = lock(&self.bridge.pending).take() {
            let diff = match &self.applied {
                Some(old) => old.diff(&new),
                None => SettingsDiff::all(),
            };
            if diff.contains(SettingsDomain::WindowEffect) {
                self.effect_polls_left = 40;
            }
            let results = appliers.apply(engine, &new, diff);
            if !results.is_empty() {
                lock(&self.bridge.results).extend(results);
            }
            self.applied = Some(new);
        }
        if appliers.poll_hdr_metadata(engine) {
            wake_after(Duration::from_millis(50));
        }
        if self.effect_polls_left > 0 {
            // The platform reports blur's verdict a frame or two after the request.
            self.effect_polls_left -= 1;
            if let Some(verdict) = appliers.poll_effect_status(engine) {
                self.effect_polls_left = 0;
                if verdict != Applied::Ok {
                    lock(&self.bridge.results).push((SettingsDomain::WindowEffect, verdict));
                }
            } else if self.effect_polls_left > 0 {
                wake_after(Duration::from_millis(50));
            }
        }
    }
}

impl<F> GameLogic for Shell<F>
where
    F: FnMut(&mut Ui<'_>) + Send + 'static,
{
    fn init(&mut self, engine: &mut Engine<'_>) -> Result<(), Error> {
        watchdog::step("engine started (device and window created)");
        engine.set_frame_mode(FrameMode::Continuous);
        let _ = REDRAW.set(engine.redraw_handle());
        if let Some(mut draw) = self.draw.take() {
            ui::install_draw_hook(move |ui| {
                watchdog::frame_begin();
                draw(ui);
                watchdog::frame_end();
            });
        }
        watchdog::step("draw hook installed");
        Ok(())
    }

    fn frame(&mut self, engine: &mut Engine<'_>, frame: &Frame) -> Option<FrameDescription> {
        watchdog::engine_frame();
        self.apply_pending(engine, frame.window_id);
        let icon = match WANTED_CURSOR.load(std::sync::atomic::Ordering::Relaxed) {
            1 => sturdy::render::CursorIcon::Pointer,
            2 => sturdy::render::CursorIcon::Text,
            _ => sturdy::render::CursorIcon::Default,
        };
        {
            let input = engine.input();
            let mut pressed = false;
            if input.key_just_pressed(sturdy::Key::F11) {
                F11_PRESSED.store(true, std::sync::atomic::Ordering::Relaxed);
                pressed = true;
            }
            if input.key_just_pressed(sturdy::Key::Escape) {
                ESCAPE_PRESSED.store(true, std::sync::atomic::Ordering::Relaxed);
                pressed = true;
            }
            if pressed {
                // Make sure the UI is drawn to act on it.
                wake();
            }
        }
        let surface = sturdy::render::SurfaceHandle::from_window_id(frame.window_id);
        engine.set_cursor_icon(surface, icon);
        let wanted = WANTED_FULLSCREEN.swap(NO_REQUEST, std::sync::atomic::Ordering::Relaxed);
        if wanted != NO_REQUEST {
            engine.set_window_mode(
                surface,
                match wanted {
                    1 => sturdy::WindowMode::BorderlessFullscreen,
                    2 => sturdy::WindowMode::ExclusiveFullscreen,
                    _ => sturdy::WindowMode::Windowed,
                },
            );
        }
        let game_running = GAME_RUNNING.get().is_some_and(|probe| probe())
            && HANDOFF_ENABLED.load(std::sync::atomic::Ordering::Relaxed);
        if game_running != self.handed_off {
            if let Some(mut window) = engine.primary_window_control() {
                if game_running {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                    let _ = window.raise();
                    let _ = window.focus();
                }
                self.handed_off = game_running;
            }
        }
        if let Some(mut window) = engine.primary_window_control() {
            let now = match window.fullscreen_mode() {
                sturdy::WindowMode::BorderlessFullscreen => 1,
                sturdy::WindowMode::ExclusiveFullscreen => 2,
                _ => 0,
            };
            CURRENT_FULLSCREEN.store(now, std::sync::atomic::Ordering::Relaxed);
        }
        Some(FrameDescription::ui_only())
    }
}

/// Runs the launcher window until it closes. Call after the CLI, webview-helper and single-instance checks.
///
/// `initial` is applied on the first frame, so every setting takes effect without a restart, including the ones the
/// window itself is created with.
pub fn run<F>(config: ShellConfig, initial: Config, bridge: Arc<SettingsBridge>, draw: F) -> ExitCode
where
    F: FnMut(&mut Ui<'_>) + Send + 'static,
{
    watchdog::start();
    watchdog::step("shell run() entered");
    logging::forward_engine_logs();
    let translucent = initial.window_transparency().is_translucent();
    // Ask for the saved API and GPU up front: the engine resolves the request before it creates the device, so it comes
    // up once on the right one. Where the request cannot be met it starts on its default, and the first-frame applier
    // (a no-op when the device already matches) reports whatever is left.
    let device = settings_apply::initial_device_request(&initial);
    bridge.submit(initial);
    watchdog::step("device request chosen, starting the engine");
    let runtime = RuntimeConfig::new(config.title)
        .device_request(device)
        .app_id(config.app_id)
        .size(config.size.0, config.size.1)
        .decorated(config.decorated)
        // The window is created able to become translucent; the applier then sets the real state live.
        .transparent(config.transparent || translucent)
        .transparent_composition(config.transparent || translucent);
    let code = sturdy::run(runtime, Shell { draw: Some(draw), bridge, applied: None, appliers: None, effect_polls_left: 0, handed_off: false });
    watchdog::stop();
    code
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    /// The launcher's waits ride on the engine's timer; it has to work with no window or GPU session.
    #[test]
    fn the_engine_timer_sleeps_without_a_session() {
        let started = Instant::now();
        let slept = sturdy::net::block_on(sturdy::net::sleep(Duration::from_millis(80)));
        assert!(slept.is_ok());
        assert!(started.elapsed() >= Duration::from_millis(70));
    }
}
