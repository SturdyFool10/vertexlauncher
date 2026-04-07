use app_paths as launcher_paths;
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

const INCLUDED_DEFAULT_UI_FONT_FAMILY: &str = "Maple Mono NF";
const MAPLE_FONT_FAMILIES: &[&str] = &[
    INCLUDED_DEFAULT_UI_FONT_FAMILY,
    "Maple Mono",
    "Maple Mono Normal",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GamepadCalibration {
    pub center_x: f32,
    pub center_y: f32,
    pub deadzone_x: f32,
    pub deadzone_y: f32,
    pub threshold_x: f32,
    pub threshold_y: f32,
    pub x_forward_sign: i8,
    pub y_backward_sign: i8,
}

impl Default for GamepadCalibration {
    fn default() -> Self {
        Self {
            center_x: 0.0,
            center_y: 0.0,
            deadzone_x: 0.25,
            deadzone_y: 0.25,
            threshold_x: 0.5,
            threshold_y: 0.5,
            x_forward_sign: 1,
            y_backward_sign: 1,
        }
    }
}

impl GamepadCalibration {
    pub fn normalize(&mut self) {
        self.center_x = self.center_x.clamp(-1.0, 1.0);
        self.center_y = self.center_y.clamp(-1.0, 1.0);
        self.deadzone_x = self.deadzone_x.clamp(0.05, 0.95);
        self.deadzone_y = self.deadzone_y.clamp(0.05, 0.95);
        self.threshold_x = self
            .threshold_x
            .clamp((self.deadzone_x + 0.05).min(0.95), 0.98);
        self.threshold_y = self
            .threshold_y
            .clamp((self.deadzone_y + 0.05).min(0.95), 0.98);
        self.x_forward_sign = if self.x_forward_sign >= 0 { 1 } else { -1 };
        self.y_backward_sign = if self.y_backward_sign >= 0 { 1 } else { -1 };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkinPreviewAaMode {
    Off,
    Msaa,
    Smaa,
    Fxaa,
    Taa,
    FxaaTaa,
}

impl SkinPreviewAaMode {
    pub const ALL: [SkinPreviewAaMode; 6] = [
        SkinPreviewAaMode::Msaa,
        SkinPreviewAaMode::Smaa,
        SkinPreviewAaMode::Fxaa,
        SkinPreviewAaMode::Taa,
        SkinPreviewAaMode::FxaaTaa,
        SkinPreviewAaMode::Off,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            SkinPreviewAaMode::Off => "Off",
            SkinPreviewAaMode::Msaa => "MSAA (GPU)",
            SkinPreviewAaMode::Smaa => "SMAA (GPU Post)",
            SkinPreviewAaMode::Fxaa => "FXAA (Post)",
            SkinPreviewAaMode::Taa => "TAA (Temporal)",
            SkinPreviewAaMode::FxaaTaa => "FXAA + TAA (Temporal)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkinPreviewTexelAaMode {
    Off,
    TexelBoundary,
}

impl SkinPreviewTexelAaMode {
    pub const ALL: [SkinPreviewTexelAaMode; 2] = [
        SkinPreviewTexelAaMode::Off,
        SkinPreviewTexelAaMode::TexelBoundary,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            SkinPreviewTexelAaMode::Off => "Off",
            SkinPreviewTexelAaMode::TexelBoundary => "Texel Border AA",
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SvgAaMode {
    Off,
    Balanced,
    Crisp,
    Ultra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextRenderingPath {
    Auto,
    AlphaMask,
    Sdf,
    Msdf,
}

impl TextRenderingPath {
    pub const ALL: [TextRenderingPath; 4] = [
        TextRenderingPath::Auto,
        TextRenderingPath::AlphaMask,
        TextRenderingPath::Sdf,
        TextRenderingPath::Msdf,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            TextRenderingPath::Auto => "Auto",
            TextRenderingPath::AlphaMask => "Alpha Mask",
            TextRenderingPath::Sdf => "SDF",
            TextRenderingPath::Msdf => "MSDF",
        }
    }
}

impl SvgAaMode {
    pub const ALL: [SvgAaMode; 4] = [
        SvgAaMode::Balanced,
        SvgAaMode::Crisp,
        SvgAaMode::Ultra,
        SvgAaMode::Off,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            SvgAaMode::Off => "Off",
            SvgAaMode::Balanced => "Balanced (SSAA 2x)",
            SvgAaMode::Crisp => "Crisp (SSAA 3x)",
            SvgAaMode::Ultra => "Ultra (SSAA 4x)",
        }
    }

    pub const fn supersample_scale(self) -> u32 {
        match self {
            SvgAaMode::Off => 1,
            SvgAaMode::Balanced => 2,
            SvgAaMode::Crisp => 3,
            SvgAaMode::Ultra => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsTransparencyLevel {
    Solid,
    Low,
    Medium,
    High,
}

impl WindowsTransparencyLevel {
    pub const fn ui_opacity_percent(self) -> u8 {
        match self {
            WindowsTransparencyLevel::Solid => 100,
            WindowsTransparencyLevel::Low => 70,
            WindowsTransparencyLevel::Medium => 50,
            WindowsTransparencyLevel::High => 30,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsBackdropType {
    Auto,
    Mica,
    Acrylic,
    MicaAlt,
    LegacyBlur,
}

impl WindowsBackdropType {
    pub const ALL: [WindowsBackdropType; 5] = [
        WindowsBackdropType::Auto,
        WindowsBackdropType::Mica,
        WindowsBackdropType::Acrylic,
        WindowsBackdropType::MicaAlt,
        WindowsBackdropType::LegacyBlur,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            WindowsBackdropType::Auto => "Auto",
            WindowsBackdropType::Mica => "Mica",
            WindowsBackdropType::Acrylic => "Acrylic",
            WindowsBackdropType::MicaAlt => "Mica Alt",
            WindowsBackdropType::LegacyBlur => "Legacy Blur",
        }
    }
}

const fn default_windows_backdrop_type() -> WindowsBackdropType {
    WindowsBackdropType::Auto
}

const fn default_ui_opacity_percent() -> u8 {
    100
}

/// File format choice for config creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigFormat {
    Json,
    Toml,
}

impl ConfigFormat {
    /// Human-readable label for config format selection UI.
    pub fn label(self) -> &'static str {
        match self {
            ConfigFormat::Json => "JSON (.json)",
            ConfigFormat::Toml => "TOML (.toml)",
        }
    }

    /// Filename extension associated with this config format.
    pub fn extension(self) -> &'static str {
        match self {
            ConfigFormat::Json => "json",
            ConfigFormat::Toml => "toml",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UiFontFamily(String);

impl UiFontFamily {
    /// Creates the bundled Maple Mono family selection.
    pub fn included_default() -> Self {
        Self(INCLUDED_DEFAULT_UI_FONT_FAMILY.to_owned())
    }

    /// Creates a font family from a discovered or configured family name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(normalize_ui_font_family_name(name.into()))
    }

    /// Returns whether this font family is shipped with the launcher.
    pub fn is_included_default(&self) -> bool {
        self.matches_name(INCLUDED_DEFAULT_UI_FONT_FAMILY)
    }

    /// Short display label.
    pub fn label(&self) -> &str {
        &self.0
    }

    /// Settings-facing label including default marker when applicable.
    pub fn settings_label(&self) -> String {
        if self.is_included_default() {
            format!("{} (Included default)", self.label())
        } else {
            self.label().to_owned()
        }
    }

    /// Font family candidates used when applying the selected face.
    pub fn query_families(&self) -> Vec<&str> {
        if self.is_included_default() {
            MAPLE_FONT_FAMILIES.to_vec()
        } else {
            vec![self.label()]
        }
    }

    /// Case-insensitive match used when reconciling discovered font families.
    pub fn matches(&self, other: &Self) -> bool {
        normalized_ui_font_family_key(self.label()) == normalized_ui_font_family_key(other.label())
    }

    /// Case-insensitive match against a raw family name.
    pub fn matches_name(&self, other: &str) -> bool {
        normalized_ui_font_family_key(self.label()) == normalized_ui_font_family_key(other)
    }

    /// Normalizes the stored family name in place.
    pub fn normalize(&mut self) {
        self.0 = normalize_ui_font_family_name(std::mem::take(&mut self.0));
    }
}

impl Serialize for UiFontFamily {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.label())
    }
}

impl<'de> Deserialize<'de> for UiFontFamily {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::new(raw))
    }
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

const INCLUDED_DEFAULT_EMOJI_FONT_FAMILY: &str = "Noto Color Emoji";
const INCLUDED_EMOJI_FONT_SETTINGS_LABEL: &str = "Noto Color Emoji (Included)";

/// Identifies which emoji font to use for glyph fallback when text contains emoji.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UiEmojiFontFamily(String);

impl UiEmojiFontFamily {
    /// Creates the bundled Noto Color Emoji selection.
    pub fn included_default() -> Self {
        Self(INCLUDED_EMOJI_FONT_SETTINGS_LABEL.to_owned())
    }

    /// Creates an emoji font family from a discovered or configured family name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(normalize_emoji_font_family_name(name.into()))
    }

    /// Returns whether this font family is the bundled Noto Color Emoji.
    pub fn is_included_default(&self) -> bool {
        normalize_emoji_font_family_key(&self.0) == normalize_emoji_font_family_key(INCLUDED_EMOJI_FONT_SETTINGS_LABEL)
            || normalize_emoji_font_family_key(&self.0) == normalize_emoji_font_family_key(INCLUDED_DEFAULT_EMOJI_FONT_FAMILY)
    }

    /// Display label used in the settings UI.
    pub fn label(&self) -> &str {
        &self.0
    }

    /// Settings-facing label with the included marker when applicable.
    pub fn settings_label(&self) -> String {
        if self.is_included_default() {
            INCLUDED_EMOJI_FONT_SETTINGS_LABEL.to_owned()
        } else {
            self.0.clone()
        }
    }

    /// The font family name to query from the font catalog.
    pub fn family_name(&self) -> &str {
        if self.is_included_default() {
            INCLUDED_DEFAULT_EMOJI_FONT_FAMILY
        } else {
            &self.0
        }
    }

    /// Case-insensitive match used when reconciling discovered font families.
    pub fn matches(&self, other: &Self) -> bool {
        normalize_emoji_font_family_key(self.family_name())
            == normalize_emoji_font_family_key(other.family_name())
    }

    /// Normalizes the stored family name in place.
    pub fn normalize(&mut self) {
        self.0 = normalize_emoji_font_family_name(std::mem::take(&mut self.0));
    }
}

impl Serialize for UiEmojiFontFamily {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.settings_label())
    }
}

impl<'de> Deserialize<'de> for UiEmojiFontFamily {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::new(raw))
    }
}

fn normalize_emoji_font_family_name(name: String) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || normalize_emoji_font_family_key(trimmed)
            == normalize_emoji_font_family_key(INCLUDED_DEFAULT_EMOJI_FONT_FAMILY)
        || normalize_emoji_font_family_key(trimmed)
            == normalize_emoji_font_family_key(INCLUDED_EMOJI_FONT_SETTINGS_LABEL)
    {
        return INCLUDED_EMOJI_FONT_SETTINGS_LABEL.to_owned();
    }
    trimmed.to_owned()
}

fn normalize_emoji_font_family_key(name: &str) -> String {
    name.trim().to_lowercase()
}

fn normalize_ui_font_family_name(name: String) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return INCLUDED_DEFAULT_UI_FONT_FAMILY.to_owned();
    }

    match trimmed {
        "maple_mono_nf" => INCLUDED_DEFAULT_UI_FONT_FAMILY.to_owned(),
        "jetbrains_mono" => "JetBrains Mono".to_owned(),
        "fira_code" => "Fira Code".to_owned(),
        "cascadia_code" => "Cascadia Code".to_owned(),
        "iosevka" => "Iosevka".to_owned(),
        _ if trimmed.eq_ignore_ascii_case(INCLUDED_DEFAULT_UI_FONT_FAMILY) => {
            INCLUDED_DEFAULT_UI_FONT_FAMILY.to_owned()
        }
        _ => trimmed.to_owned(),
    }
}

fn normalized_ui_font_family_key(name: &str) -> String {
    name.trim().to_lowercase()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JavaRuntimeVersion {
    Java8,
    Java16,
    Java17,
    Java21,
    Java25,
}

impl JavaRuntimeVersion {
    pub const ALL: [JavaRuntimeVersion; 5] = [
        JavaRuntimeVersion::Java8,
        JavaRuntimeVersion::Java16,
        JavaRuntimeVersion::Java17,
        JavaRuntimeVersion::Java21,
        JavaRuntimeVersion::Java25,
    ];

    /// Java major version number.
    pub const fn major(self) -> u8 {
        match self {
            JavaRuntimeVersion::Java8 => 8,
            JavaRuntimeVersion::Java16 => 16,
            JavaRuntimeVersion::Java17 => 17,
            JavaRuntimeVersion::Java21 => 21,
            JavaRuntimeVersion::Java25 => 25,
        }
    }

    /// Settings label for Java runtime path input.
    pub const fn label(self) -> &'static str {
        match self {
            JavaRuntimeVersion::Java8 => "Java 8 JVM Path",
            JavaRuntimeVersion::Java16 => "Java 16 JVM Path",
            JavaRuntimeVersion::Java17 => "Java 17 JVM Path",
            JavaRuntimeVersion::Java21 => "Java 21 JVM Path",
            JavaRuntimeVersion::Java25 => "Java 25 JVM Path",
        }
    }

    /// Tooltip explaining Minecraft version compatibility for this runtime.
    pub const fn info_tooltip(self) -> &'static str {
        match self {
            JavaRuntimeVersion::Java8 => "Used for Minecraft 1.16.5 and older release versions.",
            JavaRuntimeVersion::Java16 => "Used for Minecraft 1.17.x release versions.",
            JavaRuntimeVersion::Java17 => {
                "Used for Minecraft 1.18 through 1.20.4 release versions."
            }
            JavaRuntimeVersion::Java21 => "Used for Minecraft 1.20.5 through 1.x release versions.",
            JavaRuntimeVersion::Java25 => "Used for Minecraft 26.x and newer release versions.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToggleSettingId {
    LowPowerGpuPreferred,
    StreamerModeEnabled,
    WindowBlurEnabled,
    OpenTypeFeaturesEnabled,
    NotificationExpiryBarsEmptyLeft,
    SkinPreviewFreshFormatEnabled,
    SkinPreview3dLayersEnabled,
    SnapshotsAndBetasEnabled,
    ForceJava21Minimum,
    FrameLimiterEnabled,
    DiscordRichPresenceEnabled,
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
                    "Uses integrated graphics when both integrated and discrete GPUs are available. Requires restart.",
                ),
            },
            ToggleSettingId::StreamerModeEnabled => ToggleSettingSpec {
                id: ToggleSettingId::StreamerModeEnabled,
                label: "Enable Streamer Mode",
                info_tooltip: Some(
                    "Hides account names, avatars, and account-identifying details across the launcher UI.",
                ),
            },
            ToggleSettingId::WindowBlurEnabled => ToggleSettingSpec {
                id: ToggleSettingId::WindowBlurEnabled,
                label: "Enable Window Blur",
                info_tooltip: Some(
                    "Enables acrylic (Windows) and KDE blur (Linux). Temporarily disabled on macOS while the launch-safe fallback is in place. Requires restart.",
                ),
            },
            ToggleSettingId::OpenTypeFeaturesEnabled => ToggleSettingSpec {
                id: ToggleSettingId::OpenTypeFeaturesEnabled,
                label: "Enable OpenType Features",
                info_tooltip: Some(
                    "When enabled and the list below is empty, defaults to liga, calt.",
                ),
            },
            ToggleSettingId::NotificationExpiryBarsEmptyLeft => ToggleSettingSpec {
                id: ToggleSettingId::NotificationExpiryBarsEmptyLeft,
                label: "Empty Expiry Bars to the Left",
                info_tooltip: Some(
                    "Notification expiry bars drain from left to right instead of right to left.",
                ),
            },
            ToggleSettingId::SkinPreviewFreshFormatEnabled => ToggleSettingSpec {
                id: ToggleSettingId::SkinPreviewFreshFormatEnabled,
                label: "Enable Skin Expressions",
                info_tooltip: Some(
                    "Animates supported eye and eyebrow layouts in the skin preview using a built-in Rust implementation inspired by Fresh-style expression packs.",
                ),
            },
            ToggleSettingId::SkinPreview3dLayersEnabled => ToggleSettingSpec {
                id: ToggleSettingId::SkinPreview3dLayersEnabled,
                label: "Enable 3D Skin Layers",
                info_tooltip: Some(
                    "Turns skin second-layer pixels into voxelized 3D detail in the skin preview. Compatible with the Fresh skin format toggle.",
                ),
            },
            ToggleSettingId::SnapshotsAndBetasEnabled => ToggleSettingSpec {
                id: ToggleSettingId::SnapshotsAndBetasEnabled,
                label: "Include Snapshots and Betas",
                info_tooltip: Some(
                    "Allows selecting snapshot and beta/alpha Minecraft versions in instance version dropdowns.",
                ),
            },
            ToggleSettingId::ForceJava21Minimum => ToggleSettingSpec {
                id: ToggleSettingId::ForceJava21Minimum,
                label: "Force Java 21 Minimum",
                info_tooltip: Some(
                    "When enabled, versions requiring Java 8/16/17 use Java 21 instead. Higher Java requirements are unchanged.",
                ),
            },
            ToggleSettingId::FrameLimiterEnabled => ToggleSettingSpec {
                id: ToggleSettingId::FrameLimiterEnabled,
                label: "Enable Frame Limiter",
                info_tooltip: Some(
                    "Caps launcher rendering FPS to reduce power usage and heat. Applied immediately.",
                ),
            },
            ToggleSettingId::DiscordRichPresenceEnabled => ToggleSettingSpec {
                id: ToggleSettingId::DiscordRichPresenceEnabled,
                label: "Enable Discord Rich Presence",
                info_tooltip: Some(
                    "Shows the instance currently being played and an elapsed session timer in Discord while the launcher owns presence for that session.",
                ),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropdownSettingId {
    UiFontFamily,
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
                info_tooltip: Some("Select the primary font used by the launcher UI."),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloatSettingId {
    UiFontSize,
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
                info_tooltip: Some("Floating-point point size used for body/button text."),
                min: UI_FONT_SIZE_MIN,
                max: UI_FONT_SIZE_MAX,
                step: UI_FONT_SIZE_STEP,
            },
            FloatSettingId::SkinPreviewMotionBlurAmount => FloatSettingSpec {
                id: FloatSettingId::SkinPreviewMotionBlurAmount,
                label: "Skin Preview Motion Blur Amount",
                info_tooltip: Some(
                    "Controls how strongly off-center shutter samples contribute to the final image.",
                ),
                min: SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MIN,
                max: SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MAX,
                step: SKIN_PREVIEW_MOTION_BLUR_AMOUNT_STEP,
            },
            FloatSettingId::SkinPreviewMotionBlurShutterFrames => FloatSettingSpec {
                id: FloatSettingId::SkinPreviewMotionBlurShutterFrames,
                label: "Skin Preview Motion Blur Shutter",
                info_tooltip: Some(
                    "Total shutter interval in 60 FPS frame lengths used for motion blur accumulation.",
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
                info_tooltip: Some("Integer CSS-like font weight (100-900)."),
                min: UI_FONT_WEIGHT_MIN,
                max: UI_FONT_WEIGHT_MAX,
                step: UI_FONT_WEIGHT_STEP,
            },
            IntSettingId::FrameLimitFps => IntSettingSpec {
                id: IntSettingId::FrameLimitFps,
                label: "Frame Limit FPS",
                info_tooltip: Some("Maximum UI frame rate when frame limiter is enabled."),
                min: FRAME_LIMIT_FPS_MIN,
                max: FRAME_LIMIT_FPS_MAX,
                step: 1,
            },
            IntSettingId::SkinPreviewMsaaSamples => IntSettingSpec {
                id: IntSettingId::SkinPreviewMsaaSamples,
                label: "Skin Preview MSAA Samples",
                info_tooltip: Some(
                    "GPU MSAA sample count used when Skin Preview Anti-Aliasing is set to MSAA.",
                ),
                min: SKIN_PREVIEW_MSAA_SAMPLES_MIN,
                max: SKIN_PREVIEW_MSAA_SAMPLES_MAX,
                step: SKIN_PREVIEW_MSAA_SAMPLES_STEP,
            },
            IntSettingId::SkinPreviewMotionBlurSampleCount => IntSettingSpec {
                id: IntSettingId::SkinPreviewMotionBlurSampleCount,
                label: "Skin Preview Motion Blur Samples",
                info_tooltip: Some(
                    "How many temporal samples are accumulated across the shutter interval.",
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
                    "Comma-separated feature tags. Example: liga, calt. Leave empty to use the default list: liga, calt.",
                ),
            },
        }
    }
}

/// Launcher configuration persisted as JSON/TOML.
///
/// Values are normalized on load/save via [`Config::normalize`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    low_power_gpu_preferred: bool,
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
    frame_limiter_enabled: bool,
    discord_rich_presence_enabled: bool,
    frame_limit_fps: i32,
    ui_font_size: f32,
    ui_font_weight: i32,
    include_snapshots_and_betas: bool,
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
    }

    /// Visits each toggle setting with mutable access to its backing value.
    pub fn for_each_toggle_mut(&mut self, mut visit: impl FnMut(ToggleSettingSpec, &mut bool)) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred,
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
            ui_font_size: _,
            ui_font_weight: _,
            include_snapshots_and_betas,
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
            ui_font_size: _,
            ui_font_weight: _,
            include_snapshots_and_betas: _,
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
            ui_font_size,
            ui_font_weight: _,
            include_snapshots_and_betas: _,
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
            ui_font_size: _,
            ui_font_weight,
            frame_limiter_enabled: _,
            discord_rich_presence_enabled: _,
            frame_limit_fps,
            skin_preview_motion_blur_sample_count,
            include_snapshots_and_betas: _,
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
            ui_font_size: _,
            ui_font_weight: _,
            frame_limiter_enabled: _,
            discord_rich_presence_enabled: _,
            frame_limit_fps: _,
            include_snapshots_and_betas: _,
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
            frame_limiter_enabled: false,
            discord_rich_presence_enabled: true,
            frame_limit_fps: 120,
            ui_font_size: 18.0,
            ui_font_weight: 400,
            include_snapshots_and_betas: false,
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
