//! The launcher's Tabler SVG icons on the engine's SVG pipeline. The engine tints an SVG by multiplying, so each icon is
//! made once with `currentColor` set to white and handed to the engine as text (nothing is written to disk); drawing
//! passes the theme colour as the element's tint. A turned icon is the same single SVG rotated when it is drawn.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use launcher_ui::assets;
use vertex_shell::sturdy::glam::{Vec2, Vec4};
use vertex_shell::sturdy::ui::{ElementDesc, Sizing, Ui};
use vertex_shell::sturdy::ui_effects::UiEffect;

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
    Browser => ("browser", assets::BROWSER_SVG),
    Check => ("check", assets::CHECK_SVG),
    ChevronDown => ("chevron-down", assets::CHEVRON_DOWN_SVG),
    Copy => ("copy", assets::COPY_SVG),
    Dice => ("dice", assets::DICE_SVG),
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
    SquareRotated => ("square-rotated", assets::SQUARE_ROTATED_SVG),
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

/// The white-on-transparent SVG text of `icon`, made the first time it is asked for.
fn svg_text(icon: Icon) -> Arc<[u8]> {
    static TEXTS: OnceLock<Mutex<HashMap<Icon, Arc<[u8]>>>> = OnceLock::new();
    let mut texts = TEXTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        texts
            .entry(icon)
            .or_insert_with(|| Arc::from(String::from_utf8_lossy(icon.svg()).replace("currentColor", "#ffffff").into_bytes())),
    )
}

/// Draws `icon` `size` pixels square in `color`. `supersample` renders it larger and scales it down, which is what
/// the "Icon anti-aliasing" setting controls.
pub fn draw(ui: &mut Ui<'_>, icon: Icon, size: f32, color: Vec4, supersample: u32) {
    draw_turned(ui, icon, 0.0, size, color, supersample);
}

/// Like [`draw`], rotated clockwise by `degrees`. The icon is its one original SVG; the rotation is the engine's
/// transform effect on the element, so no turned copy of the icon is ever made.
pub fn draw_turned(
    ui: &mut Ui<'_>,
    icon: Icon,
    degrees: f32,
    size: f32,
    color: Vec4,
    supersample: u32,
) {
    let slot = ElementDesc::new()
        .width(Sizing::Fixed(size))
        .height(Sizing::Fixed(size));
    let text = svg_text(icon);
    // Rounded up to a multiple of 4: the engine block-compresses these textures, and WebGPU rejects a BC7 texture
    // whose size is not a whole number of 4x4 blocks (Vulkan tolerates it), which left odd-sized icons invisible.
    let target = ((size * supersample.max(1) as f32 / 4.0).ceil() * 4.0).max(4.0);
    let Ok(texture) = ui.load_svg_data(icon.file(), &text, target) else {
        drop(ui.element(&slot));
        return;
    };
    let mut element = ElementDesc::new()
        .width(Sizing::Fixed(size))
        .height(Sizing::Fixed(size))
        .background(color);
    let degrees = degrees.rem_euclid(360.0);
    if degrees > 0.01 {
        // The outset is the room a turned square needs past its box (a corner reaches 0.21 of its side beyond it); without
        // one the engine would give the effect the whole surface to render into. Frames already built keep drawing with
        // the effect once this handle is dropped.
        let room = (size * 0.25).ceil();
        element = element.effect(
            UiEffect::transform(Vec2::ZERO, degrees.to_radians(), Vec2::ONE, Vec2::splat(0.5), [room; 4]).ok(),
        );
    }
    ui.svg(&element, texture);
}
