//! The launcher's Tabler SVG icons on the engine's SVG pipeline. The engine tints an SVG by multiplying, so each icon is
//! made once with `currentColor` set to white and handed to the engine as text (nothing is written to disk); drawing
//! passes the theme colour as the element's tint.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

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

/// The name the engine knows an icon (turned by `turn` steps, see [`TURN_STEP_DEGREES`]) by.
fn icon_name(icon: Icon, turn: u8) -> String {
    if turn == 0 {
        icon.file().to_owned()
    } else {
        format!("{}-t{turn}", icon.file())
    }
}

/// The white-on-transparent SVG text of `icon` turned by `turn` steps, made the first time it is asked for.
fn svg_text(icon: Icon, turn: u8) -> Arc<[u8]> {
    static TEXTS: OnceLock<Mutex<HashMap<(Icon, u8), Arc<[u8]>>>> = OnceLock::new();
    let mut texts = TEXTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(texts.entry((icon, turn)).or_insert_with(|| {
        let mut white = String::from_utf8_lossy(icon.svg()).replace("currentColor", "#ffffff");
        if turn != 0
            && let (Some(open_end), Some(close)) = (white.find('>'), white.rfind("</svg>"))
        {
            // The viewBox is 24 wide, so the icon turns about (12, 12).
            let degrees = f32::from(turn) * TURN_STEP_DEGREES;
            white.insert_str(close, "</g>");
            white.insert_str(open_end + 1, &format!("<g transform=\"rotate({degrees} 12 12)\">"));
        }
        Arc::from(white.into_bytes())
    }))
}

/// Draws `icon` `size` pixels square in `color`. `supersample` renders it larger and scales it down, which is what
/// the "Icon anti-aliasing" setting controls.
pub fn draw(ui: &mut Ui<'_>, icon: Icon, size: f32, color: Vec4, supersample: u32) {
    draw_turned(ui, icon, 0.0, size, color, supersample);
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
    let name = icon_name(icon, turn);
    let text = svg_text(icon, turn);
    // Rounded up to a multiple of 4: the engine block-compresses these textures, and WebGPU rejects a BC7 texture
    // whose size is not a whole number of 4x4 blocks (Vulkan tolerates it), which left odd-sized icons invisible.
    let target = ((size * supersample.max(1) as f32 / 4.0).ceil() * 4.0).max(4.0);
    let Ok(texture) = ui.load_svg_data(&name, &text, target) else {
        drop(ui.element(&slot));
        return;
    };
    ui.svg(
        &ElementDesc::new()
            .width(Sizing::Fixed(size))
            .height(Sizing::Fixed(size))
            .background(color),
        texture,
    );
}
