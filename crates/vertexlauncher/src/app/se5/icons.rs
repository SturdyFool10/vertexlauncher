//! The launcher's Tabler SVG icons on the engine's SVG pipeline. The engine loads SVGs from files and tints them
//! by multiplying, so each icon is written once to the cache folder with `currentColor` set to white; drawing
//! passes the theme colour as the element's tint.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use launcher_ui::assets;
use vertex_shell::sturdy::glam::Vec4;
use vertex_shell::sturdy::ui::{ElementDesc, Sizing, Ui};

macro_rules! icons {
    ($($variant:ident => ($file:literal, $bytes:expr)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon { $($variant),+ }

        impl Icon {
            fn file(self) -> &'static str {
                match self { $(Self::$variant => $file),+ }
            }

            fn svg(self) -> &'static [u8] {
                match self { $(Self::$variant => $bytes),+ }
            }
        }
    };
}

icons! {
    AntennaOff => ("antenna-bars-off", assets::ANTENNA_BARS_OFF_SVG),
    Antenna1 => ("antenna-bars-1", assets::ANTENNA_BARS_1_SVG),
    Antenna2 => ("antenna-bars-2", assets::ANTENNA_BARS_2_SVG),
    Antenna3 => ("antenna-bars-3", assets::ANTENNA_BARS_3_SVG),
    Antenna4 => ("antenna-bars-4", assets::ANTENNA_BARS_4_SVG),
    Antenna5 => ("antenna-bars-5", assets::ANTENNA_BARS_5_SVG),
    ArrowsMaximize => ("arrows-maximize", assets::ARROWS_MAXIMIZE_SVG),
    ArrowsMinimize => ("arrows-minimize", assets::ARROWS_MINIMIZE_SVG),
    Check => ("check", assets::CHECK_SVG),
    ChevronDown => ("chevron-down", assets::CHEVRON_DOWN_SVG),
    ChevronRight => ("chevron-right", assets::CHEVRON_RIGHT_SVG),
    ChevronUp => ("chevron-up", assets::CHEVRON_UP_SVG),
    Copy => ("copy", assets::COPY_SVG),
    Discover => ("discover", assets::DISCOVER_SVG),
    Download => ("download", assets::DOWNLOAD_SVG),
    Edit => ("edit", assets::EDIT_SVG),
    Error => ("error", assets::ERROR_SVG),
    Folder => ("folder", assets::FOLDER_SVG),
    Home => ("home", assets::HOME_SVG),
    Info => ("info-circle", assets::INFO_CIRCLE_SVG),
    Legal => ("legal", assets::LEGAL_SVG),
    Library => ("library", assets::LIBRARY_SVG),
    Log => ("log", assets::LOG_SVG),
    Minus => ("minus", assets::MINUS_SVG),
    Play => ("play", assets::PLAY_SVG),
    Plus => ("plus", assets::PLUS_SVG),
    Refresh => ("refresh", assets::REFRESH_SVG),
    Restore => ("restore", assets::RESTORE_SVG),
    Settings => ("settings", assets::SETTINGS_SVG),
    Share => ("share", assets::SHARE_SVG),
    Skins => ("skin_selector", assets::SKIN_SELECTOR_SVG),
    Star => ("star", assets::STAR_SVG),
    Stop => ("stop", assets::STOP_SVG),
    Steam => ("brand-steam", assets::STEAM_SVG),
    Terminal => ("terminal", assets::TERMINAL_SVG),
    Terminal2 => ("terminal-2", assets::TERMINAL_2_SVG),
    Trash => ("trash-x", assets::TRASH_X_SVG),
    User => ("user", assets::USER_SVG),
    UsersGroup => ("users-group", assets::USERS_GROUP_SVG),
    Warn => ("warn", assets::WARN_SVG),
    Close => ("x", assets::X_SVG),
}

fn icon_dir() -> PathBuf {
    app_paths::cache_root().join("icons")
}

/// Writes the white-on-transparent copy of `icon` if it is not there yet (or the bundled icon changed) and
/// returns its path.
///
/// The check and the write run once per icon on the launcher runtime, never on the UI thread; until the file is ready
/// this returns `None` and the icon is skipped for a frame.
fn ensure_file(icon: Icon, turn: u8) -> Option<PathBuf> {
    enum State {
        Pending,
        Ready(PathBuf),
        Failed,
    }
    static STATES: OnceLock<Mutex<HashMap<(Icon, u8), State>>> = OnceLock::new();
    let mut states = STATES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match states.get(&(icon, turn)) {
        Some(State::Ready(path)) => return Some(path.clone()),
        Some(State::Pending | State::Failed) => return None,
        None => {}
    }
    states.insert((icon, turn), State::Pending);
    drop(states);
    launcher_runtime::spawn_blocking_detached(move || {
        let path = if turn == 0 {
            icon_dir().join(format!("{}.svg", icon.file()))
        } else {
            icon_dir().join(format!("{}-t{turn}.svg", icon.file()))
        };
        let mut white = String::from_utf8_lossy(icon.svg()).replace("currentColor", "#ffffff");
        if turn != 0
            && let (Some(open_end), Some(close)) = (white.find('>'), white.rfind("</svg>"))
        {
            // The viewBox is 24 wide, so the icon turns about (12, 12).
            let degrees = f32::from(turn) * TURN_STEP_DEGREES;
            white.insert_str(close, "</g>");
            white.insert_str(open_end + 1, &format!("<g transform=\"rotate({degrees} 12 12)\">"));
        }
        let white = white.into_bytes();
        let written = if std::fs::read(&path).ok().as_deref() == Some(white.as_slice()) {
            true
        } else {
            std::fs::create_dir_all(icon_dir()).is_ok() && std::fs::write(&path, &white).is_ok()
        };
        let state = if written {
            State::Ready(path)
        } else {
            State::Failed
        };
        STATES
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert((icon, turn), state);
        vertex_shell::wake();
    });
    None
}

/// Draws `icon` `size` pixels square in `color`. `supersample` renders it larger and scales it down, which is what
/// the "Icon anti-aliasing" setting controls.
pub fn draw(ui: &mut Ui<'_>, icon: Icon, size: f32, color: Vec4, supersample: u32) {
    draw_turned(ui, icon, 0.0, size, color, supersample);
}

/// Starts writing the rotated copies of `icon` between `from` and `to` degrees, so an animation through that range does
/// not skip frames while their files are made. Cheap to call every frame.
pub fn warm_turns(icon: Icon, from: f32, to: f32) {
    let (low, high) = (from.min(to), from.max(to));
    let first = (low / TURN_STEP_DEGREES).floor() as u32;
    let last = (high / TURN_STEP_DEGREES).ceil() as u32;
    for step in first..=last {
        let _ = ensure_file(icon, (step % 48) as u8);
    }
}

/// Degrees between the pre-rotated copies of an icon that [`draw_turned`] picks from.
const TURN_STEP_DEGREES: f32 = 7.5;

/// Like [`draw`], rotated clockwise by `degrees` (0..360). The engine cannot rotate an element, so each rotation is
/// its own SVG file, rounded to the nearest 7.5 degrees.
pub fn draw_turned(
    ui: &mut Ui<'_>,
    icon: Icon,
    degrees: f32,
    size: f32,
    color: Vec4,
    supersample: u32,
) {
    let turn = ((degrees.rem_euclid(360.0) / TURN_STEP_DEGREES).round() as u32 % 48) as u8;
    let slot = ElementDesc::new()
        .width(Sizing::Fixed(size))
        .height(Sizing::Fixed(size));
    // Until the icon is ready its slot stays empty, so the layout does not move when it appears.
    let Some(path) = ensure_file(icon, turn) else {
        drop(ui.element(&slot));
        return;
    };
    // Rounded up to a multiple of 4: the engine block-compresses these textures, and WebGPU rejects a BC7 texture
    // whose size is not a whole number of 4x4 blocks (Vulkan tolerates it), which left odd-sized icons invisible.
    let target = ((size * supersample.max(1) as f32 / 4.0).ceil() * 4.0).max(4.0);
    let Ok(texture) = ui.load_svg(&path.to_string_lossy(), target) else {
        drop(ui.element(&slot));
        return;
    };
    // A texture that is not on the GPU yet is drawn as a solid block of the tint colour. So the icon waits until the
    // engine has resources for it, and at least one frame after it was created, then replaces the empty slot.
    let on_gpu = ui.engine().assets().rhi_texture(texture).is_some();
    if !texture_seen(&path, target) || !on_gpu {
        drop(ui.element(&slot));
        vertex_shell::wake_after(std::time::Duration::from_millis(16));
        return;
    }
    ui.svg(
        &ElementDesc::new()
            .width(Sizing::Fixed(size))
            .height(Sizing::Fixed(size))
            .background(color),
        texture,
    );
}

/// The textures (file, raster size) that an earlier frame has already asked the engine for. Returns whether this one
/// was among them, and records it.
fn texture_seen(path: &std::path::Path, target: f32) -> bool {
    let mut seen = SEEN_TEXTURES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    !seen.insert((path.to_path_buf(), target.to_bits()))
}

static SEEN_TEXTURES: OnceLock<Mutex<std::collections::HashSet<(PathBuf, u32)>>> = OnceLock::new();

/// Call after the engine's SVG cache is cleared: every texture is gone, so each icon starts over as not yet loaded.
pub fn forget_textures() {
    SEEN_TEXTURES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}
