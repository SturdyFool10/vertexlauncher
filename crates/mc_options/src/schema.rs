//! Every known Java Edition `options.txt` key, with type, range, default and the versions
//! it exists in. Sourced from the Minecraft Wiki's options.txt page and checked against real
//! 26.3 option files; unknown keys (mods, future versions) are handled generically.

use crate::version::Ver;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    Video,
    Audio,
    Controls,
    Keybinds,
    Chat,
    Accessibility,
    Skin,
    Language,
    Online,
    /// Bookkeeping the game manages itself (shown only under Advanced).
    Internal,
}

impl Group {
    pub const ALL: [Group; 10] = [
        Group::Video,
        Group::Audio,
        Group::Controls,
        Group::Keybinds,
        Group::Chat,
        Group::Accessibility,
        Group::Skin,
        Group::Language,
        Group::Online,
        Group::Internal,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Group::Video => vertex_i18n::src!("Video"),
            Group::Audio => vertex_i18n::src!("Music & Sounds"),
            Group::Controls => vertex_i18n::src!("Controls"),
            Group::Keybinds => vertex_i18n::src!("Key Binds"),
            Group::Chat => vertex_i18n::src!("Chat"),
            Group::Accessibility => vertex_i18n::src!("Accessibility"),
            Group::Skin => vertex_i18n::src!("Skin"),
            Group::Language => vertex_i18n::src!("Language & Fonts"),
            Group::Online => vertex_i18n::src!("Online & Misc"),
            Group::Internal => vertex_i18n::src!("Advanced"),
        }
    }

    pub const fn slug(self) -> &'static str {
        match self {
            Group::Video => "video",
            Group::Audio => "audio",
            Group::Controls => "controls",
            Group::Keybinds => "keybinds",
            Group::Chat => "chat",
            Group::Accessibility => "accessibility",
            Group::Skin => "skin",
            Group::Language => "language",
            Group::Online => "online",
            Group::Internal => "internal",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    Float {
        min: f64,
        max: f64,
    },
    /// String enum: `(stored value, label)`.
    Choice(&'static [(&'static str, &'static str)]),
    /// Integer enum: `(stored value, label)`.
    IntChoice(&'static [(i64, &'static str)]),
    Text,
    /// `["a","b"]` list.
    List,
    /// Key or mouse button, in the version's own format.
    Keybind,
}

/// How a numeric value is shown to the user.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Display {
    Plain,
    /// Shown as `value * 100` percent.
    Percent,
    /// Shown as `offset + value * scale` degrees (FOV is stored normalized).
    Degrees {
        offset: f64,
        scale: f64,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct OptionDef {
    pub key: &'static str,
    pub label: &'static str,
    pub group: Group,
    pub kind: Kind,
    pub default: &'static str,
    /// First version with this key/format (inclusive).
    pub since: Ver,
    /// First version without it (exclusive).
    pub until: Ver,
    /// String values are wrapped in quotes in this version's file format.
    pub quoted: bool,
    pub display: Display,
    /// Tied to this machine or session; never synced between instances.
    pub machine_specific: bool,
    /// Bookkeeping the game or launcher manages; never shown in the settings UI. The value is
    /// still preserved in the file.
    pub hidden: bool,
    pub note: &'static str,
}

impl OptionDef {
    const fn new(
        key: &'static str,
        label: &'static str,
        group: Group,
        kind: Kind,
        default: &'static str,
    ) -> Self {
        Self {
            key,
            label,
            group,
            kind,
            default,
            since: Ver::OLDEST,
            until: Ver::NEWEST,
            quoted: false,
            display: Display::Plain,
            machine_specific: false,
            hidden: false,
            note: "",
        }
    }

    const fn since(mut self, major: u16, minor: u16, patch: u16) -> Self {
        self.since = Ver(major, minor, patch);
        self
    }

    /// Exclusive upper bound.
    const fn until(mut self, major: u16, minor: u16, patch: u16) -> Self {
        self.until = Ver(major, minor, patch);
        self
    }

    const fn quoted(mut self) -> Self {
        self.quoted = true;
        self
    }

    const fn display(mut self, display: Display) -> Self {
        self.display = display;
        self
    }

    const fn machine(mut self) -> Self {
        self.machine_specific = true;
        self
    }

    const fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    const fn note(mut self, note: &'static str) -> Self {
        self.note = note;
        self
    }

    pub fn applies_to(&self, version: Option<Ver>) -> bool {
        match version {
            Some(v) => v >= self.since && v < self.until,
            // Unknown version: consider only what is current.
            None => self.until == Ver::NEWEST,
        }
    }
}

use Display::Percent;
use Group::*;

const OFF_ON_NARRATOR: &[(i64, &str)] = &[(0, vertex_i18n::src!("Off")), (1, vertex_i18n::src!("All")), (2, vertex_i18n::src!("Chat")), (3, vertex_i18n::src!("System"))];
const PARTICLES: &[(i64, &str)] = &[(0, vertex_i18n::src!("All")), (1, vertex_i18n::src!("Decreased")), (2, vertex_i18n::src!("Minimal"))];
const CHAT_VISIBILITY: &[(i64, &str)] = &[(0, vertex_i18n::src!("Shown")), (1, vertex_i18n::src!("Commands Only")), (2, vertex_i18n::src!("Hidden"))];
const ATTACK_INDICATOR: &[(i64, &str)] = &[(0, vertex_i18n::src!("Off")), (1, vertex_i18n::src!("Crosshair")), (2, vertex_i18n::src!("Hotbar"))];
const GRAPHICS_MODE: &[(i64, &str)] = &[(0, vertex_i18n::src!("Fast")), (1, vertex_i18n::src!("Fancy")), (2, vertex_i18n::src!("Fabulous!"))];
const GRAPHICS_PRESET: &[(&str, &str)] = &[
    ("fast", vertex_i18n::src!("Fast")),
    ("fancy", vertex_i18n::src!("Fancy")),
    ("fabulous", vertex_i18n::src!("Fabulous!")),
    ("custom", vertex_i18n::src!("Custom")),
];
const CLOUDS: &[(&str, &str)] = &[("false", vertex_i18n::src!("Off")), ("fast", vertex_i18n::src!("Fast")), ("true", vertex_i18n::src!("Fancy"))];
const AO_LEVELS: &[(i64, &str)] = &[(0, vertex_i18n::src!("Off")), (1, vertex_i18n::src!("Minimum")), (2, vertex_i18n::src!("Maximum"))];
const CHUNK_UPDATES: &[(i64, &str)] = &[(0, vertex_i18n::src!("None")), (1, vertex_i18n::src!("Player Affected")), (2, vertex_i18n::src!("Nearby"))];
const HAND: &[(&str, &str)] = &[("right", vertex_i18n::src!("Right")), ("left", vertex_i18n::src!("Left"))];
const BACKENDS: &[(&str, &str)] = &[
    ("default", vertex_i18n::src!("Default")),
    ("opengl", vertex_i18n::src!("OpenGL")),
    ("vulkan", vertex_i18n::src!("Vulkan")),
];
const INACTIVITY: &[(&str, &str)] = &[("afk", vertex_i18n::src!("When AFK")), ("minimized", vertex_i18n::src!("When Minimized"))];
const MUSIC_TOAST: &[(&str, &str)] = &[
    ("never", vertex_i18n::src!("Never")),
    ("pause", vertex_i18n::src!("On Pause")),
    ("pause_and_toast", vertex_i18n::src!("Pause and Toast")),
];
const MUSIC_FREQUENCY: &[(&str, &str)] = &[
    (vertex_i18n::src!("DEFAULT"), vertex_i18n::src!("Default")),
    (vertex_i18n::src!("FREQUENT"), vertex_i18n::src!("Frequent")),
    (vertex_i18n::src!("CONSTANT"), vertex_i18n::src!("Constant")),
];
const TUTORIAL: &[(&str, &str)] = &[
    ("movement", vertex_i18n::src!("Movement")),
    ("find_tree", vertex_i18n::src!("Find Tree")),
    ("punch_tree", vertex_i18n::src!("Punch Tree")),
    ("open_inventory", vertex_i18n::src!("Open Inventory")),
    ("craft_planks", vertex_i18n::src!("Craft Planks")),
    ("none", vertex_i18n::src!("None")),
];
const SHARE_PRESENCE: &[(&str, &str)] = &[("all", vertex_i18n::src!("All")), ("limited", vertex_i18n::src!("Limited")), ("none", vertex_i18n::src!("None"))];
const DIFFICULTY: &[(i64, &str)] = &[(0, vertex_i18n::src!("Peaceful")), (1, vertex_i18n::src!("Easy")), (2, vertex_i18n::src!("Normal")), (3, vertex_i18n::src!("Hard"))];
const FPS_LIMIT_LEGACY: &[(i64, &str)] = &[(0, vertex_i18n::src!("Max")), (1, vertex_i18n::src!("Balanced")), (2, vertex_i18n::src!("Power Saver"))];
const VIEW_DISTANCE_LEGACY: &[(i64, &str)] =
    &[(0, vertex_i18n::src!("Far")), (1, vertex_i18n::src!("Normal")), (2, vertex_i18n::src!("Short")), (3, vertex_i18n::src!("Tiny"))];

const fn unit_float() -> Kind {
    Kind::Float { min: 0.0, max: 1.0 }
}

const fn toggle(
    key: &'static str,
    label: &'static str,
    group: Group,
    default: &'static str,
) -> OptionDef {
    OptionDef::new(key, label, group, Kind::Bool, default)
}

const fn slider(
    key: &'static str,
    label: &'static str,
    group: Group,
    default: &'static str,
) -> OptionDef {
    OptionDef::new(key, label, group, unit_float(), default).display(Percent)
}

const fn sound(key: &'static str, label: &'static str) -> OptionDef {
    slider(key, label, Audio, "1.0").since(1, 7, 2)
}

const fn part(key: &'static str, label: &'static str) -> OptionDef {
    toggle(key, label, Skin, "true").since(1, 8, 0)
}

const fn bind(key: &'static str, label: &'static str, default: &'static str) -> OptionDef {
    OptionDef::new(key, label, Keybinds, Kind::Keybind, default)
}

/// Every known key. The same key may appear more than once when its type changed.
pub static OPTIONS: &[OptionDef] = &[
    // ── Video ──────────────────────────────────────────────────────────────────────────
    OptionDef::new(
        "ao",
        vertex_i18n::src!("Smooth Lighting"),
        Video,
        Kind::IntChoice(AO_LEVELS),
        "2",
    )
    .until(1, 14, 0),
    toggle("ao", vertex_i18n::src!("Smooth Lighting"), Video, "true").since(1, 14, 0),
    OptionDef::new(
        "biomeBlendRadius",
        vertex_i18n::src!("Biome Blend"),
        Video,
        Kind::Int { min: 0, max: 7 },
        "2",
    )
    .since(1, 15, 0),
    OptionDef::new(
        "chunkSectionFadeInTime",
        vertex_i18n::src!("Chunk Fade-In Time"),
        Video,
        Kind::Float { min: 0.0, max: 2.0 },
        "0.75",
    )
    .since(1, 21, 11),
    toggle("cutoutLeaves", vertex_i18n::src!("Cutout Leaves"), Video, "true").since(1, 21, 11),
    toggle("enableVsync", vertex_i18n::src!("VSync"), Video, "true").since(1, 3, 1),
    OptionDef::new(
        "entityDistanceScaling",
        vertex_i18n::src!("Entity Distance"),
        Video,
        Kind::Float { min: 0.5, max: 5.0 },
        "1.0",
    )
    .since(1, 16, 0)
    .display(Percent),
    toggle("entityShadows", vertex_i18n::src!("Entity Shadows"), Video, "true").since(1, 8, 1),
    OptionDef::new(
        "fov",
        vertex_i18n::src!("Field of View"),
        Video,
        Kind::Float {
            min: -1.0,
            max: 1.0,
        },
        "0.0",
    )
    .display(Display::Degrees {
        offset: 70.0,
        scale: 40.0,
    })
    .note(vertex_i18n::src!("Stored normalized: 0.0 is 70°.")),
    slider("fovEffectScale", vertex_i18n::src!("FOV Effects"), Accessibility, "1.0").since(1, 16, 2),
    slider(
        "screenEffectScale",
        vertex_i18n::src!("Distortion Effects"),
        Accessibility,
        "1.0",
    )
    .since(1, 16, 2),
    slider(
        "darknessEffectScale",
        vertex_i18n::src!("Darkness Pulsing"),
        Accessibility,
        "1.0",
    )
    .since(1, 19, 0),
    slider("glintSpeed", vertex_i18n::src!("Glint Speed"), Video, "0.5").since(1, 19, 4),
    slider("glintStrength", vertex_i18n::src!("Glint Strength"), Video, "0.75").since(1, 19, 4),
    OptionDef::new(
        "preferredGraphicsBackend",
        vertex_i18n::src!("Graphics Backend"),
        Video,
        Kind::Choice(BACKENDS),
        "default",
    )
    .since(26, 1, 0)
    .quoted()
    .machine(),
    toggle("fancyGraphics", vertex_i18n::src!("Fancy Graphics"), Video, "true").until(1, 16, 0),
    OptionDef::new(
        "graphicsMode",
        vertex_i18n::src!("Graphics"),
        Video,
        Kind::IntChoice(GRAPHICS_MODE),
        "1",
    )
    .since(1, 16, 0)
    .until(1, 21, 11),
    OptionDef::new(
        "graphicsPreset",
        vertex_i18n::src!("Graphics Preset"),
        Video,
        Kind::Choice(GRAPHICS_PRESET),
        "fancy",
    )
    .since(1, 21, 11)
    .quoted(),
    OptionDef::new(
        "prioritizeChunkUpdates",
        vertex_i18n::src!("Chunk Builder"),
        Video,
        Kind::IntChoice(CHUNK_UPDATES),
        "0",
    )
    .since(1, 18, 0),
    toggle("fullscreen", vertex_i18n::src!("Fullscreen"), Video, "false")
        .since(1, 3, 1)
        .machine(),
    toggle(
        "exclusiveFullscreen",
        vertex_i18n::src!("Exclusive Fullscreen"),
        Video,
        "false",
    )
    .since(26, 1, 0)
    .machine(),
    toggle(
        "macFullscreenMenuVisibility",
        vertex_i18n::src!("Mac Fullscreen Menu Bar"),
        Video,
        "false",
    )
    .since(26, 3, 0)
    .machine(),
    slider("gamma", vertex_i18n::src!("Brightness"), Video, "0.5"),
    OptionDef::new(
        "guiScale",
        vertex_i18n::src!("GUI Scale (0 = Auto)"),
        Video,
        Kind::Int { min: 0, max: 10 },
        "0",
    ),
    OptionDef::new(
        "debugGuiScale",
        vertex_i18n::src!("Debug GUI Scale"),
        Video,
        Kind::Int { min: -1, max: 10 },
        "0",
    )
    .since(26, 3, 0),
    OptionDef::new(
        "maxAnisotropyBit",
        vertex_i18n::src!("Anisotropic Filtering (bits)"),
        Video,
        Kind::Int { min: 1, max: 3 },
        "2",
    )
    .since(1, 21, 11),
    OptionDef::new(
        "textureFiltering",
        vertex_i18n::src!("Texture Filtering"),
        Video,
        Kind::Int { min: 0, max: 2 },
        "0",
    )
    .since(1, 21, 11),
    OptionDef::new(
        "maxFps",
        vertex_i18n::src!("Max Framerate"),
        Video,
        Kind::Int { min: 10, max: 260 },
        "120",
    )
    .since(1, 7, 2),
    OptionDef::new(
        "fpsLimit",
        vertex_i18n::src!("Max Framerate"),
        Video,
        Kind::IntChoice(FPS_LIMIT_LEGACY),
        "0",
    )
    .until(1, 7, 2),
    toggle(
        "improvedTransparency",
        vertex_i18n::src!("Improved Transparency"),
        Video,
        "false",
    )
    .since(1, 21, 11),
    OptionDef::new(
        "inactivityFpsLimit",
        vertex_i18n::src!("Inactivity FPS Limit"),
        Video,
        Kind::Choice(INACTIVITY),
        "afk",
    )
    .since(1, 21, 2)
    .quoted(),
    OptionDef::new(
        "mipmapLevels",
        vertex_i18n::src!("Mipmap Levels"),
        Video,
        Kind::Int { min: 0, max: 4 },
        "4",
    )
    .since(1, 7, 2),
    toggle("clouds", vertex_i18n::src!("Clouds"), Video, "true").until(1, 8, 0),
    OptionDef::new(
        "renderClouds",
        vertex_i18n::src!("Clouds"),
        Video,
        Kind::Choice(CLOUDS),
        "true",
    )
    .since(1, 8, 0)
    .quoted(),
    OptionDef::new(
        "cloudRange",
        vertex_i18n::src!("Cloud Distance"),
        Video,
        Kind::Int { min: 2, max: 128 },
        "128",
    )
    .since(1, 21, 6),
    OptionDef::new(
        "viewDistance",
        vertex_i18n::src!("Render Distance"),
        Video,
        Kind::IntChoice(VIEW_DISTANCE_LEGACY),
        "1",
    )
    .until(1, 7, 2),
    OptionDef::new(
        "renderDistance",
        vertex_i18n::src!("Render Distance (chunks)"),
        Video,
        Kind::Int { min: 2, max: 32 },
        "12",
    )
    .since(1, 7, 2),
    OptionDef::new(
        "simulationDistance",
        vertex_i18n::src!("Simulation Distance (chunks)"),
        Video,
        Kind::Int { min: 5, max: 32 },
        "12",
    )
    .since(1, 18, 0),
    toggle("vignette", vertex_i18n::src!("Vignette"), Video, "true").since(1, 21, 11),
    OptionDef::new(
        "weatherRadius",
        vertex_i18n::src!("Weather Radius"),
        Video,
        Kind::Int { min: 3, max: 10 },
        "10",
    )
    .since(1, 21, 11),
    toggle("bobView", vertex_i18n::src!("View Bobbing"), Video, "true"),
    OptionDef::new(
        "particles",
        vertex_i18n::src!("Particles"),
        Video,
        Kind::IntChoice(PARTICLES),
        "0",
    ),
    toggle("reducedDebugInfo", vertex_i18n::src!("Reduced Debug Info"), Video, "false").since(1, 8, 0),
    OptionDef::new(
        "attackIndicator",
        vertex_i18n::src!("Attack Indicator"),
        Video,
        Kind::IntChoice(ATTACK_INDICATOR),
        "1",
    )
    .since(1, 9, 0),
    toggle("anaglyph3d", vertex_i18n::src!("3D Anaglyph"), Video, "false").until(1, 13, 0),
    toggle("advancedOpengl", vertex_i18n::src!("Advanced OpenGL"), Video, "false").until(1, 14, 0),
    toggle("useVbo", vertex_i18n::src!("Use VBOs"), Video, "true")
        .since(1, 1, 0)
        .until(1, 14, 0),
    toggle("fboEnable", vertex_i18n::src!("Framebuffer Objects"), Video, "true")
        .since(1, 1, 0)
        .until(1, 14, 0),
    OptionDef::new(
        "overrideWidth",
        vertex_i18n::src!("Window Width Override (0 = none)"),
        Video,
        Kind::Int { min: 0, max: 16384 },
        "0",
    )
    .since(1, 4, 6)
    .machine(),
    OptionDef::new(
        "overrideHeight",
        vertex_i18n::src!("Window Height Override (0 = none)"),
        Video,
        Kind::Int { min: 0, max: 16384 },
        "0",
    )
    .since(1, 4, 6)
    .machine(),
    OptionDef::new(
        "fullscreenResolution",
        vertex_i18n::src!("Fullscreen Resolution"),
        Video,
        Kind::Text,
        "",
    )
    .since(1, 13, 0)
    .machine(),
    // ── Audio ──────────────────────────────────────────────────────────────────────────
    slider("music", vertex_i18n::src!("Music"), Audio, "1.0").until(1, 7, 2),
    slider("sound", vertex_i18n::src!("Sound"), Audio, "1.0").until(1, 7, 2),
    sound("soundCategory_master", vertex_i18n::src!("Master Volume")),
    sound("soundCategory_music", vertex_i18n::src!("Music")),
    sound("soundCategory_record", vertex_i18n::src!("Jukebox/Note Blocks")),
    sound("soundCategory_weather", vertex_i18n::src!("Weather")),
    sound("soundCategory_block", vertex_i18n::src!("Blocks")),
    sound("soundCategory_hostile", vertex_i18n::src!("Hostile Creatures")),
    sound("soundCategory_neutral", vertex_i18n::src!("Friendly Creatures")),
    sound("soundCategory_player", vertex_i18n::src!("Players")),
    sound("soundCategory_ambient", "Ambient/Environment"),
    sound("soundCategory_voice", "Voice/Speech").since(1, 9, 0),
    sound("soundCategory_ui", vertex_i18n::src!("UI")).since(1, 21, 6),
    OptionDef::new("soundDevice", vertex_i18n::src!("Output Device"), Audio, Kind::Text, "")
        .since(1, 18, 0)
        .quoted()
        .machine(),
    toggle("showSubtitles", vertex_i18n::src!("Show Subtitles"), Audio, "false").since(1, 9, 0),
    toggle("directionalAudio", vertex_i18n::src!("Directional Audio"), Audio, "false").since(1, 19, 0),
    toggle(
        "showNowPlayingToast",
        vertex_i18n::src!("Show Now Playing Toast"),
        Audio,
        "true",
    )
    .since(1, 21, 5)
    .until(1, 21, 11),
    OptionDef::new(
        "musicToast",
        vertex_i18n::src!("Music Toast"),
        Audio,
        Kind::Choice(MUSIC_TOAST),
        "never",
    )
    .since(1, 21, 11)
    .quoted(),
    OptionDef::new(
        "musicFrequency",
        vertex_i18n::src!("Music Frequency"),
        Audio,
        Kind::Choice(MUSIC_FREQUENCY),
        vertex_i18n::src!("DEFAULT"),
    )
    .since(1, 21, 6)
    .quoted(),
    OptionDef::new(
        "narrator",
        vertex_i18n::src!("Narrator"),
        Audio,
        Kind::IntChoice(OFF_ON_NARRATOR),
        "0",
    )
    .since(1, 12, 0),
    // ── Controls ───────────────────────────────────────────────────────────────────────
    OptionDef::new(
        "mouseSensitivity",
        vertex_i18n::src!("Mouse Sensitivity"),
        Controls,
        unit_float(),
        "0.5",
    )
    .display(Percent),
    toggle("invertYMouse", vertex_i18n::src!("Invert Mouse Y"), Controls, "false"),
    toggle("invertXMouse", vertex_i18n::src!("Invert Mouse X"), Controls, "false").since(1, 21, 9),
    OptionDef::new(
        "mouseWheelSensitivity",
        vertex_i18n::src!("Scroll Sensitivity"),
        Controls,
        Kind::Float {
            min: 1.0,
            max: 10.0,
        },
        "1.0",
    )
    .since(1, 13, 0),
    toggle(
        "discrete_mouse_scroll",
        vertex_i18n::src!("Discrete Scrolling"),
        Controls,
        "false",
    )
    .since(1, 14, 0),
    toggle("touchscreen", vertex_i18n::src!("Touchscreen Mode"), Controls, "false")
        .since(1, 14, 0)
        .until(1, 21, 11),
    toggle("rawMouseInput", vertex_i18n::src!("Raw Input"), Controls, "true")
        .since(1, 15, 0)
        .until(1, 21, 11),
    toggle("autoJump", vertex_i18n::src!("Auto-Jump"), Controls, "false").since(1, 10, 0),
    toggle("toggleCrouch", vertex_i18n::src!("Toggle Crouch"), Controls, "false").since(1, 15, 0),
    toggle("toggleSprint", vertex_i18n::src!("Toggle Sprint"), Controls, "false").since(1, 15, 0),
    toggle("toggleAttack", vertex_i18n::src!("Toggle Attack"), Controls, "false").since(1, 21, 9),
    toggle("toggleUse", vertex_i18n::src!("Toggle Use"), Controls, "false").since(1, 21, 9),
    OptionDef::new(
        "sprintWindow",
        vertex_i18n::src!("Sprint Window (ticks)"),
        Controls,
        Kind::Int { min: 0, max: 10 },
        "7",
    )
    .since(1, 21, 9),
    toggle(
        "rotateWithMinecart",
        vertex_i18n::src!("Rotate With Minecart"),
        Controls,
        "false",
    )
    .since(1, 21, 2),
    toggle(
        "allowCursorChanges",
        vertex_i18n::src!("Allow Cursor Changes"),
        Controls,
        "true",
    )
    .since(1, 21, 9),
    toggle("quitShortcuts", vertex_i18n::src!("Quit Shortcuts"), Controls, "true").since(26, 3, 0),
    toggle(
        "ctrlClickEmulatesRightClick",
        vertex_i18n::src!("Ctrl-Click Is Right-Click"),
        Controls,
        "false",
    )
    .since(26, 3, 0),
    toggle("operatorItemsTab", vertex_i18n::src!("Operator Items Tab"), Controls, "false").since(1, 19, 3),
    // ── Chat ───────────────────────────────────────────────────────────────────────────
    OptionDef::new(
        "chatVisibility",
        vertex_i18n::src!("Chat"),
        Chat,
        Kind::IntChoice(CHAT_VISIBILITY),
        "0",
    )
    .since(1, 3, 1),
    toggle("chatColors", vertex_i18n::src!("Colors"), Chat, "true").since(1, 3, 1),
    toggle("chatLinks", vertex_i18n::src!("Web Links"), Chat, "true").since(1, 3, 1),
    toggle("chatLinksPrompt", vertex_i18n::src!("Prompt on Links"), Chat, "true").since(1, 3, 1),
    slider("chatOpacity", vertex_i18n::src!("Chat Text Opacity"), Chat, "1.0").since(1, 3, 1),
    slider("chatLineSpacing", vertex_i18n::src!("Line Spacing"), Chat, "0.0").since(1, 16, 0),
    slider(
        "textBackgroundOpacity",
        vertex_i18n::src!("Text Background Opacity"),
        Chat,
        "0.5",
    )
    .since(1, 14, 0),
    toggle(
        "backgroundForChatOnly",
        vertex_i18n::src!("Background for Chat Only"),
        Chat,
        "true",
    )
    .since(1, 14, 0),
    slider("chatHeightFocused", vertex_i18n::src!("Focused Height"), Chat, "1.0").since(1, 5, 1),
    slider("chatHeightUnfocused", vertex_i18n::src!("Unfocused Height"), Chat, "0.4375").since(1, 5, 1),
    slider("chatScale", vertex_i18n::src!("Chat Text Size"), Chat, "1.0").since(1, 5, 1),
    slider("chatWidth", vertex_i18n::src!("Chat Width"), Chat, "1.0").since(1, 5, 1),
    OptionDef::new(
        "chatDelay",
        vertex_i18n::src!("Chat Delay (seconds)"),
        Chat,
        Kind::Float { min: 0.0, max: 6.0 },
        "0.0",
    )
    .since(1, 16, 0),
    OptionDef::new(
        "notificationDisplayTime",
        vertex_i18n::src!("Notification Time"),
        Chat,
        Kind::Float {
            min: 0.0,
            max: 10.0,
        },
        "1.0",
    )
    .since(1, 19, 4),
    toggle("autoSuggestions", vertex_i18n::src!("Command Suggestions"), Chat, "true").since(1, 13, 0),
    toggle("hideMatchedNames", vertex_i18n::src!("Hide Matched Names"), Chat, "true").since(1, 16, 4),
    toggle("onlyShowSecureChat", vertex_i18n::src!("Only Show Secure Chat"), Chat, "false").since(1, 19, 0),
    toggle("saveChatDrafts", vertex_i18n::src!("Save Chat Drafts"), Chat, "false").since(1, 21, 9),
    toggle("hideServerAddress", vertex_i18n::src!("Hide Server Address"), Chat, "false").since(1, 3, 2),
    toggle("advancedItemTooltips", vertex_i18n::src!("Advanced Tooltips"), Chat, "false").since(1, 4, 2),
    // ── Accessibility ──────────────────────────────────────────────────────────────────
    toggle("highContrast", vertex_i18n::src!("High Contrast"), Accessibility, "false").since(1, 19, 4),
    toggle(
        "highContrastBlockOutline",
        vertex_i18n::src!("High Contrast Block Outline"),
        Accessibility,
        "false",
    )
    .since(1, 21, 2),
    toggle("narratorHotkey", vertex_i18n::src!("Narrator Hotkey"), Accessibility, "true").since(1, 20, 2),
    toggle(
        "darkMojangStudiosBackground",
        vertex_i18n::src!("Dark Loading Background"),
        Accessibility,
        "false",
    )
    .since(1, 17, 0),
    toggle(
        "hideLightningFlashes",
        vertex_i18n::src!("Hide Lightning Flashes"),
        Accessibility,
        "false",
    )
    .since(1, 18, 0),
    toggle(
        "hideSplashTexts",
        vertex_i18n::src!("Hide Splash Texts"),
        Accessibility,
        "false",
    )
    .since(1, 20, 3),
    slider("damageTiltStrength", vertex_i18n::src!("Damage Tilt"), Accessibility, "1.0").since(1, 19, 4),
    slider(
        "panoramaScrollSpeed",
        vertex_i18n::src!("Panorama Scroll Speed"),
        Accessibility,
        "1.0",
    )
    .since(1, 19, 3),
    OptionDef::new(
        "menuBackgroundBlurriness",
        vertex_i18n::src!("Menu Background Blur"),
        Accessibility,
        Kind::Int { min: 0, max: 10 },
        "5",
    )
    .since(1, 20, 5),
    toggle(
        "showAutosaveIndicator",
        vertex_i18n::src!("Autosave Indicator"),
        Accessibility,
        "true",
    )
    .since(1, 18, 0),
    toggle(
        "onboardAccessibility",
        vertex_i18n::src!("Accessibility Onboarding"),
        Internal,
        "true",
    )
    .since(1, 19, 4),
    // ── Skin ───────────────────────────────────────────────────────────────────────────
    part("modelPart_cape", vertex_i18n::src!("Cape")),
    part("modelPart_jacket", vertex_i18n::src!("Jacket")),
    part("modelPart_left_sleeve", vertex_i18n::src!("Left Sleeve")),
    part("modelPart_right_sleeve", vertex_i18n::src!("Right Sleeve")),
    part("modelPart_left_pants_leg", vertex_i18n::src!("Left Pants Leg")),
    part("modelPart_right_pants_leg", vertex_i18n::src!("Right Pants Leg")),
    part("modelPart_hat", vertex_i18n::src!("Hat")),
    OptionDef::new("mainHand", vertex_i18n::src!("Main Hand"), Skin, Kind::Choice(HAND), "right")
        .since(1, 9, 0)
        .quoted(),
    // ── Language & fonts ───────────────────────────────────────────────────────────────
    OptionDef::new("lang", vertex_i18n::src!("Language"), Language, Kind::Text, "en_us").since(1, 1, 0),
    toggle("forceUnicodeFont", vertex_i18n::src!("Force Unicode Font"), Language, "false").since(1, 7, 2),
    toggle(
        "japaneseGlyphVariants",
        vertex_i18n::src!("Japanese Glyph Variants"),
        Language,
        "false",
    )
    .since(1, 20, 5),
    // ── Online & misc ──────────────────────────────────────────────────────────────────
    OptionDef::new("lastServer", vertex_i18n::src!("Last Server"), Online, Kind::Text, "")
        .machine()
        .hidden(),
    OptionDef::new(
        "resourcePacks",
        vertex_i18n::src!("Enabled Resource Packs"),
        Online,
        Kind::List,
        "[]",
    )
    .since(1, 7, 2),
    OptionDef::new(
        "incompatibleResourcePacks",
        vertex_i18n::src!("Incompatible Resource Packs"),
        Online,
        Kind::List,
        "[]",
    )
    .since(1, 8, 8),
    OptionDef::new(
        "difficulty",
        vertex_i18n::src!("Difficulty"),
        Online,
        Kind::IntChoice(DIFFICULTY),
        "2",
    )
    .until(1, 14, 0),
    toggle("pauseOnLostFocus", vertex_i18n::src!("Pause on Lost Focus"), Online, "true").since(1, 4, 2),
    toggle(
        "realmsNotifications",
        vertex_i18n::src!("Realms Notifications"),
        Online,
        "true",
    )
    .since(1, 8, 9),
    toggle(
        "skipMultiplayerWarning",
        vertex_i18n::src!("Skip Multiplayer Warning"),
        Online,
        "false",
    )
    .since(1, 15, 2),
    toggle("allowServerListing", vertex_i18n::src!("Allow Server Listing"), Online, "true").since(1, 18, 0),
    toggle("useNativeTransport", vertex_i18n::src!("Native Networking"), Online, "true").since(1, 8, 1),
    toggle("telemetryOptInExtra", vertex_i18n::src!("Extra Telemetry"), Online, "false").since(1, 19, 3),
    OptionDef::new(
        "sharePresence",
        vertex_i18n::src!("Share Presence"),
        Online,
        Kind::Choice(SHARE_PRESENCE),
        "all",
    )
    .since(26, 2, 0)
    .quoted(),
    toggle(
        "inGameNotification",
        vertex_i18n::src!("In-Game Notifications"),
        Online,
        "false",
    )
    .since(26, 2, 0),
    // ── Internal bookkeeping ───────────────────────────────────────────────────────────
    OptionDef::new(
        "version",
        vertex_i18n::src!("Options Version"),
        Internal,
        Kind::Int {
            min: 0,
            max: 1_000_000,
        },
        "0",
    )
    .since(1, 10, 0)
    .machine()
    .hidden(),
    OptionDef::new(
        "tutorialStep",
        vertex_i18n::src!("Tutorial Step"),
        Internal,
        Kind::Choice(TUTORIAL),
        "movement",
    )
    .since(1, 12, 0)
    .machine(),
    toggle(
        "joinedFirstServer",
        vertex_i18n::src!("Joined First Server"),
        Internal,
        "false",
    )
    .since(1, 16, 4)
    .machine()
    .hidden(),
    toggle("syncChunkWrites", vertex_i18n::src!("Sync Chunk Writes"), Internal, "false")
        .since(1, 16, 0)
        .machine(),
    toggle("startedCleanly", vertex_i18n::src!("Started Cleanly"), Internal, "true")
        .since(1, 21, 5)
        .machine()
        .hidden(),
    OptionDef::new(
        "glDebugVerbosity",
        vertex_i18n::src!("GL Debug Verbosity"),
        Internal,
        Kind::Int { min: 0, max: 4 },
        "1",
    )
    .since(1, 13, 0)
    .machine(),
    // ── Key binds ──────────────────────────────────────────────────────────────────────
    bind("key_key.attack", "Attack/Destroy", "key.mouse.left"),
    bind("key_key.use", vertex_i18n::src!("Use Item/Place Block"), "key.mouse.right"),
    bind("key_key.forward", vertex_i18n::src!("Walk Forwards"), "key.keyboard.w"),
    bind("key_key.left", vertex_i18n::src!("Strafe Left"), "key.keyboard.a"),
    bind("key_key.back", vertex_i18n::src!("Walk Backwards"), "key.keyboard.s"),
    bind("key_key.right", vertex_i18n::src!("Strafe Right"), "key.keyboard.d"),
    bind("key_key.jump", vertex_i18n::src!("Jump"), "key.keyboard.space"),
    bind("key_key.sneak", vertex_i18n::src!("Sneak"), "key.keyboard.left.shift"),
    bind("key_key.sprint", vertex_i18n::src!("Sprint"), "key.keyboard.left.control").since(1, 3, 1),
    bind("key_key.drop", vertex_i18n::src!("Drop Selected Item"), "key.keyboard.q"),
    bind(
        "key_key.inventory",
        vertex_i18n::src!("Open/Close Inventory"),
        "key.keyboard.e",
    ),
    bind("key_key.chat", vertex_i18n::src!("Open Chat"), "key.keyboard.t"),
    bind("key_key.playerlist", vertex_i18n::src!("List Players"), "key.keyboard.tab"),
    bind("key_key.pickItem", vertex_i18n::src!("Pick Block"), "key.mouse.middle"),
    bind("key_key.command", vertex_i18n::src!("Open Command"), "key.keyboard.slash").since(1, 3, 1),
    bind("key_key.friends", vertex_i18n::src!("Friends"), "key.keyboard.o").since(26, 2, 0),
    bind(
        "key_key.socialInteractions",
        vertex_i18n::src!("Social Interactions Screen"),
        "key.keyboard.p",
    )
    .since(1, 16, 4),
    bind("key_key.toggleGui", vertex_i18n::src!("Hide GUI"), "key.keyboard.f1").since(1, 21, 11),
    bind(
        "key_key.toggleSpectatorShaderEffects",
        vertex_i18n::src!("Toggle Spectator Shader Effects"),
        "key.keyboard.f4",
    )
    .since(1, 21, 11),
    bind("key_key.screenshot", vertex_i18n::src!("Take Screenshot"), "key.keyboard.f2").since(1, 7, 2),
    bind(
        "key_key.togglePerspective",
        vertex_i18n::src!("Toggle Perspective"),
        "key.keyboard.f5",
    )
    .since(1, 7, 2),
    bind(
        "key_key.smoothCamera",
        vertex_i18n::src!("Cinematic Camera"),
        "key.keyboard.unknown",
    )
    .since(1, 7, 2),
    bind(
        "key_key.fullscreen",
        vertex_i18n::src!("Toggle Fullscreen"),
        "key.keyboard.f11",
    )
    .since(1, 7, 4),
    bind(
        "key_key.spectatorOutlines",
        vertex_i18n::src!("Spectator Outlines"),
        "key.keyboard.unknown",
    )
    .since(1, 8, 0),
    bind(
        "key_key.spectatorHotbar",
        vertex_i18n::src!("Spectator Hotbar"),
        "key.mouse.middle",
    )
    .since(1, 21, 9),
    bind(
        "key_key.swapHands",
        vertex_i18n::src!("Swap Item With Offhand"),
        "key.keyboard.f",
    )
    .since(1, 9, 0)
    .until(1, 16, 0),
    bind(
        "key_key.swapOffhand",
        vertex_i18n::src!("Swap Item With Offhand"),
        "key.keyboard.f",
    )
    .since(1, 16, 0),
    bind(
        "key_key.saveToolbarActivator",
        vertex_i18n::src!("Save Hotbar Activator"),
        "key.keyboard.c",
    )
    .since(1, 12, 0),
    bind(
        "key_key.loadToolbarActivator",
        vertex_i18n::src!("Load Hotbar Activator"),
        "key.keyboard.x",
    )
    .since(1, 12, 0),
    bind("key_key.advancements", vertex_i18n::src!("Advancements"), "key.keyboard.l").since(1, 12, 0),
    bind("key_key.quickActions", vertex_i18n::src!("Quick Actions"), "key.keyboard.g").since(1, 21, 6),
    bind("key_key.debug.overlay", vertex_i18n::src!("Debug Overlay"), "key.keyboard.f3").since(1, 21, 11),
    bind(
        "key_key.debug.modifier",
        vertex_i18n::src!("Debug Shortcut Modifier"),
        "key.keyboard.f3",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.reloadChunk",
        vertex_i18n::src!("Debug: Reload Chunks"),
        "key.keyboard.a",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.showHitboxes",
        vertex_i18n::src!("Debug: Show Hitboxes"),
        "key.keyboard.b",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.clearChat",
        vertex_i18n::src!("Debug: Clear Chat"),
        "key.keyboard.d",
    )
    .since(1, 21, 11),
    bind("key_key.debug.crash", vertex_i18n::src!("Debug: Crash"), "key.keyboard.c").since(1, 21, 11),
    bind(
        "key_key.debug.showChunkBorders",
        vertex_i18n::src!("Debug: Chunk Borders"),
        "key.keyboard.g",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.showAdvancedTooltips",
        vertex_i18n::src!("Debug: Advanced Tooltips"),
        "key.keyboard.h",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.copyRecreateCommand",
        vertex_i18n::src!("Debug: Copy Recreate Command"),
        "key.keyboard.i",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.spectate",
        vertex_i18n::src!("Debug: Spectate"),
        "key.keyboard.n",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.switchGameMode",
        vertex_i18n::src!("Debug: Switch Game Mode"),
        "key.keyboard.f4",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.debugOptions",
        vertex_i18n::src!("Debug: Options Screen"),
        "key.keyboard.f6",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.focusPause",
        vertex_i18n::src!("Debug: Pause on Focus Loss"),
        "key.keyboard.p",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.dumpDynamicTextures",
        vertex_i18n::src!("Debug: Dump Textures"),
        "key.keyboard.s",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.reloadResourcePacks",
        vertex_i18n::src!("Debug: Reload Resource Packs"),
        "key.keyboard.t",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.profiling",
        vertex_i18n::src!("Debug: Profiling"),
        "key.keyboard.l",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.copyLocation",
        vertex_i18n::src!("Debug: Copy Location"),
        "key.keyboard.c",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.dumpVersion",
        vertex_i18n::src!("Debug: Dump Version"),
        "key.keyboard.v",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.profilingChart",
        vertex_i18n::src!("Debug: Profiling Chart"),
        "key.keyboard.1",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.fpsCharts",
        vertex_i18n::src!("Debug: FPS Charts"),
        "key.keyboard.2",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.networkCharts",
        vertex_i18n::src!("Debug: Network Charts"),
        "key.keyboard.3",
    )
    .since(1, 21, 11),
    bind(
        "key_key.debug.lightmapTexture",
        vertex_i18n::src!("Debug: Lightmap Texture"),
        "key.keyboard.4",
    )
    .since(26, 1, 0),
    bind(
        "key_key.debug.improvedTransparency",
        vertex_i18n::src!("Debug: Improved Transparency"),
        "key.keyboard.x",
    )
    .since(26, 3, 0),
    bind("key_key.hotbar.1", vertex_i18n::src!("Hotbar Slot 1"), "key.keyboard.1"),
    bind("key_key.hotbar.2", vertex_i18n::src!("Hotbar Slot 2"), "key.keyboard.2"),
    bind("key_key.hotbar.3", vertex_i18n::src!("Hotbar Slot 3"), "key.keyboard.3"),
    bind("key_key.hotbar.4", vertex_i18n::src!("Hotbar Slot 4"), "key.keyboard.4"),
    bind("key_key.hotbar.5", vertex_i18n::src!("Hotbar Slot 5"), "key.keyboard.5"),
    bind("key_key.hotbar.6", vertex_i18n::src!("Hotbar Slot 6"), "key.keyboard.6"),
    bind("key_key.hotbar.7", vertex_i18n::src!("Hotbar Slot 7"), "key.keyboard.7"),
    bind("key_key.hotbar.8", vertex_i18n::src!("Hotbar Slot 8"), "key.keyboard.8"),
    bind("key_key.hotbar.9", vertex_i18n::src!("Hotbar Slot 9"), "key.keyboard.9"),
];

/// The definition of `key` for `version` (`None` = unknown version, current definitions only).
pub fn find_def(key: &str, version: Option<Ver>) -> Option<&'static OptionDef> {
    OPTIONS
        .iter()
        .find(|def| def.key == key && def.applies_to(version))
}

/// The definition of `key` from any version; used to describe keys present in a file whose
/// version gating doesn't match (for example when the game version is unknown).
pub fn find_def_any(key: &str) -> Option<&'static OptionDef> {
    OPTIONS.iter().rev().find(|def| def.key == key)
}

/// Every definition available in `version`.
pub fn defs_for(version: Option<Ver>) -> impl Iterator<Item = &'static OptionDef> {
    OPTIONS.iter().filter(move |def| def.applies_to(version))
}

/// True for `key_*` entries, including ones this launcher doesn't know (mods).
pub fn is_keybind_key(key: &str) -> bool {
    key.starts_with("key_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_never_overlaps_between_versions() {
        for (i, a) in OPTIONS.iter().enumerate() {
            for b in &OPTIONS[i + 1..] {
                if a.key == b.key {
                    assert!(
                        a.until <= b.since || b.until <= a.since,
                        "{} has overlapping definitions",
                        a.key
                    );
                }
            }
        }
    }

    #[test]
    fn defaults_parse_for_their_kind() {
        for def in OPTIONS {
            match def.kind {
                Kind::Bool => assert!(
                    def.default == "true" || def.default == "false",
                    "{}",
                    def.key
                ),
                Kind::Int { .. } | Kind::IntChoice(_) => {
                    assert!(def.default.parse::<i64>().is_ok(), "{}", def.key)
                }
                Kind::Float { min, max } => {
                    let value: f64 = def
                        .default
                        .parse()
                        .unwrap_or_else(|_| panic!("{}", def.key));
                    assert!(
                        value >= min && value <= max,
                        "{} default out of range",
                        def.key
                    );
                }
                Kind::Choice(choices) => assert!(
                    choices.iter().any(|(value, _)| *value == def.default),
                    "{} default not a choice",
                    def.key
                ),
                Kind::Text | Kind::List | Kind::Keybind => {}
            }
        }
    }

    #[test]
    fn gating_picks_the_right_format() {
        assert!(matches!(
            find_def("ao", Some(Ver(1, 12, 2))).unwrap().kind,
            Kind::IntChoice(_)
        ));
        assert!(matches!(
            find_def("ao", Some(Ver(1, 21, 4))).unwrap().kind,
            Kind::Bool
        ));
        assert!(find_def("graphicsPreset", Some(Ver(1, 20, 4))).is_none());
        assert!(find_def("macFullscreenMenuVisibility", Some(Ver(26, 3, 0))).is_some());
    }
}

#[cfg(test)]
mod real_file_tests {
    use super::*;
    use crate::{OptionsFile, value};

    /// Checks the schema against real option files: set `VERTEX_OPTIONS_SAMPLES` to a
    /// `path[:path...]` list, each with the game version as `path=version`.
    #[test]
    fn schema_understands_real_option_files() {
        let Ok(samples) = std::env::var("VERTEX_OPTIONS_SAMPLES") else {
            return;
        };
        for sample in samples.split(',') {
            let (path, version) = sample.split_once('=').expect("path=version");
            let file = OptionsFile::read(std::path::Path::new(path)).expect("readable sample");
            let version = crate::parse_game_version(version);
            let mut problems = Vec::new();
            for key in file.keys() {
                let raw = file.get(key).unwrap();
                let Some(def) = find_def(key, version) else {
                    // Unknown keys are legal (mods add their own); report, don't fail.
                    if !is_keybind_key(key) && !key.starts_with("soundCategory_") {
                        eprintln!("{path}: key not in schema: {key}");
                    }
                    continue;
                };
                let ok = match def.kind {
                    Kind::Bool => value::parse_bool(raw).is_some(),
                    Kind::Int { min, max } => {
                        value::parse_i64(raw).is_some_and(|v| v >= min && v <= max)
                    }
                    Kind::IntChoice(c) => {
                        value::parse_i64(raw).is_some_and(|v| c.iter().any(|(k, _)| *k == v))
                    }
                    Kind::Float { min, max } => {
                        value::parse_f64(raw).is_some_and(|v| v >= min && v <= max)
                    }
                    Kind::Choice(c) => c.iter().any(|(k, _)| *k == value::unquote(raw).0),
                    Kind::Text | Kind::List | Kind::Keybind => true,
                };
                if !ok {
                    problems.push(format!("{key}:{raw} does not fit its definition"));
                }
                if def.quoted && matches!(def.kind, Kind::Choice(_)) && !value::unquote(raw).1 {
                    problems.push(format!("{key} expected quoted"));
                }
            }
            // Keys the version should have but the file lacks (informational).
            let missing: Vec<_> = defs_for(version)
                .filter(|d| !file.contains(d.key) && d.group != Group::Keybinds)
                .map(|d| d.key)
                .collect();
            eprintln!("{path}: missing-from-file {missing:?}");
            assert!(problems.is_empty(), "{path}: {problems:#?}");
        }
    }
}
