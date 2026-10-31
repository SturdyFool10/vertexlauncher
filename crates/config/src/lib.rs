use app_paths as launcher_paths;
mod config_format;
mod config_setting_bindings;
mod gamepad_calibration;
mod graphics_adapter_preference;
mod graphics_api_preference;
mod java_runtime_version;
mod hdr_color_space;
mod linux_blur_protocol;
mod macos_visual_effect_blending_mode;
mod macos_visual_effect_material;
mod macos_visual_effect_state;
mod notification_easing;
mod setting_specs;
mod settings_diff;
mod skin_preview_aa_mode;
mod skin_preview_texel_aa_mode;
mod svg_aa_mode;
mod text_rendering_path;
mod typography;
mod ui_fonts;
mod fullscreen_style;
mod window_transparency;
mod windows_backdrop_type;
mod windows_transparency_level;

pub use config_format::ConfigFormat;
pub use gamepad_calibration::GamepadCalibration;
pub use graphics_adapter_preference::{GraphicsAdapterPreferenceType, GraphicsAdapterProfile};
pub use graphics_api_preference::GraphicsApiPreference;
pub use java_runtime_version::JavaRuntimeVersion;
pub use hdr_color_space::HdrColorSpace;
pub use linux_blur_protocol::LinuxBlurProtocol;
pub use macos_visual_effect_blending_mode::MacosVisualEffectBlendingMode;
pub use macos_visual_effect_material::MacosVisualEffectMaterial;
pub use macos_visual_effect_state::MacosVisualEffectState;
pub use notification_easing::NotificationEasing;
pub use setting_specs::{
    DropdownSettingId, DropdownSettingSpec, FloatSettingId, FloatSettingSpec, IntSettingId,
    IntSettingSpec, TextSettingId, TextSettingSpec, ToggleSettingId, ToggleSettingSpec,
};
pub use settings_diff::{SettingsDiff, SettingsDomain};
pub use skin_preview_aa_mode::SkinPreviewAaMode;
pub use skin_preview_texel_aa_mode::SkinPreviewTexelAaMode;
pub use svg_aa_mode::SvgAaMode;
pub use text_rendering_path::TextRenderingPath;
pub use typography::{
    ROLE_FONT_SIZE_MAX, ROLE_FONT_SIZE_MIN, ROLE_FONT_SIZE_STEP, ResolvedTypography, RoleDefaults,
    RoleOverride, TextRole, TypographySettings,
};
pub use ui_fonts::{UiCjkFontFamily, UiEmojiFontFamily, UiFontFamily};
pub use fullscreen_style::FullscreenStyle;
pub use window_transparency::WindowTransparency;
pub use windows_backdrop_type::WindowsBackdropType;
pub use windows_transparency_level::WindowsTransparencyLevel;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Visitor},
};
use std::collections::BTreeMap;
use std::io::{Error as IOError, Write};
use std::path::{Path, PathBuf};

pub const UI_FONT_SIZE_MIN: f32 = 10.0;
pub const UI_FONT_SIZE_MAX: f32 = 42.0;
pub const UI_FONT_SIZE_STEP: f32 = 0.5;
pub const UI_OPACITY_PERCENT_MIN: u8 = 0;
pub const UI_OPACITY_PERCENT_MAX: u8 = 100;
/// Corner radius of the translucent window, in logical pixels. 0 is square.
pub const WINDOW_CORNER_RADIUS_MIN: u8 = 0;
pub const WINDOW_CORNER_RADIUS_MAX: u8 = 28;
pub const DEFAULT_WINDOW_CORNER_RADIUS: u8 = 12;
/// How strongly the Windows legacy blur and acrylic backdrops are tinted, in percent. 0 is untinted.
pub const WINDOWS_BLUR_TINT_OPACITY_MIN: u8 = 0;
pub const WINDOWS_BLUR_TINT_OPACITY_MAX: u8 = 100;
pub const UI_FONT_WEIGHT_MIN: i32 = 100;
pub const UI_FONT_WEIGHT_MAX: i32 = 900;
pub const UI_FONT_WEIGHT_STEP: i32 = 100;
pub const INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN: u128 = 512;
pub const INSTANCE_DEFAULT_MAX_MEMORY_MIB_MAX: u128 = 1_048_576;
pub const INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP: u128 = 256;
pub const DOWNLOAD_CONCURRENCY_MIN: u32 = 1;
pub const DOWNLOAD_CONCURRENCY_MAX: u32 = 128;
pub const DEFAULT_DOWNLOAD_CONCURRENCY: u32 = 8;
pub const HDR_UI_WHITE_SCALE_MIN: f32 = 0.5;
pub const HDR_UI_WHITE_SCALE_MAX: f32 = 4.0;
pub const HDR_LIGHT_LEVEL_NITS_MIN: i32 = 50;
pub const HDR_LIGHT_LEVEL_NITS_MAX: i32 = 10_000;
pub const FRAME_LIMIT_FPS_MIN: i32 = 30;
pub const FRAME_LIMIT_FPS_MAX: i32 = 240;
pub const NOTIFICATION_FADE_OUT_SECONDS_MIN: f32 = 0.0;
pub const NOTIFICATION_FADE_OUT_SECONDS_MAX: f32 = 2.0;
pub const NOTIFICATION_FADE_OUT_SECONDS_STEP: f32 = 0.05;
pub const DEFAULT_NOTIFICATION_FADE_OUT_SECONDS: f32 = 0.25;
pub const SKIN_PREVIEW_MSAA_SAMPLES_MIN: i32 = 1;
pub const SKIN_PREVIEW_MSAA_SAMPLES_MAX: i32 = 8;
pub const SKIN_PREVIEW_MSAA_SAMPLES_STEP: i32 = 1;
pub const SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MIN: f32 = 0.0;
pub const SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MAX: f32 = 1.0;
pub const SKIN_PREVIEW_MOTION_BLUR_AMOUNT_STEP: f32 = 0.05;
pub const SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MIN: f32 = 0.1;
pub const SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MAX: f32 = 4.0;
pub const SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_STEP: f32 = 0.05;
pub const SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MIN: i32 = 2;
pub const SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MAX: i32 = 16;
pub const SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_STEP: i32 = 1;

const fn default_fullscreen_style() -> FullscreenStyle {
    FullscreenStyle::Borderless
}

const fn default_window_transparency() -> WindowTransparency {
    WindowTransparency::Opaque
}

const fn default_windows_backdrop_type() -> WindowsBackdropType {
    WindowsBackdropType::Auto
}

const fn default_hdr_color_space() -> HdrColorSpace {
    HdrColorSpace::Hdr10St2084
}

const fn default_hdr_ui_white_scale() -> f32 {
    1.0
}

const fn default_hdr_max_cll_nits() -> i32 {
    1000
}

const fn default_hdr_max_fall_nits() -> i32 {
    400
}

const fn default_linux_blur_protocol() -> LinuxBlurProtocol {
    LinuxBlurProtocol::Auto
}

const fn default_macos_visual_effect_material() -> MacosVisualEffectMaterial {
    MacosVisualEffectMaterial::UnderWindowBackground
}

const fn default_macos_visual_effect_blending_mode() -> MacosVisualEffectBlendingMode {
    MacosVisualEffectBlendingMode::BehindWindow
}

const fn default_macos_visual_effect_state() -> MacosVisualEffectState {
    MacosVisualEffectState::Active
}

const fn default_notification_fade_out_seconds() -> f32 {
    DEFAULT_NOTIFICATION_FADE_OUT_SECONDS
}

const fn default_notification_easing() -> NotificationEasing {
    NotificationEasing::CubicInOut
}

const fn default_drag_scroll_enabled() -> bool {
    true
}

/// Whether this is a Steam Deck (or SteamOS), where a game launched from Vertex needs Vertex's window out of the way
/// before the game can take the screen.
pub fn running_on_steam_deck() -> bool {
    if std::env::var_os("SteamDeck").is_some() || std::env::var_os("SteamGamepadUI").is_some() {
        return true;
    }
    std::fs::read_to_string("/etc/os-release")
        .is_ok_and(|text| text.lines().any(|line| line.trim() == "ID=steamos"))
}

fn default_steam_deck_handoff() -> bool {
    running_on_steam_deck()
}

const fn default_follow_console_on_launch() -> bool {
    true
}

const fn default_obs_detection_enabled() -> bool {
    true
}

const fn default_window_corner_radius() -> u8 {
    DEFAULT_WINDOW_CORNER_RADIUS
}

const fn default_ui_opacity_percent() -> u8 {
    100
}

fn serialize_toml_safe_u128<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if *value <= i64::MAX as u128 {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_str(&value.to_string())
    }
}

fn deserialize_toml_safe_u128<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: Deserializer<'de>,
{
    struct U128Visitor;

    impl<'de> Visitor<'de> for U128Visitor {
        type Value = u128;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a non-negative integer or decimal string")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value as u128)
        }

        fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 {
                return Err(E::custom("expected non-negative integer"));
            }
            Ok(value as u128)
        }

        fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 {
                return Err(E::custom("expected non-negative integer"));
            }
            Ok(value as u128)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            value
                .trim()
                .parse::<u128>()
                .map_err(|_| E::custom("expected decimal string for u128 value"))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }
    }

    deserializer.deserialize_any(U128Visitor)
}

/// Launcher configuration persisted as JSON/TOML.
///
/// Values are normalized on load/save via [`Config::normalize`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    low_power_gpu_preferred: bool,
    graphics_adapter_preference_type: GraphicsAdapterPreferenceType,
    graphics_adapter_profile: GraphicsAdapterProfile,
    graphics_adapter_explicit_hash: Option<u64>,
    graphics_api_preference: GraphicsApiPreference,
    streamer_mode_enabled: bool,
    /// Apply streamer mode automatically while OBS is running (not saved). On by default.
    #[serde(default = "default_obs_detection_enabled")]
    obs_detection_enabled: bool,
    /// Keep sign-in tokens in a plain-text file instead of the OS keyring. Off by default.
    insecure_file_account_storage: bool,
    #[serde(default = "default_window_transparency")]
    window_transparency: WindowTransparency,
    #[serde(default = "default_fullscreen_style")]
    fullscreen_style: FullscreenStyle,
    #[serde(default, rename = "window_blur_enabled", skip_serializing)]
    legacy_window_blur_enabled: Option<bool>,
    #[serde(default = "default_windows_backdrop_type")]
    windows_backdrop_type: WindowsBackdropType,
    #[serde(default = "default_linux_blur_protocol")]
    linux_blur_protocol: LinuxBlurProtocol,
    #[serde(default = "default_macos_visual_effect_material")]
    macos_visual_effect_material: MacosVisualEffectMaterial,
    #[serde(default = "default_macos_visual_effect_blending_mode")]
    macos_visual_effect_blending_mode: MacosVisualEffectBlendingMode,
    #[serde(default = "default_macos_visual_effect_state")]
    macos_visual_effect_state: MacosVisualEffectState,
    macos_visual_effect_emphasized: bool,
    #[serde(default = "default_ui_opacity_percent")]
    ui_opacity_percent: u8,
    #[serde(default = "default_window_corner_radius")]
    window_corner_radius: u8,
    /// Tint colour (0xRRGGBB) of the Windows legacy blur and legacy acrylic backdrops.
    #[serde(default)]
    windows_blur_tint_rgb: u32,
    /// Strength of that tint, in percent.
    #[serde(default)]
    windows_blur_tint_opacity_percent: u8,
    #[serde(default, rename = "windows_transparency_level", skip_serializing)]
    legacy_windows_transparency_level: Option<WindowsTransparencyLevel>,
    linux_set_opengl_driver: bool,
    linux_use_zink_driver: bool,
    theme_id: String,
    open_type_features_enabled: bool,
    open_type_features_to_enable: String,
    notification_expiry_bars_empty_left: bool,
    #[serde(default = "default_notification_fade_out_seconds")]
    notification_fade_out_seconds: f32,
    #[serde(default = "default_notification_easing")]
    notification_easing: NotificationEasing,
    ui_font_family: UiFontFamily,
    ui_emoji_font_family: UiEmojiFontFamily,
    ui_cjk_font_family: UiCjkFontFamily,
    text_rendering_path: TextRenderingPath,
    skin_preview_aa_mode: SkinPreviewAaMode,
    skin_preview_texel_aa_mode: SkinPreviewTexelAaMode,
    svg_aa_mode: SvgAaMode,
    skin_preview_msaa_samples: i32,
    skin_preview_motion_blur_enabled: bool,
    skin_preview_motion_blur_amount: f32,
    skin_preview_motion_blur_shutter_frames: f32,
    skin_preview_motion_blur_sample_count: i32,
    skin_preview_fresh_format_enabled: bool,
    skin_preview_3d_layers_enabled: bool,
    hdr_when_available: bool,
    #[serde(default = "default_hdr_color_space")]
    hdr_color_space: HdrColorSpace,
    /// UI reference white relative to the display's SDR white (1.0 = same); raise it to brighten the UI.
    #[serde(default = "default_hdr_ui_white_scale")]
    hdr_ui_white_scale: f32,
    /// MaxCLL reported to the display, in nits.
    #[serde(default = "default_hdr_max_cll_nits")]
    hdr_max_cll_nits: i32,
    /// MaxFALL reported to the display, in nits.
    #[serde(default = "default_hdr_max_fall_nits")]
    hdr_max_fall_nits: i32,
    frame_limiter_enabled: bool,
    discord_rich_presence_enabled: bool,
    #[serde(default)]
    sync_servers_enabled: bool,
    #[serde(default)]
    sync_command_history_enabled: bool,
    #[serde(default)]
    sync_hotbars_enabled: bool,
    #[serde(default)]
    sync_servers_to_all_instances_by_default: bool,
    #[serde(default = "default_drag_scroll_enabled")]
    drag_scroll_enabled: bool,
    #[serde(default = "default_follow_console_on_launch")]
    follow_console_on_launch: bool,
    /// Open the launcher fullscreen. Off by default.
    #[serde(default)]
    start_fullscreen: bool,
    /// Hide the launcher's window while a game runs so the game can take the screen (Steam Deck game mode). On by
    /// default on a Steam Deck.
    #[serde(default = "default_steam_deck_handoff")]
    steam_deck_handoff: bool,
    frame_limit_fps: i32,
    ui_font_size: f32,
    ui_font_weight: i32,
    #[serde(default)]
    typography: TypographySettings,
    include_snapshots_and_betas: bool,
    include_alpha_versions: bool,
    include_experimental_versions: bool,
    force_java_21_minimum: bool,
    #[serde(
        serialize_with = "serialize_toml_safe_u128",
        deserialize_with = "deserialize_toml_safe_u128"
    )]
    default_instance_max_memory_mib: u128,
    default_instance_cli_args: String,
    minecraft_installations_root: PathBuf,
    #[serde(alias = "download_starts_per_second")]
    download_max_concurrent: u32,
    download_speed_limit_enabled: bool,
    download_speed_limit: String,
    curseforge_api_key: String,
    java_8_jvm_path: Option<PathBuf>,
    java_16_jvm_path: Option<PathBuf>,
    java_17_jvm_path: Option<PathBuf>,
    java_21_jvm_path: Option<PathBuf>,
    java_25_jvm_path: Option<PathBuf>,
    gamepad_calibrations: BTreeMap<String, GamepadCalibration>,
}

impl Config {
    /// Returns whether integrated GPU preference is enabled.
    pub fn low_power_gpu_preferred(&self) -> bool {
        self.low_power_gpu_preferred
    }

    pub fn graphics_adapter_preference_type(&self) -> GraphicsAdapterPreferenceType {
        self.graphics_adapter_preference_type
    }

    pub fn set_graphics_adapter_preference_type(&mut self, value: GraphicsAdapterPreferenceType) {
        self.graphics_adapter_preference_type = value;
    }

    pub fn graphics_adapter_profile(&self) -> GraphicsAdapterProfile {
        self.graphics_adapter_profile
    }

    pub fn set_graphics_adapter_profile(&mut self, value: GraphicsAdapterProfile) {
        self.graphics_adapter_profile = value;
        self.low_power_gpu_preferred = matches!(value, GraphicsAdapterProfile::LowPower);
    }

    pub fn graphics_adapter_explicit_hash(&self) -> Option<u64> {
        self.graphics_adapter_explicit_hash
    }

    pub fn set_graphics_adapter_explicit_hash(&mut self, value: Option<u64>) {
        self.graphics_adapter_explicit_hash = value;
    }

    pub fn graphics_api_preference(&self) -> GraphicsApiPreference {
        self.graphics_api_preference
    }

    pub fn set_graphics_api_preference(&mut self, value: GraphicsApiPreference) {
        self.graphics_api_preference = value;
    }

    pub fn streamer_mode_enabled(&self) -> bool {
        self.streamer_mode_enabled
    }

    /// Whether streamer mode is applied automatically while OBS is running.
    pub fn obs_detection_enabled(&self) -> bool {
        self.obs_detection_enabled
    }

    /// Whether sign-in tokens are kept in a plain-text file instead of the OS keyring.
    pub fn insecure_file_account_storage(&self) -> bool {
        self.insecure_file_account_storage
    }

    pub fn set_insecure_file_account_storage(&mut self, enabled: bool) {
        self.insecure_file_account_storage = enabled;
    }

    /// Returns whether launcher-owned Discord Rich Presence is enabled.
    /// Java major version `game_version` needs, honoring the "force Java 21 minimum" setting.
    pub fn effective_required_java_major(&self, game_version: &str) -> Option<u8> {
        let required = JavaRuntimeVersion::required_major_for_game(game_version)?;
        Some(if self.force_java_21_minimum() && required < 21 {
            21
        } else {
            required
        })
    }

    /// Path of the configured Java to launch with: the per-instance override if it is enabled
    /// and available, else the runtime `required_major` needs. `None` means "let the launcher
    /// provision one".
    pub fn choose_java_executable(
        &self,
        override_enabled: bool,
        override_major: Option<u8>,
        required_major: Option<u8>,
    ) -> Option<String> {
        let existing = |major: u8| {
            let runtime = JavaRuntimeVersion::from_major(major)?;
            let path = self.java_runtime_path_ref(runtime)?;
            let text = path.as_os_str().to_string_lossy().trim().to_owned();
            (!text.is_empty() && path.exists()).then_some(text)
        };
        override_major
            .filter(|_| override_enabled)
            .and_then(existing)
            .or_else(|| required_major.and_then(existing))
    }

    /// Whether multiplayer servers are copied between instances.
    pub fn sync_servers_enabled(&self) -> bool {
        self.sync_servers_enabled
    }

    /// Whether launching a game switches to that instance's console.
    pub fn follow_console_on_launch(&self) -> bool {
        self.follow_console_on_launch
    }

    /// Whether the launcher opens fullscreen.
    pub fn start_fullscreen(&self) -> bool {
        self.start_fullscreen
    }

    /// Whether the launcher's window is hidden while a game runs.
    pub fn steam_deck_handoff(&self) -> bool {
        self.steam_deck_handoff
    }

    pub fn set_steam_deck_handoff(&mut self, value: bool) {
        self.steam_deck_handoff = value;
    }

    /// Whether click-dragging scrollable content scrolls it.
    pub fn drag_scroll_enabled(&self) -> bool {
        self.drag_scroll_enabled
    }

    /// Whether `command_history.txt` is merged across instances.
    pub fn sync_command_history_enabled(&self) -> bool {
        self.sync_command_history_enabled
    }

    /// Whether creative hotbars (`hotbar.nbt`) are shared across compatible instances.
    pub fn sync_hotbars_enabled(&self) -> bool {
        self.sync_hotbars_enabled
    }

    /// Whether servers without an explicit scope are copied to every instance.
    pub fn sync_servers_to_all_instances_by_default(&self) -> bool {
        self.sync_servers_to_all_instances_by_default
    }

    pub fn discord_rich_presence_enabled(&self) -> bool {
        self.discord_rich_presence_enabled
    }

    /// Returns currently selected UI font family.
    pub fn ui_font_family(&self) -> UiFontFamily {
        self.ui_font_family.clone()
    }

    /// Returns currently selected emoji font family.
    pub fn ui_emoji_font_family(&self) -> UiEmojiFontFamily {
        self.ui_emoji_font_family.clone()
    }

    /// Sets the emoji font family.
    pub fn set_ui_emoji_font_family(&mut self, value: UiEmojiFontFamily) {
        self.ui_emoji_font_family = value;
    }

    /// The launcher-wide font for CJK characters. A text group can override it.
    pub fn ui_cjk_font_family(&self) -> UiCjkFontFamily {
        self.ui_cjk_font_family.clone()
    }

    pub fn set_ui_cjk_font_family(&mut self, value: UiCjkFontFamily) {
        self.ui_cjk_font_family = value;
    }

    pub fn text_rendering_path(&self) -> TextRenderingPath {
        self.text_rendering_path
    }

    pub fn set_text_rendering_path(&mut self, value: TextRenderingPath) {
        self.text_rendering_path = value;
    }

    /// Returns configured skin preview anti-aliasing mode.
    pub fn skin_preview_aa_mode(&self) -> SkinPreviewAaMode {
        self.skin_preview_aa_mode
    }

    /// Sets skin preview anti-aliasing mode.
    pub fn set_skin_preview_aa_mode(&mut self, mode: SkinPreviewAaMode) {
        self.skin_preview_aa_mode = mode;
    }

    pub fn skin_preview_texel_aa_mode(&self) -> SkinPreviewTexelAaMode {
        self.skin_preview_texel_aa_mode
    }

    pub fn set_skin_preview_texel_aa_mode(&mut self, mode: SkinPreviewTexelAaMode) {
        self.skin_preview_texel_aa_mode = mode;
    }

    /// Returns configured SVG rasterization anti-aliasing mode.
    pub fn svg_aa_mode(&self) -> SvgAaMode {
        self.svg_aa_mode
    }

    /// Sets SVG rasterization anti-aliasing mode.
    pub fn set_svg_aa_mode(&mut self, mode: SvgAaMode) {
        self.svg_aa_mode = mode;
    }

    pub fn skin_preview_msaa_samples(&self) -> i32 {
        self.skin_preview_msaa_samples
    }

    pub fn set_skin_preview_msaa_samples(&mut self, samples: i32) {
        self.skin_preview_msaa_samples =
            samples.clamp(SKIN_PREVIEW_MSAA_SAMPLES_MIN, SKIN_PREVIEW_MSAA_SAMPLES_MAX);
    }

    pub fn skin_preview_motion_blur_enabled(&self) -> bool {
        self.skin_preview_motion_blur_enabled
    }

    pub fn set_skin_preview_motion_blur_enabled(&mut self, enabled: bool) {
        self.skin_preview_motion_blur_enabled = enabled;
    }

    pub fn skin_preview_motion_blur_amount(&self) -> f32 {
        self.skin_preview_motion_blur_amount
    }

    pub fn set_skin_preview_motion_blur_amount(&mut self, amount: f32) {
        self.skin_preview_motion_blur_amount = amount.clamp(
            SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MIN,
            SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MAX,
        );
    }

    pub fn skin_preview_motion_blur_shutter_frames(&self) -> f32 {
        self.skin_preview_motion_blur_shutter_frames
    }

    pub fn set_skin_preview_motion_blur_shutter_frames(&mut self, frames: f32) {
        self.skin_preview_motion_blur_shutter_frames = frames.clamp(
            SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MIN,
            SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MAX,
        );
    }

    pub fn skin_preview_motion_blur_sample_count(&self) -> i32 {
        self.skin_preview_motion_blur_sample_count
    }

    pub fn set_skin_preview_motion_blur_sample_count(&mut self, count: i32) {
        self.skin_preview_motion_blur_sample_count = count.clamp(
            SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MIN,
            SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MAX,
        );
    }

    pub fn skin_preview_fresh_format_enabled(&self) -> bool {
        self.skin_preview_fresh_format_enabled
    }

    pub fn set_skin_preview_fresh_format_enabled(&mut self, enabled: bool) {
        self.skin_preview_fresh_format_enabled = enabled;
    }

    pub fn skin_preview_3d_layers_enabled(&self) -> bool {
        self.skin_preview_3d_layers_enabled
    }

    pub fn set_skin_preview_3d_layers_enabled(&mut self, enabled: bool) {
        self.skin_preview_3d_layers_enabled = enabled;
    }

    pub fn hdr_when_available(&self) -> bool {
        self.hdr_when_available
    }

    pub fn set_hdr_when_available(&mut self, enabled: bool) {
        self.hdr_when_available = enabled;
    }

    /// Returns whether frame limiter is enabled.
    pub fn frame_limiter_enabled(&self) -> bool {
        self.frame_limiter_enabled
    }

    /// Returns configured FPS cap used by frame limiter.
    pub fn frame_limit_fps(&self) -> i32 {
        self.frame_limit_fps
    }

    /// Sets configured FPS cap used by frame limiter.
    pub fn set_frame_limit_fps(&mut self, fps: i32) {
        self.frame_limit_fps = fps.clamp(FRAME_LIMIT_FPS_MIN, FRAME_LIMIT_FPS_MAX);
    }

    /// How F11 and the title bar's fullscreen button take the window fullscreen.
    pub fn fullscreen_style(&self) -> FullscreenStyle {
        self.fullscreen_style
    }

    pub fn set_fullscreen_style(&mut self, value: FullscreenStyle) {
        self.fullscreen_style = value;
    }

    /// Returns the app-wide window transparency mode.
    pub fn window_transparency(&self) -> WindowTransparency {
        self.window_transparency
    }

    /// Sets the app-wide window transparency mode.
    pub fn set_window_transparency(&mut self, value: WindowTransparency) {
        self.window_transparency = value;
    }

    /// Returns whether native platform blur/backdrop effects are requested.
    pub fn window_blur_enabled(&self) -> bool {
        self.window_transparency.uses_native_blur()
    }

    /// Compatibility helper for legacy call sites; prefer [`Self::set_window_transparency`].
    pub fn set_window_blur_enabled(&mut self, enabled: bool) {
        self.window_transparency = if enabled {
            WindowTransparency::Blurred
        } else {
            WindowTransparency::Opaque
        };
    }

    pub fn windows_backdrop_type(&self) -> WindowsBackdropType {
        self.windows_backdrop_type
    }

    pub fn set_windows_backdrop_type(&mut self, value: WindowsBackdropType) {
        self.windows_backdrop_type = value;
    }

    pub fn hdr_color_space(&self) -> HdrColorSpace {
        self.hdr_color_space
    }

    pub fn set_hdr_color_space(&mut self, value: HdrColorSpace) {
        self.hdr_color_space = value;
    }

    pub fn hdr_ui_white_scale(&self) -> f32 {
        self.hdr_ui_white_scale
    }

    pub fn set_hdr_ui_white_scale(&mut self, value: f32) {
        self.hdr_ui_white_scale = value.clamp(HDR_UI_WHITE_SCALE_MIN, HDR_UI_WHITE_SCALE_MAX);
    }

    pub fn hdr_max_cll_nits(&self) -> i32 {
        self.hdr_max_cll_nits
    }

    pub fn set_hdr_max_cll_nits(&mut self, value: i32) {
        self.hdr_max_cll_nits = value.clamp(HDR_LIGHT_LEVEL_NITS_MIN, HDR_LIGHT_LEVEL_NITS_MAX);
    }

    pub fn hdr_max_fall_nits(&self) -> i32 {
        self.hdr_max_fall_nits
    }

    pub fn set_hdr_max_fall_nits(&mut self, value: i32) {
        self.hdr_max_fall_nits = value.clamp(HDR_LIGHT_LEVEL_NITS_MIN, HDR_LIGHT_LEVEL_NITS_MAX);
    }

    pub fn linux_blur_protocol(&self) -> LinuxBlurProtocol {
        self.linux_blur_protocol
    }

    pub fn set_linux_blur_protocol(&mut self, value: LinuxBlurProtocol) {
        self.linux_blur_protocol = value;
    }

    pub fn macos_visual_effect_material(&self) -> MacosVisualEffectMaterial {
        self.macos_visual_effect_material
    }

    pub fn set_macos_visual_effect_material(&mut self, value: MacosVisualEffectMaterial) {
        self.macos_visual_effect_material = value;
    }

    pub fn macos_visual_effect_blending_mode(&self) -> MacosVisualEffectBlendingMode {
        self.macos_visual_effect_blending_mode
    }

    pub fn set_macos_visual_effect_blending_mode(&mut self, value: MacosVisualEffectBlendingMode) {
        self.macos_visual_effect_blending_mode = value;
    }

    pub fn macos_visual_effect_state(&self) -> MacosVisualEffectState {
        self.macos_visual_effect_state
    }

    pub fn set_macos_visual_effect_state(&mut self, value: MacosVisualEffectState) {
        self.macos_visual_effect_state = value;
    }

    pub fn macos_visual_effect_emphasized(&self) -> bool {
        self.macos_visual_effect_emphasized
    }

    pub fn set_macos_visual_effect_emphasized(&mut self, value: bool) {
        self.macos_visual_effect_emphasized = value;
    }

    pub fn ui_opacity_percent(&self) -> u8 {
        self.ui_opacity_percent
    }

    /// Returns the window's corner radius in logical pixels (used while the window is translucent).
    pub fn windows_blur_tint_rgb(&self) -> u32 {
        self.windows_blur_tint_rgb
    }

    pub fn set_windows_blur_tint_rgb(&mut self, value: u32) {
        self.windows_blur_tint_rgb = value & 0x00ff_ffff;
    }

    pub fn windows_blur_tint_opacity_percent(&self) -> u8 {
        self.windows_blur_tint_opacity_percent
    }

    pub fn set_windows_blur_tint_opacity_percent(&mut self, value: u8) {
        self.windows_blur_tint_opacity_percent =
            value.clamp(WINDOWS_BLUR_TINT_OPACITY_MIN, WINDOWS_BLUR_TINT_OPACITY_MAX);
    }

    pub fn window_corner_radius(&self) -> u8 {
        self.window_corner_radius
    }

    pub fn set_window_corner_radius(&mut self, value: u8) {
        self.window_corner_radius = value.clamp(WINDOW_CORNER_RADIUS_MIN, WINDOW_CORNER_RADIUS_MAX);
    }

    pub fn set_ui_opacity_percent(&mut self, value: u8) {
        self.ui_opacity_percent = value.clamp(UI_OPACITY_PERCENT_MIN, UI_OPACITY_PERCENT_MAX);
    }

    /// Returns whether launch commands should explicitly manage the Linux OpenGL driver.
    pub fn linux_set_opengl_driver(&self) -> bool {
        self.linux_set_opengl_driver
    }

    /// Enables or disables Linux OpenGL driver management for launched games.
    pub fn set_linux_set_opengl_driver(&mut self, enabled: bool) {
        self.linux_set_opengl_driver = enabled;
    }

    /// Returns whether Linux OpenGL launches should force Mesa Zink.
    pub fn linux_use_zink_driver(&self) -> bool {
        self.linux_use_zink_driver
    }

    /// Enables or disables Mesa Zink for Linux OpenGL launches.
    pub fn set_linux_use_zink_driver(&mut self, enabled: bool) {
        self.linux_use_zink_driver = enabled;
    }

    /// Returns active theme id.
    pub fn theme_id(&self) -> &str {
        &self.theme_id
    }

    /// Sets active theme id.
    pub fn set_theme_id(&mut self, theme_id: impl Into<String>) {
        self.theme_id = theme_id.into();
    }

    /// Returns whether OpenType feature toggling is enabled.
    pub fn open_type_features_enabled(&self) -> bool {
        self.open_type_features_enabled
    }

    /// Returns comma-separated OpenType feature tags configured by user.
    pub fn open_type_features_to_enable(&self) -> &str {
        &self.open_type_features_to_enable
    }

    pub fn notification_expiry_bars_empty_left(&self) -> bool {
        self.notification_expiry_bars_empty_left
    }

    /// Returns how long, in seconds, a notification takes to fade out once it expires.
    pub fn notification_fade_out_seconds(&self) -> f32 {
        self.notification_fade_out_seconds
    }

    /// Sets the notification fade-out duration in seconds.
    pub fn set_notification_fade_out_seconds(&mut self, seconds: f32) {
        self.notification_fade_out_seconds = seconds.clamp(
            NOTIFICATION_FADE_OUT_SECONDS_MIN,
            NOTIFICATION_FADE_OUT_SECONDS_MAX,
        );
    }

    /// Returns the easing curve used for the notification fade-out.
    pub fn notification_easing(&self) -> NotificationEasing {
        self.notification_easing
    }

    /// Sets the easing curve used for the notification fade-out.
    pub fn set_notification_easing(&mut self, easing: NotificationEasing) {
        self.notification_easing = easing;
    }

    /// Returns configured UI font size in points.
    pub fn ui_font_size(&self) -> f32 {
        self.ui_font_size
    }

    /// Returns configured UI font weight (CSS-like 100..900).
    /// Per-role font size, weight and family overrides.
    pub fn typography(&self) -> &TypographySettings {
        &self.typography
    }

    pub fn typography_mut(&mut self) -> &mut TypographySettings {
        &mut self.typography
    }

    pub fn ui_font_weight(&self) -> i32 {
        self.ui_font_weight
    }

    /// Returns whether snapshots/betas are included in version pickers.
    pub fn include_snapshots_and_betas(&self) -> bool {
        self.include_snapshots_and_betas
    }

    /// Returns whether alpha versions are included in version pickers.
    pub fn include_alpha_versions(&self) -> bool {
        self.include_alpha_versions
    }

    /// Returns whether experimental versions are included in version pickers.
    pub fn include_experimental_versions(&self) -> bool {
        self.include_experimental_versions
    }

    /// Returns whether Java requirements below 21 should be upgraded to 21.
    pub fn force_java_21_minimum(&self) -> bool {
        self.force_java_21_minimum
    }

    /// Returns default per-instance max memory (MiB).
    pub fn default_instance_max_memory_mib(&self) -> u128 {
        self.default_instance_max_memory_mib
    }

    /// Sets default per-instance max memory (MiB), clamped to supported range.
    pub fn set_default_instance_max_memory_mib(&mut self, memory_mib: u128) {
        self.default_instance_max_memory_mib = memory_mib.clamp(
            INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN,
            INSTANCE_DEFAULT_MAX_MEMORY_MIB_MAX,
        );
    }

    /// Mutable access to default per-instance CLI args string.
    pub fn default_instance_cli_args_mut(&mut self) -> &mut String {
        &mut self.default_instance_cli_args
    }

    /// Returns default per-instance CLI args.
    pub fn default_instance_cli_args(&self) -> &str {
        &self.default_instance_cli_args
    }

    /// Returns root directory for instance installations.
    pub fn minecraft_installations_root(&self) -> &str {
        self.minecraft_installations_root
            .as_os_str()
            .to_str()
            .unwrap_or_default()
    }

    /// Returns root directory for instance installations as a path.
    pub fn minecraft_installations_root_path(&self) -> &Path {
        self.minecraft_installations_root.as_path()
    }

    /// Sets root directory for instance installations and normalizes empties.
    pub fn set_minecraft_installations_root(&mut self, path: impl Into<String>) {
        self.set_minecraft_installations_root_path(PathBuf::from(path.into()));
    }

    /// Sets root directory for instance installations from a path value.
    pub fn set_minecraft_installations_root_path(&mut self, path: impl AsRef<Path>) {
        self.minecraft_installations_root = path.as_ref().to_path_buf();
        let default_root = default_minecraft_installations_root_path();
        normalize_required_path(
            &mut self.minecraft_installations_root,
            default_root.as_path(),
        );
    }

    /// Returns max concurrent downloads.
    pub fn download_max_concurrent(&self) -> u32 {
        self.download_max_concurrent
    }

    /// Sets max concurrent downloads, clamped to supported range.
    pub fn set_download_max_concurrent(&mut self, max_concurrent: u32) {
        self.download_max_concurrent =
            max_concurrent.clamp(DOWNLOAD_CONCURRENCY_MIN, DOWNLOAD_CONCURRENCY_MAX);
    }

    /// Returns whether bandwidth limiting is enabled.
    pub fn download_speed_limit_enabled(&self) -> bool {
        self.download_speed_limit_enabled
    }

    /// Enables/disables download bandwidth limiting.
    pub fn set_download_speed_limit_enabled(&mut self, enabled: bool) {
        self.download_speed_limit_enabled = enabled;
    }

    /// Returns configured bandwidth limit text (for example `10mbps`).
    pub fn download_speed_limit(&self) -> &str {
        &self.download_speed_limit
    }

    /// Mutable access to configured bandwidth limit text.
    pub fn download_speed_limit_mut(&mut self) -> &mut String {
        &mut self.download_speed_limit
    }

    /// Returns user-provided CurseForge API key.
    pub fn curseforge_api_key(&self) -> &str {
        &self.curseforge_api_key
    }

    /// Mutable access to user-provided CurseForge API key.
    pub fn curseforge_api_key_mut(&mut self) -> &mut String {
        &mut self.curseforge_api_key
    }

    /// Sets user-provided CurseForge API key.
    pub fn set_curseforge_api_key(&mut self, api_key: impl Into<String>) {
        self.curseforge_api_key = api_key.into().trim().to_owned();
    }

    /// Parses configured bandwidth limit into bits per second when enabled.
    pub fn parsed_download_speed_limit_bps(&self) -> Option<u64> {
        if !self.download_speed_limit_enabled {
            return None;
        }
        parse_bitrate_to_bps(self.download_speed_limit())
    }

    /// Returns user-provided Java runtime path for the requested runtime major.
    pub fn java_runtime_path(&self, runtime: JavaRuntimeVersion) -> Option<&str> {
        self.java_runtime_path_ref(runtime)
            .and_then(|path| path.as_os_str().to_str())
    }

    /// Returns user-provided Java runtime path for the requested runtime major as a path.
    pub fn java_runtime_path_ref(&self, runtime: JavaRuntimeVersion) -> Option<&Path> {
        match runtime {
            JavaRuntimeVersion::Java8 => self.java_8_jvm_path.as_deref(),
            JavaRuntimeVersion::Java16 => self.java_16_jvm_path.as_deref(),
            JavaRuntimeVersion::Java17 => self.java_17_jvm_path.as_deref(),
            JavaRuntimeVersion::Java21 => self.java_21_jvm_path.as_deref(),
            JavaRuntimeVersion::Java25 => self.java_25_jvm_path.as_deref(),
        }
    }

    /// Sets Java runtime path for the requested runtime major.
    pub fn set_java_runtime_path(&mut self, runtime: JavaRuntimeVersion, path: Option<String>) {
        self.set_java_runtime_path_ref(runtime, path.as_deref().map(Path::new));
    }

    /// Sets Java runtime path for the requested runtime major from a path value.
    pub fn set_java_runtime_path_ref(&mut self, runtime: JavaRuntimeVersion, path: Option<&Path>) {
        let path = path.map(Path::to_path_buf);
        match runtime {
            JavaRuntimeVersion::Java8 => self.java_8_jvm_path = path,
            JavaRuntimeVersion::Java16 => self.java_16_jvm_path = path,
            JavaRuntimeVersion::Java17 => self.java_17_jvm_path = path,
            JavaRuntimeVersion::Java21 => self.java_21_jvm_path = path,
            JavaRuntimeVersion::Java25 => self.java_25_jvm_path = path,
        }
    }

    pub fn gamepad_calibration(&self, device_key: &str) -> Option<&GamepadCalibration> {
        self.gamepad_calibrations.get(device_key)
    }

    pub fn gamepad_calibrations(&self) -> &BTreeMap<String, GamepadCalibration> {
        &self.gamepad_calibrations
    }

    pub fn set_gamepad_calibration(
        &mut self,
        device_key: impl Into<String>,
        calibration: GamepadCalibration,
    ) {
        let key = device_key.into().trim().to_owned();
        if key.is_empty() {
            return;
        }
        let mut calibration = calibration;
        calibration.normalize();
        self.gamepad_calibrations.insert(key, calibration);
    }

    /// Normalizes all config values into launcher-supported ranges/defaults.
    pub fn normalize(&mut self) {
        if let Some(legacy_blur_enabled) = self.legacy_window_blur_enabled.take() {
            if self.window_transparency == default_window_transparency() {
                self.window_transparency = if legacy_blur_enabled {
                    WindowTransparency::Blurred
                } else {
                    WindowTransparency::Opaque
                };
            }
        }
        if !WindowTransparency::ALL.contains(&self.window_transparency) {
            self.window_transparency = default_window_transparency();
        }
        if !WindowsBackdropType::ALL.contains(&self.windows_backdrop_type) {
            self.windows_backdrop_type = default_windows_backdrop_type();
        }
        if !LinuxBlurProtocol::ALL.contains(&self.linux_blur_protocol) {
            self.linux_blur_protocol = default_linux_blur_protocol();
        }
        if !MacosVisualEffectMaterial::ALL.contains(&self.macos_visual_effect_material) {
            self.macos_visual_effect_material = default_macos_visual_effect_material();
        }
        if !MacosVisualEffectBlendingMode::ALL.contains(&self.macos_visual_effect_blending_mode) {
            self.macos_visual_effect_blending_mode = default_macos_visual_effect_blending_mode();
        }
        if !MacosVisualEffectState::ALL.contains(&self.macos_visual_effect_state) {
            self.macos_visual_effect_state = default_macos_visual_effect_state();
        }
        if let Some(level) = self.legacy_windows_transparency_level.take() {
            if self.ui_opacity_percent == default_ui_opacity_percent() {
                self.ui_opacity_percent = level.ui_opacity_percent();
            }
        }
        self.ui_font_family.normalize();
        self.ui_emoji_font_family.normalize();
        self.ui_cjk_font_family.normalize();
        if !TextRenderingPath::ALL.contains(&self.text_rendering_path) {
            self.text_rendering_path = TextRenderingPath::Auto;
        }
        self.ui_opacity_percent = self
            .ui_opacity_percent
            .clamp(UI_OPACITY_PERCENT_MIN, UI_OPACITY_PERCENT_MAX);
        self.window_corner_radius = self
            .window_corner_radius
            .clamp(WINDOW_CORNER_RADIUS_MIN, WINDOW_CORNER_RADIUS_MAX);
        self.windows_blur_tint_rgb &= 0x00ff_ffff;
        self.windows_blur_tint_opacity_percent = self
            .windows_blur_tint_opacity_percent
            .clamp(WINDOWS_BLUR_TINT_OPACITY_MIN, WINDOWS_BLUR_TINT_OPACITY_MAX);
        self.ui_font_size = self.ui_font_size.clamp(UI_FONT_SIZE_MIN, UI_FONT_SIZE_MAX);
        self.notification_fade_out_seconds = if self.notification_fade_out_seconds.is_finite() {
            self.notification_fade_out_seconds.clamp(
                NOTIFICATION_FADE_OUT_SECONDS_MIN,
                NOTIFICATION_FADE_OUT_SECONDS_MAX,
            )
        } else {
            DEFAULT_NOTIFICATION_FADE_OUT_SECONDS
        };
        self.typography.normalize();
        self.skin_preview_motion_blur_amount = self.skin_preview_motion_blur_amount.clamp(
            SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MIN,
            SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MAX,
        );
        self.skin_preview_motion_blur_shutter_frames =
            self.skin_preview_motion_blur_shutter_frames.clamp(
                SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MIN,
                SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MAX,
            );
        self.skin_preview_motion_blur_sample_count =
            self.skin_preview_motion_blur_sample_count.clamp(
                SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MIN,
                SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MAX,
            );
        self.skin_preview_msaa_samples = self
            .skin_preview_msaa_samples
            .clamp(SKIN_PREVIEW_MSAA_SAMPLES_MIN, SKIN_PREVIEW_MSAA_SAMPLES_MAX);
        self.hdr_ui_white_scale = if self.hdr_ui_white_scale.is_finite() {
            self.hdr_ui_white_scale.clamp(HDR_UI_WHITE_SCALE_MIN, HDR_UI_WHITE_SCALE_MAX)
        } else {
            default_hdr_ui_white_scale()
        };
        self.hdr_max_cll_nits = self.hdr_max_cll_nits.clamp(HDR_LIGHT_LEVEL_NITS_MIN, HDR_LIGHT_LEVEL_NITS_MAX);
        self.hdr_max_fall_nits = self.hdr_max_fall_nits.clamp(HDR_LIGHT_LEVEL_NITS_MIN, HDR_LIGHT_LEVEL_NITS_MAX);
        self.ui_font_weight = self
            .ui_font_weight
            .clamp(UI_FONT_WEIGHT_MIN, UI_FONT_WEIGHT_MAX);
        self.frame_limit_fps = self
            .frame_limit_fps
            .clamp(FRAME_LIMIT_FPS_MIN, FRAME_LIMIT_FPS_MAX);
        self.default_instance_max_memory_mib = self.default_instance_max_memory_mib.clamp(
            INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN,
            INSTANCE_DEFAULT_MAX_MEMORY_MIB_MAX,
        );
        self.download_max_concurrent = self
            .download_max_concurrent
            .clamp(DOWNLOAD_CONCURRENCY_MIN, DOWNLOAD_CONCURRENCY_MAX);
        self.download_speed_limit = self.download_speed_limit.trim().to_owned();
        self.curseforge_api_key = self.curseforge_api_key.trim().to_owned();
        let default_root = default_minecraft_installations_root_path();
        normalize_required_path(
            &mut self.minecraft_installations_root,
            default_root.as_path(),
        );
        normalize_optional_path(&mut self.java_8_jvm_path);
        normalize_optional_path(&mut self.java_16_jvm_path);
        normalize_optional_path(&mut self.java_17_jvm_path);
        normalize_optional_path(&mut self.java_21_jvm_path);
        normalize_optional_path(&mut self.java_25_jvm_path);
        self.gamepad_calibrations
            .retain(|key, _| !key.trim().is_empty());
        for calibration in self.gamepad_calibrations.values_mut() {
            calibration.normalize();
        }
        if self.theme_id.trim().is_empty() {
            self.theme_id = "matrix_oled".to_owned();
        }
        if !GraphicsAdapterPreferenceType::ALL.contains(&self.graphics_adapter_preference_type) {
            self.graphics_adapter_preference_type =
                GraphicsAdapterPreferenceType::PerformanceProfile;
        }
        if !GraphicsAdapterProfile::ALL.contains(&self.graphics_adapter_profile) {
            self.graphics_adapter_profile = if self.low_power_gpu_preferred {
                GraphicsAdapterProfile::LowPower
            } else {
                GraphicsAdapterProfile::HighPerformance
            };
        }
        if self.graphics_adapter_explicit_hash == Some(0) {
            self.graphics_adapter_explicit_hash = None;
        }
        if !GraphicsApiPreference::ALL.contains(&self.graphics_api_preference) {
            self.graphics_api_preference = GraphicsApiPreference::Auto;
        }
        #[cfg(target_os = "windows")]
        if self.window_transparency.is_translucent()
            && matches!(self.graphics_api_preference, GraphicsApiPreference::Vulkan)
        {
            self.graphics_api_preference = GraphicsApiPreference::Dx12;
        }
    }

    /// Visits each toggle setting with mutable access to its backing value.
    pub fn for_each_toggle_mut(&mut self, mut visit: impl FnMut(ToggleSettingSpec, &mut bool)) {
        let b = self.settings_bindings_mut();
        visit(
            ToggleSettingId::LowPowerGpuPreferred.spec(),
            b.low_power_gpu_preferred,
        );
        visit(
            ToggleSettingId::StreamerModeEnabled.spec(),
            b.streamer_mode_enabled,
        );
        visit(
            ToggleSettingId::ObsDetectionEnabled.spec(),
            b.obs_detection_enabled,
        );
        visit(
            ToggleSettingId::OpenTypeFeaturesEnabled.spec(),
            b.open_type_features_enabled,
        );
        visit(
            ToggleSettingId::NotificationExpiryBarsEmptyLeft.spec(),
            b.notification_expiry_bars_empty_left,
        );
        visit(
            ToggleSettingId::SkinPreviewFreshFormatEnabled.spec(),
            b.skin_preview_fresh_format_enabled,
        );
        visit(
            ToggleSettingId::SkinPreview3dLayersEnabled.spec(),
            b.skin_preview_3d_layers_enabled,
        );
        visit(
            ToggleSettingId::SnapshotsAndBetasEnabled.spec(),
            b.include_snapshots_and_betas,
        );
        visit(
            ToggleSettingId::AlphaVersionsEnabled.spec(),
            b.include_alpha_versions,
        );
        visit(
            ToggleSettingId::ExperimentalVersionsEnabled.spec(),
            b.include_experimental_versions,
        );
        visit(
            ToggleSettingId::ForceJava21Minimum.spec(),
            b.force_java_21_minimum,
        );
        visit(
            ToggleSettingId::FrameLimiterEnabled.spec(),
            b.frame_limiter_enabled,
        );
        visit(
            ToggleSettingId::DiscordRichPresenceEnabled.spec(),
            b.discord_rich_presence_enabled,
        );
        visit(
            ToggleSettingId::SyncServersEnabled.spec(),
            b.sync_servers_enabled,
        );
        visit(
            ToggleSettingId::SyncCommandHistoryEnabled.spec(),
            b.sync_command_history_enabled,
        );
        visit(
            ToggleSettingId::SyncHotbarsEnabled.spec(),
            b.sync_hotbars_enabled,
        );
        visit(
            ToggleSettingId::SyncServersToAllInstancesByDefault.spec(),
            b.sync_servers_to_all_instances_by_default,
        );
        visit(
            ToggleSettingId::DragScrollEnabled.spec(),
            b.drag_scroll_enabled,
        );
        visit(
            ToggleSettingId::FollowConsoleOnLaunch.spec(),
            b.follow_console_on_launch,
        );
        visit(ToggleSettingId::StartFullscreen.spec(), b.start_fullscreen);
        visit(ToggleSettingId::SteamDeckHandoff.spec(), b.steam_deck_handoff);
    }

    /// Visits each dropdown setting with mutable access to its backing value.
    pub fn for_each_dropdown_mut(
        &mut self,
        mut visit: impl FnMut(DropdownSettingSpec, &mut UiFontFamily),
    ) {
        let b = self.settings_bindings_mut();
        visit(DropdownSettingId::UiFontFamily.spec(), b.ui_font_family);
    }

    /// Visits each float setting with mutable access to its backing value.
    pub fn for_each_float_mut(&mut self, mut visit: impl FnMut(FloatSettingSpec, &mut f32)) {
        let b = self.settings_bindings_mut();
        visit(FloatSettingId::UiFontSize.spec(), b.ui_font_size);
        visit(
            FloatSettingId::NotificationFadeOutSeconds.spec(),
            b.notification_fade_out_seconds,
        );
        visit(
            FloatSettingId::SkinPreviewMotionBlurAmount.spec(),
            b.skin_preview_motion_blur_amount,
        );
        visit(
            FloatSettingId::SkinPreviewMotionBlurShutterFrames.spec(),
            b.skin_preview_motion_blur_shutter_frames,
        );
    }

    /// Visits each integer setting with mutable access to its backing value.
    pub fn for_each_int_mut(&mut self, mut visit: impl FnMut(IntSettingSpec, &mut i32)) {
        let b = self.settings_bindings_mut();
        visit(IntSettingId::UiFontWeight.spec(), b.ui_font_weight);
        visit(IntSettingId::FrameLimitFps.spec(), b.frame_limit_fps);
        visit(
            IntSettingId::SkinPreviewMsaaSamples.spec(),
            b.skin_preview_msaa_samples,
        );
        visit(
            IntSettingId::SkinPreviewMotionBlurSampleCount.spec(),
            b.skin_preview_motion_blur_sample_count,
        );
    }

    /// Visits each text setting with mutable access to its backing value.
    pub fn for_each_text_mut(&mut self, mut visit: impl FnMut(TextSettingSpec, &mut String)) {
        let b = self.settings_bindings_mut();
        visit(
            TextSettingId::OpenTypeFeaturesToEnable.spec(),
            b.open_type_features_to_enable,
        );
    }
}

fn normalize_optional_path(path: &mut Option<PathBuf>) {
    *path = path
        .as_ref()
        .map(|value| value.as_os_str().to_string_lossy().trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
}

fn normalize_required_path(path: &mut PathBuf, fallback: &Path) {
    let normalized = path.as_os_str().to_string_lossy().trim().to_owned();
    if normalized.is_empty() {
        *path = fallback.to_path_buf();
    } else {
        *path = PathBuf::from(normalized);
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            low_power_gpu_preferred: true,
            graphics_adapter_preference_type: GraphicsAdapterPreferenceType::PerformanceProfile,
            graphics_adapter_profile: GraphicsAdapterProfile::LowPower,
            graphics_adapter_explicit_hash: None,
            graphics_api_preference: GraphicsApiPreference::Auto,
            streamer_mode_enabled: false,
            obs_detection_enabled: default_obs_detection_enabled(),
            insecure_file_account_storage: false,
            window_transparency: default_window_transparency(),
            fullscreen_style: default_fullscreen_style(),
            legacy_window_blur_enabled: None,
            windows_backdrop_type: default_windows_backdrop_type(),
            linux_blur_protocol: default_linux_blur_protocol(),
            macos_visual_effect_material: default_macos_visual_effect_material(),
            macos_visual_effect_blending_mode: default_macos_visual_effect_blending_mode(),
            macos_visual_effect_state: default_macos_visual_effect_state(),
            macos_visual_effect_emphasized: false,
            ui_opacity_percent: default_ui_opacity_percent(),
            window_corner_radius: default_window_corner_radius(),
            windows_blur_tint_rgb: 0,
            windows_blur_tint_opacity_percent: 0,
            legacy_windows_transparency_level: None,
            linux_set_opengl_driver: false,
            linux_use_zink_driver: false,
            theme_id: "matrix_oled".to_owned(),
            open_type_features_enabled: true,
            open_type_features_to_enable: String::new(),
            notification_expiry_bars_empty_left: false,
            notification_fade_out_seconds: default_notification_fade_out_seconds(),
            notification_easing: default_notification_easing(),
            ui_font_family: UiFontFamily::included_default(),
            ui_emoji_font_family: UiEmojiFontFamily::included_default(),
            ui_cjk_font_family: UiCjkFontFamily::included_default(),
            text_rendering_path: TextRenderingPath::Auto,
            skin_preview_aa_mode: SkinPreviewAaMode::Fxaa,
            skin_preview_texel_aa_mode: SkinPreviewTexelAaMode::Off,
            svg_aa_mode: SvgAaMode::Balanced,
            skin_preview_msaa_samples: 4,
            skin_preview_motion_blur_enabled: false,
            skin_preview_motion_blur_amount: 0.15,
            skin_preview_motion_blur_shutter_frames: 0.75,
            skin_preview_motion_blur_sample_count: 5,
            skin_preview_fresh_format_enabled: false,
            skin_preview_3d_layers_enabled: false,
            hdr_when_available: false,
            hdr_color_space: default_hdr_color_space(),
            hdr_ui_white_scale: default_hdr_ui_white_scale(),
            hdr_max_cll_nits: default_hdr_max_cll_nits(),
            hdr_max_fall_nits: default_hdr_max_fall_nits(),
            frame_limiter_enabled: false,
            discord_rich_presence_enabled: true,
            sync_servers_enabled: false,
            sync_command_history_enabled: false,
            sync_hotbars_enabled: false,
            sync_servers_to_all_instances_by_default: false,
            drag_scroll_enabled: true,
            follow_console_on_launch: true,
            start_fullscreen: false,
            // Off here; a first run (and a saved config that predates the setting) turns it on for a Steam Deck.
            steam_deck_handoff: false,
            frame_limit_fps: 120,
            ui_font_size: 18.0,
            ui_font_weight: 400,
            typography: TypographySettings::default(),
            include_snapshots_and_betas: false,
            include_alpha_versions: false,
            include_experimental_versions: false,
            force_java_21_minimum: true,
            default_instance_max_memory_mib: 4096,
            default_instance_cli_args: String::new(),
            minecraft_installations_root: default_minecraft_installations_root_path(),
            download_max_concurrent: DEFAULT_DOWNLOAD_CONCURRENCY,
            download_speed_limit_enabled: false,
            download_speed_limit: String::new(),
            curseforge_api_key: String::new(),
            java_8_jvm_path: None,
            java_16_jvm_path: None,
            java_17_jvm_path: None,
            java_21_jvm_path: None,
            java_25_jvm_path: None,
            gamepad_calibrations: BTreeMap::new(),
        }
    }
}

/// The OpenType feature tags used when the feature list setting is empty.
pub const DEFAULT_OPEN_TYPE_FEATURE_TAGS: &str = "kern, liga, calt, onum, pnum";

/// Parses human input like `10mbps` into bits-per-second.
///
/// Supported suffixes: `kbps`, `mbps`, `gbps`, `tbps` (case-insensitive).
pub fn parse_bitrate_to_bps(value: &str) -> Option<u64> {
    // Accepts what people actually type: "250Mbps", "250 Mbps", "250mbit/s", "30MB/s", "30 MiB/s", "1.5 Gbps", or a bare
    // "250" (taken as Mbps). A lowercase or spelled-out "b"/"bit" means bits, an uppercase "B" or "byte" means bytes
    // (8 bits); "i" prefixes (KiB, MiB...) are binary. The result is always bits per second.
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let split = trimmed
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(trimmed.len());
    let (number, unit) = trimmed.split_at(split);
    let quantity = number.replace(',', ".").parse::<f64>().ok()?;
    if !quantity.is_finite() || quantity <= 0.0 {
        return None;
    }
    let unit: String = unit.chars().filter(|c| !c.is_whitespace()).collect();
    let unit = unit
        .strip_suffix("/s")
        .or_else(|| unit.strip_suffix("ps"))
        .or_else(|| unit.strip_suffix("PS"))
        .or_else(|| unit.strip_suffix("/S"))
        .unwrap_or(&unit);
    let multiplier = if unit.is_empty() {
        // A bare number: megabits, like the placeholder.
        1_000_000.0
    } else {
        let (prefix, rest) = unit.split_at(unit.chars().next().map_or(0, char::len_utf8));
        let scale_char = prefix.to_ascii_lowercase();
        let (scale_known, rest) = match scale_char.as_str() {
            "k" | "m" | "g" | "t" => (true, rest),
            _ => (false, unit),
        };
        let binary = scale_known && (rest.starts_with('i') || rest.starts_with('I'));
        let rest = if binary { &rest[1..] } else { rest };
        let base: f64 = if binary { 1024.0 } else { 1000.0 };
        let scale = if !scale_known {
            1.0
        } else {
            match scale_char.as_str() {
                "k" => base,
                "m" => base.powi(2),
                "g" => base.powi(3),
                _ => base.powi(4),
            }
        };
        let lower = rest.to_ascii_lowercase();
        let per_unit = if rest == "B" || lower == "byte" || lower == "bytes" {
            8.0
        } else if rest == "b" || lower == "bit" || lower == "bits" || (rest.is_empty() && scale_known) {
            // "250M" alone reads as megabits, matching the bps default.
            1.0
        } else {
            return None;
        };
        scale * per_unit
    };
    let bps = quantity * multiplier;
    if !bps.is_finite() || bps < 1.0 || bps > u64::MAX as f64 {
        return None;
    }
    Some(bps.round() as u64)
}

/// `bps` as a short human rate for display: "250 Mbps (31.3 MB/s)".
pub fn describe_bitrate(bps: u64) -> String {
    let bits = bps as f64;
    let (value, unit) = if bits >= 1e12 {
        (bits / 1e12, "Tbps")
    } else if bits >= 1e9 {
        (bits / 1e9, "Gbps")
    } else if bits >= 1e6 {
        (bits / 1e6, "Mbps")
    } else {
        (bits / 1e3, "Kbps")
    };
    let bytes = bits / 8.0;
    let (byte_value, byte_unit) = if bytes >= 1e9 {
        (bytes / 1e9, "GB/s")
    } else if bytes >= 1e6 {
        (bytes / 1e6, "MB/s")
    } else {
        (bytes / 1e3, "KB/s")
    };
    format!("{} {unit} ({} {byte_unit})", trim_number(value), trim_number(byte_value))
}

fn trim_number(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod bitrate_tests {
    use super::{describe_bitrate, parse_bitrate_to_bps};

    #[test]
    fn accepts_the_usual_spellings() {
        assert_eq!(parse_bitrate_to_bps("250Mbps"), Some(250_000_000));
        assert_eq!(parse_bitrate_to_bps(" 250 Mbps "), Some(250_000_000));
        assert_eq!(parse_bitrate_to_bps("250mbps"), Some(250_000_000));
        assert_eq!(parse_bitrate_to_bps("250 Mbit/s"), Some(250_000_000));
        assert_eq!(parse_bitrate_to_bps("250"), Some(250_000_000));
        assert_eq!(parse_bitrate_to_bps("250M"), Some(250_000_000));
        assert_eq!(parse_bitrate_to_bps("1.5 Gbps"), Some(1_500_000_000));
        assert_eq!(parse_bitrate_to_bps("1,5 Gbps"), Some(1_500_000_000));
        assert_eq!(parse_bitrate_to_bps("500Kbps"), Some(500_000));
        assert_eq!(parse_bitrate_to_bps("2 Tbps"), Some(2_000_000_000_000));
    }

    #[test]
    fn bytes_are_eight_bits() {
        assert_eq!(parse_bitrate_to_bps("30MB/s"), Some(240_000_000));
        assert_eq!(parse_bitrate_to_bps("30 MBps"), Some(240_000_000));
        assert_eq!(parse_bitrate_to_bps("1 MiB/s"), Some(8 * 1024 * 1024));
        assert_eq!(parse_bitrate_to_bps("512 KiB/s"), Some(8 * 512 * 1024));
    }

    #[test]
    fn rejects_nonsense() {
        assert_eq!(parse_bitrate_to_bps(""), None);
        assert_eq!(parse_bitrate_to_bps("fast"), None);
        assert_eq!(parse_bitrate_to_bps("0 Mbps"), None);
        assert_eq!(parse_bitrate_to_bps("-5 Mbps"), None);
        assert_eq!(parse_bitrate_to_bps("10 Xbps"), None);
    }

    #[test]
    fn describes_rates() {
        assert_eq!(describe_bitrate(250_000_000), "250 Mbps (31.25 MB/s)");
        assert_eq!(describe_bitrate(1_500_000_000), "1.5 Gbps (187.5 MB/s)");
    }
}

/// Result of loading configuration from disk.
#[derive(Clone, Debug)]
pub enum LoadConfigResult {
    Loaded(Config),
    Missing { default_format: ConfigFormat },
}

fn default_minecraft_installations_root_path() -> PathBuf {
    launcher_paths::installations_root()
}

fn config_base_path() -> String {
    launcher_paths::config_base_path()
        .to_string_lossy()
        .into_owned()
}

fn legacy_cwd_config_base_path() -> &'static str {
    "config"
}

fn find_existing_config_path(base: &str) -> Option<String> {
    if std::path::Path::new(&format!("{base}.json")).exists() {
        Some(format!("{base}.json"))
    } else if std::path::Path::new(&format!("{base}.toml")).exists() {
        Some(format!("{base}.toml"))
    } else {
        None
    }
}

fn resolve_existing_config_path(base: &str) -> Option<String> {
    if let Some(path) = find_existing_config_path(base) {
        return Some(path);
    }

    if launcher_paths::portable_root().is_some() {
        return None;
    }

    if let Some(legacy_base) = launcher_paths::legacy_config_base_path() {
        let legacy_base = legacy_base.to_string_lossy().into_owned();
        if legacy_base != base
            && let Some(path) = find_existing_config_path(&legacy_base)
        {
            return Some(path);
        }
    }

    let legacy_base = legacy_cwd_config_base_path();
    if legacy_base == base {
        return None;
    }

    find_existing_config_path(legacy_base)
}

fn config_extension_for_path(path: &str) -> &'static str {
    if path.ends_with(".toml") {
        ConfigFormat::Toml.extension()
    } else {
        ConfigFormat::Json.extension()
    }
}

fn preferred_config_save_path(base: &str) -> String {
    if let Some(path) = find_existing_config_path(base) {
        return path;
    }

    if launcher_paths::portable_root().is_none() {
        if let Some(legacy_base) = launcher_paths::legacy_config_base_path() {
            let legacy_base = legacy_base.to_string_lossy().into_owned();
            if let Some(path) = find_existing_config_path(&legacy_base) {
                return format!("{base}.{}", config_extension_for_path(&path));
            }
        }

        if let Some(path) = find_existing_config_path(legacy_cwd_config_base_path()) {
            return format!("{base}.{}", config_extension_for_path(&path));
        }
    }

    format!("{base}.json")
}

fn parse_config_contents(path: &str, contents: &str) -> Option<Config> {
    if path.ends_with(".json") {
        match serde_json::from_str::<Config>(contents) {
            Ok(config) => Some(config),
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/config",
                    path,
                    error = %err,
                    "failed to parse JSON config; using defaults"
                );
                None
            }
        }
    } else {
        match toml::from_str::<Config>(contents) {
            Ok(config) => Some(config),
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/config",
                    path,
                    error = %err,
                    "failed to parse TOML config; using defaults"
                );
                None
            }
        }
    }
}

fn construct_new_config(path: &str, conf: &Config) -> Result<(), IOError> {
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        tracing::debug!(
            target: "vertexlauncher/io",
            op = "create_dir_all",
            path = %parent.display(),
            context = "ensure config directory"
        );
        std::fs::create_dir_all(parent)?;
    }

    let format = if path.ends_with(".toml") {
        ConfigFormat::Toml
    } else {
        ConfigFormat::Json
    };

    match format {
        ConfigFormat::Json => {
            let value =
                serde_json::to_string_pretty(conf).map_err(|e| IOError::other(e.to_string()))?;
            tracing::debug!(target: "vertexlauncher/io", op = "file_create", path = %path, context = "save config json");
            let mut file = std::fs::File::create(path)?;
            file.write_all(value.as_bytes())?;
        }
        ConfigFormat::Toml => {
            let value = toml::to_string_pretty(conf).map_err(|e| IOError::other(e.to_string()))?;
            tracing::debug!(target: "vertexlauncher/io", op = "file_create", path = %path, context = "save config toml");
            let mut file = std::fs::File::create(path)?;
            file.write_all(value.as_bytes())?;
        }
    };

    Ok(())
}

/// Creates and persists a default config file in the selected format.
pub fn create_default_config(format: ConfigFormat) -> Result<Config, IOError> {
    let config_path = format!("{}.{}", config_base_path(), format.extension());
    let mut config = Config::default();
    config.normalize();
    construct_new_config(&config_path, &config)?;
    tracing::info!(
        target: "vertexlauncher/config",
        path = %config_path,
        format = format.extension(),
        "created default config"
    );
    Ok(config)
}

/// Saves the given config to the existing config file path (or JSON by default).
pub fn save_config(config: &Config) -> Result<(), IOError> {
    let mut normalized = config.clone();
    normalized.normalize();

    let base = config_base_path();
    let path = preferred_config_save_path(&base);
    construct_new_config(&path, &normalized)?;
    tracing::debug!(
        target: "vertexlauncher/config",
        path = %path,
        "saved launcher config"
    );
    Ok(())
}

/// Loads config from disk if present, otherwise reports the default format choice.
///
/// Parse/read failures fall back to normalized defaults and emit warnings.
pub fn load_config() -> LoadConfigResult {
    let base = config_base_path();

    match resolve_existing_config_path(&base) {
        Some(path) => {
            tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path, context = "load config");
            let contents = match std::fs::read_to_string(&path) {
                Ok(contents) => contents,
                Err(err) => {
                    tracing::warn!(
                        target: "vertexlauncher/config",
                        path = %path,
                        error = %err,
                        "failed to read config file; using defaults"
                    );
                    String::new()
                }
            };
            let mut parsed = parse_config_contents(&path, &contents).unwrap_or_default();

            parsed.normalize();
            tracing::debug!(
                target: "vertexlauncher/config",
                path = %path,
                "loaded launcher config"
            );
            LoadConfigResult::Loaded(parsed)
        }
        None => {
            tracing::debug!(
                target: "vertexlauncher/config",
                "config file not found; prompting for default format selection"
            );
            LoadConfigResult::Missing {
                default_format: ConfigFormat::Json,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_font_family_deserializes_legacy_enum_value() {
        let family: UiFontFamily = serde_json::from_str("\"jetbrains_mono\"").unwrap();
        assert_eq!(family.label(), "JetBrains Mono");
    }

    #[test]
    fn ui_font_family_serializes_as_plain_family_name() {
        let family = UiFontFamily::new("Cascadia Code");
        let serialized = serde_json::to_string(&family).unwrap();
        assert_eq!(serialized, "\"Cascadia Code\"");
    }

    #[test]
    fn config_toml_serializes_u128_memory_field_as_integer() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(serialized.contains("default_instance_max_memory_mib = 4096"));
    }

    #[test]
    fn config_toml_serializes_large_u128_memory_field_as_string() {
        let value = i64::MAX as u128 + 1;
        let mut config = Config::default();
        config.default_instance_max_memory_mib = value;

        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(
            serialized.contains(format!("default_instance_max_memory_mib = \"{value}\"").as_str())
        );
    }

    #[test]
    fn config_toml_deserializes_string_u128_memory_field() {
        let value = i64::MAX as u128 + 1;
        let serialized = format!("default_instance_max_memory_mib = \"{value}\"");

        let config: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(config.default_instance_max_memory_mib, value);
    }
}
