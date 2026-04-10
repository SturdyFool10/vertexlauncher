use crate::{
    FRAME_LIMIT_FPS_MAX, FRAME_LIMIT_FPS_MIN, SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MAX,
    SKIN_PREVIEW_MOTION_BLUR_AMOUNT_MIN, SKIN_PREVIEW_MOTION_BLUR_AMOUNT_STEP,
    SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MAX, SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_MIN,
    SKIN_PREVIEW_MOTION_BLUR_SAMPLE_COUNT_STEP, SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MAX,
    SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_MIN, SKIN_PREVIEW_MOTION_BLUR_SHUTTER_FRAMES_STEP,
    SKIN_PREVIEW_MSAA_SAMPLES_MAX, SKIN_PREVIEW_MSAA_SAMPLES_MIN, SKIN_PREVIEW_MSAA_SAMPLES_STEP,
    UI_FONT_SIZE_MAX, UI_FONT_SIZE_MIN, UI_FONT_SIZE_STEP, UI_FONT_WEIGHT_MAX, UI_FONT_WEIGHT_MIN,
    UI_FONT_WEIGHT_STEP,
};

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
                info_tooltip: Some("Select the primary font used by the launcher UI."),
            },
            DropdownSettingId::GraphicsAdapterPreferenceType => DropdownSettingSpec {
                id: DropdownSettingId::GraphicsAdapterPreferenceType,
                label: "Graphics Adapter Preference Type",
                info_tooltip: Some(
                    "Choose whether the launcher picks a GPU by performance profile or by a specific detected adapter.",
                ),
            },
            DropdownSettingId::GraphicsAdapterPreference => DropdownSettingSpec {
                id: DropdownSettingId::GraphicsAdapterPreference,
                label: "Graphics Adapter Preference",
                info_tooltip: Some(
                    "When Explicit Adapter is selected and the saved GPU is unavailable, Vertex falls back to the High Performance profile.",
                ),
            },
            DropdownSettingId::GraphicsApiPreference => DropdownSettingSpec {
                id: DropdownSettingId::GraphicsApiPreference,
                label: "Graphics API Preference",
                info_tooltip: Some(
                    "Selects which graphics API backend the launcher uses. Adapter selection is applied separately.",
                ),
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
