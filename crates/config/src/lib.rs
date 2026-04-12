use app_paths as launcher_paths;
mod config_format;
mod gamepad_calibration;
mod graphics_adapter_preference;
mod graphics_api_preference;
mod java_runtime_version;
mod setting_specs;
mod skin_preview_aa_mode;
mod skin_preview_texel_aa_mode;
mod svg_aa_mode;
mod text_rendering_path;
mod ui_fonts;
mod windows_backdrop_type;
mod windows_transparency_level;

pub use config_format::ConfigFormat;
pub use gamepad_calibration::GamepadCalibration;
pub use graphics_adapter_preference::{GraphicsAdapterPreferenceType, GraphicsAdapterProfile};
pub use graphics_api_preference::GraphicsApiPreference;
pub use java_runtime_version::JavaRuntimeVersion;
pub use setting_specs::{
    DropdownSettingId, DropdownSettingSpec, FloatSettingId, FloatSettingSpec, IntSettingId,
    IntSettingSpec, TextSettingId, TextSettingSpec, ToggleSettingId, ToggleSettingSpec,
};
pub use skin_preview_aa_mode::SkinPreviewAaMode;
pub use skin_preview_texel_aa_mode::SkinPreviewTexelAaMode;
pub use svg_aa_mode::SvgAaMode;
pub use text_rendering_path::TextRenderingPath;
pub use ui_fonts::{UiEmojiFontFamily, UiFontFamily};
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
pub const UI_FONT_WEIGHT_MIN: i32 = 100;
pub const UI_FONT_WEIGHT_MAX: i32 = 900;
pub const UI_FONT_WEIGHT_STEP: i32 = 100;
pub const INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN: u128 = 512;
pub const INSTANCE_DEFAULT_MAX_MEMORY_MIB_MAX: u128 = 1_048_576;
pub const INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP: u128 = 256;
pub const DOWNLOAD_CONCURRENCY_MIN: u32 = 1;
pub const DOWNLOAD_CONCURRENCY_MAX: u32 = 128;
pub const DEFAULT_DOWNLOAD_CONCURRENCY: u32 = 8;
pub const FRAME_LIMIT_FPS_MIN: i32 = 30;
pub const FRAME_LIMIT_FPS_MAX: i32 = 240;
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

const fn default_windows_backdrop_type() -> WindowsBackdropType {
    WindowsBackdropType::Auto
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
    window_blur_enabled: bool,
    #[serde(default = "default_windows_backdrop_type")]
    windows_backdrop_type: WindowsBackdropType,
    #[serde(default = "default_ui_opacity_percent")]
    ui_opacity_percent: u8,
    #[serde(default, rename = "windows_transparency_level", skip_serializing)]
    legacy_windows_transparency_level: Option<WindowsTransparencyLevel>,
    linux_set_opengl_driver: bool,
    linux_use_zink_driver: bool,
    theme_id: String,
    open_type_features_enabled: bool,
    open_type_features_to_enable: String,
    notification_expiry_bars_empty_left: bool,
    ui_font_family: UiFontFamily,
    ui_emoji_font_family: UiEmojiFontFamily,
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
    frame_limiter_enabled: bool,
    discord_rich_presence_enabled: bool,
    frame_limit_fps: i32,
    ui_font_size: f32,
    ui_font_weight: i32,
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

    /// Returns whether launcher-owned Discord Rich Presence is enabled.
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

    /// Returns whether platform blur effects are enabled.
    pub fn window_blur_enabled(&self) -> bool {
        self.window_blur_enabled
    }

    /// Enables or disables platform blur effects.
    pub fn set_window_blur_enabled(&mut self, enabled: bool) {
        self.window_blur_enabled = enabled;
    }

    pub fn windows_backdrop_type(&self) -> WindowsBackdropType {
        self.windows_backdrop_type
    }

    pub fn set_windows_backdrop_type(&mut self, value: WindowsBackdropType) {
        self.windows_backdrop_type = value;
    }

    pub fn ui_opacity_percent(&self) -> u8 {
        self.ui_opacity_percent
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

    /// Returns configured UI font size in points.
    pub fn ui_font_size(&self) -> f32 {
        self.ui_font_size
    }

    /// Returns configured UI font weight (CSS-like 100..900).
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
        if !WindowsBackdropType::ALL.contains(&self.windows_backdrop_type) {
            self.windows_backdrop_type = default_windows_backdrop_type();
        }
        if let Some(level) = self.legacy_windows_transparency_level.take() {
            if self.ui_opacity_percent == default_ui_opacity_percent() {
                self.ui_opacity_percent = level.ui_opacity_percent();
            }
        }
        self.ui_font_family.normalize();
        self.ui_emoji_font_family.normalize();
        if !TextRenderingPath::ALL.contains(&self.text_rendering_path) {
            self.text_rendering_path = TextRenderingPath::Auto;
        }
        self.ui_opacity_percent = self
            .ui_opacity_percent
            .clamp(UI_OPACITY_PERCENT_MIN, UI_OPACITY_PERCENT_MAX);
        self.ui_font_size = self.ui_font_size.clamp(UI_FONT_SIZE_MIN, UI_FONT_SIZE_MAX);
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
    }

    /// Visits each toggle setting with mutable access to its backing value.
    pub fn for_each_toggle_mut(&mut self, mut visit: impl FnMut(ToggleSettingSpec, &mut bool)) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred,
            graphics_adapter_preference_type: _,
            graphics_adapter_profile: _,
            graphics_adapter_explicit_hash: _,
            graphics_api_preference: _,
            streamer_mode_enabled,
            window_blur_enabled,
            windows_backdrop_type: _,
            ui_opacity_percent: _,
            legacy_windows_transparency_level: _,
            linux_set_opengl_driver: _,
            linux_use_zink_driver: _,
            theme_id: _,
            open_type_features_enabled,
            open_type_features_to_enable: _,
            notification_expiry_bars_empty_left,
            ui_font_family: _,
            ui_emoji_font_family: _,
            text_rendering_path: _,
            skin_preview_aa_mode: _,
            skin_preview_texel_aa_mode: _,
            svg_aa_mode: _,
            skin_preview_msaa_samples: _,
            skin_preview_motion_blur_enabled: _,
            skin_preview_motion_blur_amount: _,
            skin_preview_motion_blur_shutter_frames: _,
            skin_preview_motion_blur_sample_count: _,
            skin_preview_fresh_format_enabled,
            skin_preview_3d_layers_enabled,
            hdr_when_available: _,
            ui_font_size: _,
            ui_font_weight: _,
            include_snapshots_and_betas,
            include_alpha_versions,
            include_experimental_versions,
            force_java_21_minimum,
            frame_limiter_enabled,
            discord_rich_presence_enabled,
            default_instance_max_memory_mib: _,
            default_instance_cli_args: _,
            minecraft_installations_root: _,
            download_max_concurrent: _,
            download_speed_limit_enabled: _,
            download_speed_limit: _,
            frame_limit_fps: _,
            java_8_jvm_path: _,
            java_16_jvm_path: _,
            java_17_jvm_path: _,
            java_21_jvm_path: _,
            java_25_jvm_path: _,
            curseforge_api_key: _,
            gamepad_calibrations: _,
        } = self;

        visit(
            ToggleSettingId::LowPowerGpuPreferred.spec(),
            low_power_gpu_preferred,
        );
        visit(
            ToggleSettingId::StreamerModeEnabled.spec(),
            streamer_mode_enabled,
        );
        visit(
            ToggleSettingId::WindowBlurEnabled.spec(),
            window_blur_enabled,
        );
        visit(
            ToggleSettingId::OpenTypeFeaturesEnabled.spec(),
            open_type_features_enabled,
        );
        visit(
            ToggleSettingId::NotificationExpiryBarsEmptyLeft.spec(),
            notification_expiry_bars_empty_left,
        );
        visit(
            ToggleSettingId::SkinPreviewFreshFormatEnabled.spec(),
            skin_preview_fresh_format_enabled,
        );
        visit(
            ToggleSettingId::SkinPreview3dLayersEnabled.spec(),
            skin_preview_3d_layers_enabled,
        );
        visit(
            ToggleSettingId::SnapshotsAndBetasEnabled.spec(),
            include_snapshots_and_betas,
        );
        visit(ToggleSettingId::AlphaVersionsEnabled.spec(), include_alpha_versions);
        visit(
            ToggleSettingId::ExperimentalVersionsEnabled.spec(),
            include_experimental_versions,
        );
        visit(
            ToggleSettingId::ForceJava21Minimum.spec(),
            force_java_21_minimum,
        );
        visit(
            ToggleSettingId::FrameLimiterEnabled.spec(),
            frame_limiter_enabled,
        );
        visit(
            ToggleSettingId::DiscordRichPresenceEnabled.spec(),
            discord_rich_presence_enabled,
        );
    }

    /// Visits each dropdown setting with mutable access to its backing value.
    pub fn for_each_dropdown_mut(
        &mut self,
        mut visit: impl FnMut(DropdownSettingSpec, &mut UiFontFamily),
    ) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred: _,
            graphics_adapter_preference_type: _,
            graphics_adapter_profile: _,
            graphics_adapter_explicit_hash: _,
            graphics_api_preference: _,
            streamer_mode_enabled: _,
            window_blur_enabled: _,
            windows_backdrop_type: _,
            ui_opacity_percent: _,
            legacy_windows_transparency_level: _,
            linux_set_opengl_driver: _,
            linux_use_zink_driver: _,
            theme_id: _,
            open_type_features_enabled: _,
            open_type_features_to_enable: _,
            notification_expiry_bars_empty_left: _,
            ui_font_family,
            ui_emoji_font_family: _,
            text_rendering_path: _,
            skin_preview_aa_mode: _,
            skin_preview_texel_aa_mode: _,
            svg_aa_mode: _,
            skin_preview_msaa_samples: _,
            skin_preview_motion_blur_enabled: _,
            skin_preview_motion_blur_amount: _,
            skin_preview_motion_blur_shutter_frames: _,
            skin_preview_motion_blur_sample_count: _,
            skin_preview_fresh_format_enabled: _,
            skin_preview_3d_layers_enabled: _,
            hdr_when_available: _,
            ui_font_size: _,
            ui_font_weight: _,
            include_snapshots_and_betas: _,
            include_alpha_versions: _,
            include_experimental_versions: _,
            force_java_21_minimum: _,
            frame_limiter_enabled: _,
            discord_rich_presence_enabled: _,
            frame_limit_fps: _,
            default_instance_max_memory_mib: _,
            default_instance_cli_args: _,
            minecraft_installations_root: _,
            download_max_concurrent: _,
            download_speed_limit_enabled: _,
            download_speed_limit: _,
            curseforge_api_key: _,
            java_8_jvm_path: _,
            java_16_jvm_path: _,
            java_17_jvm_path: _,
            java_21_jvm_path: _,
            java_25_jvm_path: _,
            gamepad_calibrations: _,
        } = self;

        visit(DropdownSettingId::UiFontFamily.spec(), ui_font_family);
    }

    /// Visits each float setting with mutable access to its backing value.
    pub fn for_each_float_mut(&mut self, mut visit: impl FnMut(FloatSettingSpec, &mut f32)) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred: _,
            graphics_adapter_preference_type: _,
            graphics_adapter_profile: _,
            graphics_adapter_explicit_hash: _,
            graphics_api_preference: _,
            streamer_mode_enabled: _,
            window_blur_enabled: _,
            windows_backdrop_type: _,
            ui_opacity_percent: _,
            legacy_windows_transparency_level: _,
            linux_set_opengl_driver: _,
            linux_use_zink_driver: _,
            theme_id: _,
            open_type_features_enabled: _,
            open_type_features_to_enable: _,
            notification_expiry_bars_empty_left: _,
            ui_font_family: _,
            ui_emoji_font_family: _,
            text_rendering_path: _,
            skin_preview_aa_mode: _,
            skin_preview_texel_aa_mode: _,
            svg_aa_mode: _,
            skin_preview_msaa_samples: _,
            skin_preview_motion_blur_enabled: _,
            skin_preview_motion_blur_amount,
            skin_preview_motion_blur_shutter_frames,
            skin_preview_fresh_format_enabled: _,
            skin_preview_3d_layers_enabled: _,
            hdr_when_available: _,
            ui_font_size,
            ui_font_weight: _,
            include_snapshots_and_betas: _,
            include_alpha_versions: _,
            include_experimental_versions: _,
            force_java_21_minimum: _,
            frame_limiter_enabled: _,
            discord_rich_presence_enabled: _,
            frame_limit_fps: _,
            skin_preview_motion_blur_sample_count: _,
            default_instance_max_memory_mib: _,
            default_instance_cli_args: _,
            minecraft_installations_root: _,
            download_max_concurrent: _,
            download_speed_limit_enabled: _,
            download_speed_limit: _,
            curseforge_api_key: _,
            java_8_jvm_path: _,
            java_16_jvm_path: _,
            java_17_jvm_path: _,
            java_21_jvm_path: _,
            java_25_jvm_path: _,
            gamepad_calibrations: _,
        } = self;

        visit(FloatSettingId::UiFontSize.spec(), ui_font_size);
        visit(
            FloatSettingId::SkinPreviewMotionBlurAmount.spec(),
            skin_preview_motion_blur_amount,
        );
        visit(
            FloatSettingId::SkinPreviewMotionBlurShutterFrames.spec(),
            skin_preview_motion_blur_shutter_frames,
        );
    }

    /// Visits each integer setting with mutable access to its backing value.
    pub fn for_each_int_mut(&mut self, mut visit: impl FnMut(IntSettingSpec, &mut i32)) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred: _,
            graphics_adapter_preference_type: _,
            graphics_adapter_profile: _,
            graphics_adapter_explicit_hash: _,
            graphics_api_preference: _,
            streamer_mode_enabled: _,
            window_blur_enabled: _,
            windows_backdrop_type: _,
            ui_opacity_percent: _,
            legacy_windows_transparency_level: _,
            linux_set_opengl_driver: _,
            linux_use_zink_driver: _,
            theme_id: _,
            open_type_features_enabled: _,
            open_type_features_to_enable: _,
            notification_expiry_bars_empty_left: _,
            ui_font_family: _,
            ui_emoji_font_family: _,
            text_rendering_path: _,
            skin_preview_aa_mode: _,
            skin_preview_texel_aa_mode: _,
            svg_aa_mode: _,
            skin_preview_motion_blur_enabled: _,
            skin_preview_motion_blur_amount: _,
            skin_preview_motion_blur_shutter_frames: _,
            skin_preview_msaa_samples,
            skin_preview_fresh_format_enabled: _,
            skin_preview_3d_layers_enabled: _,
            hdr_when_available: _,
            ui_font_size: _,
            ui_font_weight,
            frame_limiter_enabled: _,
            discord_rich_presence_enabled: _,
            frame_limit_fps,
            skin_preview_motion_blur_sample_count,
            include_snapshots_and_betas: _,
            include_alpha_versions: _,
            include_experimental_versions: _,
            force_java_21_minimum: _,
            default_instance_max_memory_mib: _,
            default_instance_cli_args: _,
            minecraft_installations_root: _,
            download_max_concurrent: _,
            download_speed_limit_enabled: _,
            download_speed_limit: _,
            curseforge_api_key: _,
            java_8_jvm_path: _,
            java_16_jvm_path: _,
            java_17_jvm_path: _,
            java_21_jvm_path: _,
            java_25_jvm_path: _,
            gamepad_calibrations: _,
        } = self;

        visit(IntSettingId::UiFontWeight.spec(), ui_font_weight);
        visit(IntSettingId::FrameLimitFps.spec(), frame_limit_fps);
        visit(
            IntSettingId::SkinPreviewMsaaSamples.spec(),
            skin_preview_msaa_samples,
        );
        visit(
            IntSettingId::SkinPreviewMotionBlurSampleCount.spec(),
            skin_preview_motion_blur_sample_count,
        );
    }

    /// Visits each text setting with mutable access to its backing value.
    pub fn for_each_text_mut(&mut self, mut visit: impl FnMut(TextSettingSpec, &mut String)) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred: _,
            graphics_adapter_preference_type: _,
            graphics_adapter_profile: _,
            graphics_adapter_explicit_hash: _,
            graphics_api_preference: _,
            streamer_mode_enabled: _,
            window_blur_enabled: _,
            windows_backdrop_type: _,
            ui_opacity_percent: _,
            legacy_windows_transparency_level: _,
            linux_set_opengl_driver: _,
            linux_use_zink_driver: _,
            theme_id: _,
            open_type_features_enabled: _,
            open_type_features_to_enable,
            notification_expiry_bars_empty_left: _,
            ui_font_family: _,
            ui_emoji_font_family: _,
            text_rendering_path: _,
            skin_preview_aa_mode: _,
            skin_preview_texel_aa_mode: _,
            svg_aa_mode: _,
            skin_preview_msaa_samples: _,
            skin_preview_motion_blur_enabled: _,
            skin_preview_motion_blur_amount: _,
            skin_preview_motion_blur_shutter_frames: _,
            skin_preview_motion_blur_sample_count: _,
            skin_preview_fresh_format_enabled: _,
            skin_preview_3d_layers_enabled: _,
            hdr_when_available: _,
            ui_font_size: _,
            ui_font_weight: _,
            frame_limiter_enabled: _,
            discord_rich_presence_enabled: _,
            frame_limit_fps: _,
            include_snapshots_and_betas: _,
            include_alpha_versions: _,
            include_experimental_versions: _,
            force_java_21_minimum: _,
            default_instance_max_memory_mib: _,
            default_instance_cli_args: _,
            minecraft_installations_root: _,
            download_max_concurrent: _,
            download_speed_limit_enabled: _,
            download_speed_limit: _,
            curseforge_api_key: _,
            java_8_jvm_path: _,
            java_16_jvm_path: _,
            java_17_jvm_path: _,
            java_21_jvm_path: _,
            java_25_jvm_path: _,
            gamepad_calibrations: _,
        } = self;

        visit(
            TextSettingId::OpenTypeFeaturesToEnable.spec(),
            open_type_features_to_enable,
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
            window_blur_enabled: !cfg!(target_os = "macos"),
            windows_backdrop_type: default_windows_backdrop_type(),
            ui_opacity_percent: default_ui_opacity_percent(),
            legacy_windows_transparency_level: None,
            linux_set_opengl_driver: false,
            linux_use_zink_driver: false,
            theme_id: "matrix_oled".to_owned(),
            open_type_features_enabled: true,
            open_type_features_to_enable: String::new(),
            notification_expiry_bars_empty_left: false,
            ui_font_family: UiFontFamily::included_default(),
            ui_emoji_font_family: UiEmojiFontFamily::included_default(),
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
            frame_limiter_enabled: false,
            discord_rich_presence_enabled: true,
            frame_limit_fps: 120,
            ui_font_size: 18.0,
            ui_font_weight: 400,
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

/// Parses human input like `10mbps` into bits-per-second.
///
/// Supported suffixes: `kbps`, `mbps`, `gbps`, `tbps` (case-insensitive).
pub fn parse_bitrate_to_bps(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.to_ascii_lowercase();
    let (number_part, unit_multiplier): (&str, u64) =
        if let Some(prefix) = normalized.strip_suffix("tbps") {
            (prefix, 1_000_000_000_000)
        } else if let Some(prefix) = normalized.strip_suffix("gbps") {
            (prefix, 1_000_000_000)
        } else if let Some(prefix) = normalized.strip_suffix("mbps") {
            (prefix, 1_000_000)
        } else if let Some(prefix) = normalized.strip_suffix("kbps") {
            (prefix, 1_000)
        } else {
            return None;
        };

    let quantity = number_part.trim().parse::<f64>().ok()?;
    if !quantity.is_finite() || quantity <= 0.0 {
        return None;
    }

    let bps = quantity * unit_multiplier as f64;
    if !bps.is_finite() || bps <= 0.0 || bps > u64::MAX as f64 {
        return None;
    }
    Some(bps.round() as u64)
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
