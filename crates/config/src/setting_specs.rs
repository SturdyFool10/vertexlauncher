use crate::{
    FRAME_LIMIT_FPS_MAX, FRAME_LIMIT_FPS_MIN, NOTIFICATION_FADE_OUT_SECONDS_MAX,
    NOTIFICATION_FADE_OUT_SECONDS_MIN, NOTIFICATION_FADE_OUT_SECONDS_STEP,
    SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MAX, SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MIN,
    SKIN_PREVIEW_MOTION_BLUR_AMOUNT_STEP, SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MAX,
    SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MIN, SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_STEP,
    SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MAX, SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MIN,
    SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_STEP, SKIN_PREVIEW_MSAA_SAMPLES_MAX,
    SKIN_PREVIEW_MSAA_SAMPLES_MIN, SKIN_PREVIEW_MSAA_SAMPLES_STEP, UI_FONT_SIZE_MAX,
    UI_FONT_SIZE_MIN, UI_FONT_SIZE_STEP, UI_FONT_WEIGHT_MAX, UI_FONT_WEIGHT_MIN,
    UI_FONT_WEIGHT_STEP,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToggleSettingId {
    LowPowerGpuPreferred,
    StreamerModeEnabled,
    ObsDetectionEnabled,
    LazyAccountRefresh,
    OpenTypeFeaturesEnabled,
    NotificationExpiryBarsEmptyLeft,
    SkinPreviewFreshFormatEnabled,
    SkinPreview3dLayersEnabled,
    SnapshotsAndBetasEnabled,
    AlphaVersionsEnabled,
    ExperimentalVersionsEnabled,
    ForceJava21Minimum,
    FrameLimiterEnabled,
    DiscordRichPresenceEnabled,
    SyncServersEnabled,
    SyncCommandHistoryEnabled,
    SyncHotbarsEnabled,
    SyncServersToAllInstancesByDefault,
    DragScrollEnabled,
    FollowConsoleOnLaunch,
    StartFullscreen,
    SteamDeckHandoff,
    DebugFpsCounter,
    DebugFramegraph,
}

#[derive(Clone, Copy, Debug)]
pub struct ToggleSettingSpec {
    pub id: ToggleSettingId,
    pub label: &'static str,
    pub info_tooltip: Option<&'static str>,
}

impl ToggleSettingId {
    /// Returns static metadata used to render this toggle setting.
    pub const fn spec(self) -> ToggleSettingSpec {
        match self {
            ToggleSettingId::LowPowerGpuPreferred => ToggleSettingSpec {
                id: ToggleSettingId::LowPowerGpuPreferred,
                label: "Prefer Integrated Graphics",
                info_tooltip: Some(
                    "On systems with both an integrated and a discrete GPU, the launcher window renders on the integrated one (lower power and heat) instead of the discrete one. This is the same as choosing the Low Power adapter profile below. It only affects the launcher, not Minecraft, and is ignored when a specific adapter is chosen.. Default: On.",
                ),
            },
            ToggleSettingId::StreamerModeEnabled => ToggleSettingSpec {
                id: ToggleSettingId::StreamerModeEnabled,
                label: "Enable Streamer Mode",
                info_tooltip: Some(
                    "For streaming and screenshots: shows \"Hidden Account\" instead of your account name in the top bar, Home, Instance and Skins screens, and masks account IDs (player UUID, XUID) in notifications. Nothing is changed on disk. Default: Off.",
                ),
            },

            ToggleSettingId::ObsDetectionEnabled => ToggleSettingSpec {
                id: ToggleSettingId::ObsDetectionEnabled,
                label: "Detect OBS",
                info_tooltip: Some(
                    "Applies Streamer Mode while OBS is running on this computer, and shows a notification when it starts and when it stops. This is temporary and never changes the Enable Streamer Mode setting: it ends when OBS closes or when you turn this off. OBS is found by its process name. The Flatpak build asks the host for its process list, which uses the host-command permission it ships with (org.freedesktop.Flatpak); a sandbox that hides other programs and has no such permission cannot see OBS. Default: On.",
                ),
            },
            ToggleSettingId::LazyAccountRefresh => ToggleSettingSpec {
                id: ToggleSettingId::LazyAccountRefresh,
                label: "Lazy Account Refresh",
                info_tooltip: Some(
                    "Renews an account's Microsoft/Minecraft sign-in only when something needs it, and only for the active account: right before launching a game, or when you open something that needs you signed in, such as the Skins screen. Nothing is renewed at startup and your other accounts are left alone until you switch to them. Off renews every saved account when the launcher starts. Renewing the active account yourself from the account menu still works either way. Default: Off.",
                ),
            },
            ToggleSettingId::OpenTypeFeaturesEnabled => ToggleSettingSpec {
                id: ToggleSettingId::OpenTypeFeaturesEnabled,
                label: "Enable OpenType Features",
                info_tooltip: Some(
                    "Turns on OpenType font features (kerning, ligatures, alternates, number styles) for launcher text. The features used are the tags listed below; if that list is empty, the built-in list is used: kern, liga, calt, onum, pnum. Default: On.",
                ),
            },
            ToggleSettingId::NotificationExpiryBarsEmptyLeft => ToggleSettingSpec {
                id: ToggleSettingId::NotificationExpiryBarsEmptyLeft,
                label: "Empty Expiry Bars to the Left",
                info_tooltip: Some(
                    "Changes which way the timer bar on each notification empties: on, the bar empties from the left edge toward the right; off, from the right edge toward the left. Default: Off.",
                ),
            },
            ToggleSettingId::SkinPreviewFreshFormatEnabled => ToggleSettingSpec {
                id: ToggleSettingId::SkinPreviewFreshFormatEnabled,
                label: "Enable Skin Expressions",
                info_tooltip: Some(
                    "Animates the eyes and eyebrows in the skin preview for skins that use a Fresh-style expression layout. Skins without such a layout are unaffected. Preview only; it does not change your skin. Default: Off.",
                ),
            },
            ToggleSettingId::SkinPreview3dLayersEnabled => ToggleSettingSpec {
                id: ToggleSettingId::SkinPreview3dLayersEnabled,
                label: "Enable 3D Skin Layers",
                info_tooltip: Some(
                    "Renders the second skin layer (hat, jacket, sleeves, pants) as raised 3D voxels in the skin preview instead of a flat overlay. Can be combined with skin expressions. Preview only; costs some GPU time. Default: Off.",
                ),
            },
            ToggleSettingId::SnapshotsAndBetasEnabled => ToggleSettingSpec {
                id: ToggleSettingId::SnapshotsAndBetasEnabled,
                label: "Include Snapshots and Betas",
                info_tooltip: Some(
                    "Adds Minecraft snapshots, pre-releases, release candidates and old betas to the version pickers when creating or editing an instance. Newer versions are listed first. Default: Off.",
                ),
            },
            ToggleSettingId::AlphaVersionsEnabled => ToggleSettingSpec {
                id: ToggleSettingId::AlphaVersionsEnabled,
                label: "Include Alpha Versions",
                info_tooltip: Some(
                    "Adds the very old Minecraft alpha releases (from 2010) to the version pickers. Many mods and features do not exist for these versions. Default: Off.",
                ),
            },
            ToggleSettingId::ExperimentalVersionsEnabled => ToggleSettingSpec {
                id: ToggleSettingId::ExperimentalVersionsEnabled,
                label: "Include Experimental Versions",
                info_tooltip: Some(
                    "Adds experimental and other non-standard entries from Mojang's version list to the version pickers. These are unsupported and may not launch. Default: Off.",
                ),
            },
            ToggleSettingId::ForceJava21Minimum => ToggleSettingSpec {
                id: ToggleSettingId::ForceJava21Minimum,
                label: "Force Java 21 Minimum",
                info_tooltip: Some(
                    "Runs Minecraft versions that need Java 8, 16 or 17 (1.20.4 and older) on Java 21 instead. Versions that need Java 21 or 25 are unchanged. Useful for a single modern runtime, but some old mods and loaders may not work on Java 21. Default: On.",
                ),
            },
            ToggleSettingId::FrameLimiterEnabled => ToggleSettingSpec {
                id: ToggleSettingId::FrameLimiterEnabled,
                label: "Enable Frame Limiter",
                info_tooltip: Some(
                    "Limits how often the launcher window redraws (see Frame Limit FPS) to save power and heat. It does not affect Minecraft. Applied immediately. Default: Off.",
                ),
            },
            ToggleSettingId::DiscordRichPresenceEnabled => ToggleSettingSpec {
                id: ToggleSettingId::DiscordRichPresenceEnabled,
                label: "Enable Discord Rich Presence",
                info_tooltip: Some(
                    "Shows your Discord status as the instance you are playing with an elapsed timer, or the launcher screen you are on when nothing is running. Skipped for instances that have their own Discord Rich Presence mod, so the two do not fight over your status. Needs the Discord desktop app running. Default: On.",
                ),
            },
            ToggleSettingId::SyncServersEnabled => ToggleSettingSpec {
                id: ToggleSettingId::SyncServersEnabled,
                label: "Sync Multiplayer Servers",
                info_tooltip: Some(
                    "Copies each multiplayer server to the instances chosen in its sync options. Servers already in an instance are never edited or removed. Runs before an instance launches and after one closes; instances that are running are left alone. Default: Off.",
                ),
            },
            ToggleSettingId::SyncCommandHistoryEnabled => ToggleSettingSpec {
                id: ToggleSettingId::SyncCommandHistoryEnabled,
                label: "Sync Command History",
                info_tooltip: Some(
                    "Merges chat command history (command_history.txt) across all instances so commands you typed in one show up in the others. Keeps the newest 50 unique commands. Runs before launch and after an instance closes. Default: Off.",
                ),
            },
            ToggleSettingId::SyncHotbarsEnabled => ToggleSettingSpec {
                id: ToggleSettingId::SyncHotbarsEnabled,
                label: "Sync Creative Hotbars",
                info_tooltip: Some(
                    "Shares saved creative hotbars (hotbar.nbt) across instances. Hotbars only flow from older Minecraft versions to the same or newer ones, because older versions cannot read newer item data. Runs before launch and after an instance closes. Default: Off.",
                ),
            },
            ToggleSettingId::SyncServersToAllInstancesByDefault => ToggleSettingSpec {
                id: ToggleSettingId::SyncServersToAllInstancesByDefault,
                label: "Sync New Servers to All Instances",
                info_tooltip: Some(
                    "Servers that have no sync options of their own are copied to every instance. Options you set on a specific server always override this. Needs Sync Multiplayer Servers to be on. Default: Off.",
                ),
            },
            ToggleSettingId::DragScrollEnabled => ToggleSettingSpec {
                id: ToggleSettingId::DragScrollEnabled,
                label: "Click and Drag Scrolling",
                info_tooltip: Some(
                    "Lets you scroll lists and pages by clicking and dragging their contents. Turn off if dragging keeps scrolling when you meant to select text or drag something. The mouse wheel and scroll bars always work. Default: On.",
                ),
            },
            ToggleSettingId::StartFullscreen => ToggleSettingSpec {
                id: ToggleSettingId::StartFullscreen,
                label: "Start Vertex in Fullscreen",
                info_tooltip: Some(
                    "Opens the launcher fullscreen, in the style chosen under Fullscreen Style. Press Escape or F11 to leave fullscreen. Default: Off.",
                ),
            },
            ToggleSettingId::SteamDeckHandoff => ToggleSettingSpec {
                id: ToggleSettingId::SteamDeckHandoff,
                label: "Hand Off to Minecraft (Steam Deck)",
                info_tooltip: Some(
                    "Hides Vertex's window while Minecraft runs, so the game takes the screen instead of Vertex staying in front of it in Steam Deck game mode, and brings Vertex back when the game closes. Turned on automatically on a Steam Deck. Default: Off.",
                ),
            },
            ToggleSettingId::DebugFpsCounter => ToggleSettingSpec {
                id: ToggleSettingId::DebugFpsCounter,
                label: "Show FPS Counter",
                info_tooltip: Some(
                    "Draws the launcher's frames per second and frame time in the top-right corner of the window. Default: Off.",
                ),
            },
            ToggleSettingId::DebugFramegraph => ToggleSettingSpec {
                id: ToggleSettingId::DebugFramegraph,
                label: "Show Frame Graph",
                info_tooltip: Some(
                    "Draws a live graph of how long each render pass takes on the CPU and GPU, plus a history of recent frame times, over the launcher. GPU times need a GPU that supports timestamp queries. Default: Off.",
                ),
            },
            ToggleSettingId::FollowConsoleOnLaunch => ToggleSettingSpec {
                id: ToggleSettingId::FollowConsoleOnLaunch,
                label: "Open Console When Launching",
                info_tooltip: Some(
                    "Switches to the instance's console as soon as you launch it, so you can watch the game start. Turn off to stay on the page you launched from. Default: On.",
                ),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropdownSettingId {
    UiFontFamily,
    WindowTransparency,
    GraphicsAdapterPreferenceType,
    GraphicsAdapterPreference,
    GraphicsApiPreference,
}

#[derive(Clone, Copy, Debug)]
pub struct DropdownSettingSpec {
    pub id: DropdownSettingId,
    pub label: &'static str,
    pub info_tooltip: Option<&'static str>,
}

impl DropdownSettingId {
    /// Returns static metadata used to render this dropdown setting.
    pub const fn spec(self) -> DropdownSettingSpec {
        match self {
            DropdownSettingId::UiFontFamily => DropdownSettingSpec {
                id: DropdownSettingId::UiFontFamily,
                label: "UI Font",
                info_tooltip: Some(
                    "The main font for launcher text. Each text style (headings, body, buttons) can override it in the font settings below. Default: Maple Mono NF (included).",
                ),
            },
            DropdownSettingId::WindowTransparency => DropdownSettingSpec {
                id: DropdownSettingId::WindowTransparency,
                label: "Window Transparency",
                info_tooltip: Some(
                    "Opaque is a normal window. Transparent lets the desktop show through the launcher. Blurred also asks the operating system to blur what is behind it, where your platform supports that.. Default: Opaque.",
                ),
            },
            DropdownSettingId::GraphicsAdapterPreferenceType => DropdownSettingSpec {
                id: DropdownSettingId::GraphicsAdapterPreferenceType,
                label: "Graphics Adapter Preference Type",
                info_tooltip: Some(
                    "Pick the launcher's GPU either by a performance profile (High Performance or Low Power) or by choosing one specific detected adapter.. Default: Performance Profile.",
                ),
            },
            DropdownSettingId::GraphicsAdapterPreference => DropdownSettingSpec {
                id: DropdownSettingId::GraphicsAdapterPreference,
                label: "Graphics Adapter Preference",
                info_tooltip: Some(
                    "With Performance Profile above, the kind of GPU to prefer: Default, High Performance, Low Power, Discrete Only or Integrated Only. With Explicit Adapter, the exact GPU to use; if it is no longer available (unplugged or driver change) the launcher falls back to the High Performance profile.. Default: Low Power.",
                ),
            },
            DropdownSettingId::GraphicsApiPreference => DropdownSettingSpec {
                id: DropdownSettingId::GraphicsApiPreference,
                label: "Graphics API Preference",
                info_tooltip: Some(
                    "Which graphics API the launcher renders with. Auto lets the launcher choose; Vulkan, Metal (macOS), DirectX 12 (Windows) and WebGPU force one; WebGPU reaches GPUs that neither Vulkan nor DirectX 12 can. Only APIs your system supports are listed. Try another if the launcher shows graphical glitches.. Default: Auto.",
                ),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloatSettingId {
    UiFontSize,
    NotificationFadeOutSeconds,
    SkinPreviewMotionBlurAmount,
    SkinPreviewMotionBlurShutterFrames,
}

#[derive(Clone, Copy, Debug)]
pub struct FloatSettingSpec {
    pub id: FloatSettingId,
    pub label: &'static str,
    pub info_tooltip: Option<&'static str>,
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

impl FloatSettingId {
    /// Returns static metadata used to render this float setting.
    pub const fn spec(self) -> FloatSettingSpec {
        match self {
            FloatSettingId::UiFontSize => FloatSettingSpec {
                id: FloatSettingId::UiFontSize,
                label: "UI Font Size",
                info_tooltip: Some(
                    "Base text size in points for body and button text. Fractions like 14.5 are allowed. Default: 18.",
                ),
                min: UI_FONT_SIZE_MIN,
                max: UI_FONT_SIZE_MAX,
                step: UI_FONT_SIZE_STEP,
            },
            FloatSettingId::NotificationFadeOutSeconds => FloatSettingSpec {
                id: FloatSettingId::NotificationFadeOutSeconds,
                label: "Notification Fade Out Time",
                info_tooltip: Some(
                    "How long, in seconds, a notification takes to fade away after its timer runs out. 0 makes it disappear instantly. Dismissing a notification with its X button is always instant. Default: 0.25.",
                ),
                min: NOTIFICATION_FADE_OUT_SECONDS_MIN,
                max: NOTIFICATION_FADE_OUT_SECONDS_MAX,
                step: NOTIFICATION_FADE_OUT_SECONDS_STEP,
            },
            FloatSettingId::SkinPreviewMotionBlurAmount => FloatSettingSpec {
                id: FloatSettingId::SkinPreviewMotionBlurAmount,
                label: "Skin Preview Motion Blur Amount",
                info_tooltip: Some(
                    "How strong the motion blur trail is on the rotating skin preview. 0 turns the blur off. Default: 0.15.",
                ),
                min: SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MIN,
                max: SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MAX,
                step: SKIN_PREVIEW_MOTION_BLUR_AMOUNT_STEP,
            },
            FloatSettingId::SkinPreviewMotionBlurShutterFrames => FloatSettingSpec {
                id: FloatSettingId::SkinPreviewMotionBlurShutterFrames,
                label: "Skin Preview Motion Blur Shutter",
                info_tooltip: Some(
                    "How long the virtual camera shutter stays open, in 60 FPS frame lengths. Longer shutters give longer blur trails. Default: 0.75.",
                ),
                min: SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MIN,
                max: SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MAX,
                step: SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_STEP,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntSettingId {
    UiFontWeight,
    FrameLimitFps,
    SkinPreviewMsaaSamples,
    SkinPreviewMotionBlurSampleCount,
}

#[derive(Clone, Copy, Debug)]
pub struct IntSettingSpec {
    pub id: IntSettingId,
    pub label: &'static str,
    pub info_tooltip: Option<&'static str>,
    pub min: i32,
    pub max: i32,
    pub step: i32,
}

impl IntSettingId {
    /// Returns static metadata used to render this integer setting.
    pub const fn spec(self) -> IntSettingSpec {
        match self {
            IntSettingId::UiFontWeight => IntSettingSpec {
                id: IntSettingId::UiFontWeight,
                label: "UI Font Weight",
                info_tooltip: Some(
                    "Font weight from 100 (thin) to 900 (black), in steps of 100; 400 is regular and 700 is bold. Fonts that lack the chosen weight may look different. Default: 400.",
                ),
                min: UI_FONT_WEIGHT_MIN,
                max: UI_FONT_WEIGHT_MAX,
                step: UI_FONT_WEIGHT_STEP,
            },
            IntSettingId::FrameLimitFps => IntSettingSpec {
                id: IntSettingId::FrameLimitFps,
                label: "Frame Limit FPS",
                info_tooltip: Some(
                    "The most frames per second the launcher window draws when the frame limiter is on (30\u{2013}240). Lower saves more power. Default: 120.",
                ),
                min: FRAME_LIMIT_FPS_MIN,
                max: FRAME_LIMIT_FPS_MAX,
                step: 1,
            },
            IntSettingId::SkinPreviewMsaaSamples => IntSettingSpec {
                id: IntSettingId::SkinPreviewMsaaSamples,
                label: "Skin Preview MSAA Samples",
                info_tooltip: Some(
                    "Samples per pixel for the skin preview when its anti-aliasing is MSAA (1\u{2013}8). More samples give smoother edges but use more GPU memory. Default: 4.",
                ),
                min: SKIN_PREVIEW_MSAA_SAMPLES_MIN,
                max: SKIN_PREVIEW_MSAA_SAMPLES_MAX,
                step: SKIN_PREVIEW_MSAA_SAMPLES_STEP,
            },
            IntSettingId::SkinPreviewMotionBlurSampleCount => IntSettingSpec {
                id: IntSettingId::SkinPreviewMotionBlurSampleCount,
                label: "Skin Preview Motion Blur Samples",
                info_tooltip: Some(
                    "How many snapshots of the skin preview are blended across the shutter interval (2\u{2013}16). More samples make smoother blur and cost more GPU work. Default: 5.",
                ),
                min: SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MIN,
                max: SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MAX,
                step: SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_STEP,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextSettingId {
    OpenTypeFeaturesToEnable,
}

#[derive(Clone, Copy, Debug)]
pub struct TextSettingSpec {
    pub id: TextSettingId,
    pub label: &'static str,
    pub info_tooltip: Option<&'static str>,
}

impl TextSettingId {
    /// Returns static metadata used to render this text setting.
    pub const fn spec(self) -> TextSettingSpec {
        match self {
            TextSettingId::OpenTypeFeaturesToEnable => TextSettingSpec {
                id: TextSettingId::OpenTypeFeaturesToEnable,
                label: "OpenType Features to Enable",
                info_tooltip: Some(
                    "Comma-separated four-letter OpenType feature tags, for example: liga, calt, ss01. Leave empty to use the built-in list. Only has an effect when OpenType features are enabled above, and only for fonts that provide those features. Default: empty (uses kern, liga, calt, onum, pnum).",
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    /// Every setting applies while the launcher runs, so no help text may ask for a restart.
    #[test]
    fn no_setting_asks_for_a_restart() {
        let source = include_str!("setting_specs.rs");
        let help_texts = source.split("mod tests").next().unwrap_or(source).to_ascii_lowercase();
        assert!(!help_texts.contains("restart"), "a setting's help text mentions a restart");
    }
}
