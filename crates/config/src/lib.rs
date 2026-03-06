use serde::{Deserialize, Serialize};
use std::io::{Error as IOError, Write};

pub const UI_FONT_SIZE_MIN: f32 = 10.0;
pub const UI_FONT_SIZE_MAX: f32 = 42.0;
pub const UI_FONT_SIZE_STEP: f32 = 0.5;
pub const UI_FONT_WEIGHT_MIN: i32 = 100;
pub const UI_FONT_WEIGHT_MAX: i32 = 900;
pub const UI_FONT_WEIGHT_STEP: i32 = 100;

const MAPLE_FONT_FAMILIES: &[&str] = &["Maple Mono NF", "Maple Mono", "Maple Mono Normal"];
const JETBRAINS_FONT_FAMILIES: &[&str] = &[
    "JetBrains Mono",
    "JetBrainsMono",
    "JetBrainsMono Nerd Font",
    "JetBrainsMono Nerd Font Mono",
    "JetBrainsMono NF",
    "JetBrainsMono NFM",
];
const FIRA_FONT_FAMILIES: &[&str] = &[
    "Fira Code",
    "FiraCode",
    "FiraCode Nerd Font",
    "FiraCode Nerd Font Mono",
    "FiraCode NF",
    "FiraCode NFM",
];
const CASCADIA_FONT_FAMILIES: &[&str] = &[
    "Cascadia Code",
    "Cascadia Mono",
    "CaskaydiaCove Nerd Font",
    "CaskaydiaCove Nerd Font Mono",
    "CaskaydiaMono Nerd Font",
    "CaskaydiaMono Nerd Font Mono",
];
const IOSEVKA_FONT_FAMILIES: &[&str] = &[
    "Iosevka",
    "Iosevka Term",
    "Iosevka Nerd Font",
    "Iosevka Nerd Font Mono",
    "Iosevka NFM",
    "IosevkaTerm Nerd Font",
    "IosevkaTerm Nerd Font Mono",
    "IosevkaTerm NFM",
];

const UI_FONT_OPTIONS: &[UiFontFamily] = &[
    UiFontFamily::MapleMonoNf,
    UiFontFamily::JetBrainsMono,
    UiFontFamily::FiraCode,
    UiFontFamily::CascadiaCode,
    UiFontFamily::Iosevka,
];

const UI_FONT_SYSTEM_OPTIONS: &[UiFontFamily] = &[
    UiFontFamily::JetBrainsMono,
    UiFontFamily::FiraCode,
    UiFontFamily::CascadiaCode,
    UiFontFamily::Iosevka,
];

const UI_FONT_OPTION_LABELS: &[&str] = &[
    "Maple Mono NF (Included default)",
    "JetBrains Mono",
    "Fira Code",
    "Cascadia Code",
    "Iosevka",
];

/// File format choice for config creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigFormat {
    Json,
    Toml,
}

impl ConfigFormat {
    pub fn label(self) -> &'static str {
        match self {
            ConfigFormat::Json => "JSON (.json)",
            ConfigFormat::Toml => "TOML (.toml)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ConfigFormat::Json => "json",
            ConfigFormat::Toml => "toml",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiFontFamily {
    MapleMonoNf,
    JetBrainsMono,
    FiraCode,
    CascadiaCode,
    Iosevka,
}

impl UiFontFamily {
    pub fn is_included_default(self) -> bool {
        matches!(self, UiFontFamily::MapleMonoNf)
    }

    pub fn label(self) -> &'static str {
        match self {
            UiFontFamily::MapleMonoNf => "Maple Mono NF",
            UiFontFamily::JetBrainsMono => "JetBrains Mono",
            UiFontFamily::FiraCode => "Fira Code",
            UiFontFamily::CascadiaCode => "Cascadia Code",
            UiFontFamily::Iosevka => "Iosevka",
        }
    }

    pub fn settings_label(self) -> &'static str {
        match self {
            UiFontFamily::MapleMonoNf => "Maple Mono NF (Included default)",
            _ => self.label(),
        }
    }

    pub fn query_families(self) -> &'static [&'static str] {
        match self {
            UiFontFamily::MapleMonoNf => MAPLE_FONT_FAMILIES,
            UiFontFamily::JetBrainsMono => JETBRAINS_FONT_FAMILIES,
            UiFontFamily::FiraCode => FIRA_FONT_FAMILIES,
            UiFontFamily::CascadiaCode => CASCADIA_FONT_FAMILIES,
            UiFontFamily::Iosevka => IOSEVKA_FONT_FAMILIES,
        }
    }

    pub fn system_options() -> &'static [UiFontFamily] {
        UI_FONT_SYSTEM_OPTIONS
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToggleSettingId {
    LowPowerGpuPreferred,
    OpenTypeFeaturesEnabled,
}

#[derive(Clone, Copy, Debug)]
pub struct ToggleSettingSpec {
    pub id: ToggleSettingId,
    pub label: &'static str,
    pub info_tooltip: Option<&'static str>,
}

impl ToggleSettingId {
    pub const fn spec(self) -> ToggleSettingSpec {
        match self {
            ToggleSettingId::LowPowerGpuPreferred => ToggleSettingSpec {
                id: ToggleSettingId::LowPowerGpuPreferred,
                label: "Prefer Integrated Graphics",
                info_tooltip: Some(
                    "Uses integrated graphics when both integrated and discrete GPUs are available. Requires restart.",
                ),
            },
            ToggleSettingId::OpenTypeFeaturesEnabled => ToggleSettingSpec {
                id: ToggleSettingId::OpenTypeFeaturesEnabled,
                label: "Enable OpenType Features",
                info_tooltip: Some(
                    "When enabled and the list below is empty, defaults to liga, calt.",
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
    pub options: &'static [UiFontFamily],
    pub option_labels: &'static [&'static str],
}

impl DropdownSettingId {
    pub const fn spec(self) -> DropdownSettingSpec {
        match self {
            DropdownSettingId::UiFontFamily => DropdownSettingSpec {
                id: DropdownSettingId::UiFontFamily,
                label: "UI Font",
                info_tooltip: Some("Select the primary font used by the launcher UI."),
                options: UI_FONT_OPTIONS,
                option_labels: UI_FONT_OPTION_LABELS,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloatSettingId {
    UiFontSize,
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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntSettingId {
    UiFontWeight,
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    low_power_gpu_preferred: bool,
    open_type_features_enabled: bool,
    open_type_features_to_enable: String,
    ui_font_family: UiFontFamily,
    ui_font_size: f32,
    ui_font_weight: i32,
}

impl Config {
    pub fn low_power_gpu_preferred(&self) -> bool {
        self.low_power_gpu_preferred
    }

    pub fn ui_font_family(&self) -> UiFontFamily {
        self.ui_font_family
    }

    pub fn open_type_features_enabled(&self) -> bool {
        self.open_type_features_enabled
    }

    pub fn open_type_features_to_enable(&self) -> &str {
        &self.open_type_features_to_enable
    }

    pub fn ui_font_size(&self) -> f32 {
        self.ui_font_size
    }

    pub fn ui_font_weight(&self) -> i32 {
        self.ui_font_weight
    }

    pub fn normalize(&mut self) {
        self.ui_font_size = self.ui_font_size.clamp(UI_FONT_SIZE_MIN, UI_FONT_SIZE_MAX);
        self.ui_font_weight = self
            .ui_font_weight
            .clamp(UI_FONT_WEIGHT_MIN, UI_FONT_WEIGHT_MAX);
    }

    pub fn for_each_toggle_mut(&mut self, mut visit: impl FnMut(ToggleSettingSpec, &mut bool)) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred,
            open_type_features_enabled,
            open_type_features_to_enable: _,
            ui_font_family: _,
            ui_font_size: _,
            ui_font_weight: _,
        } = self;

        visit(
            ToggleSettingId::LowPowerGpuPreferred.spec(),
            low_power_gpu_preferred,
        );
        visit(
            ToggleSettingId::OpenTypeFeaturesEnabled.spec(),
            open_type_features_enabled,
        );
    }

    pub fn for_each_dropdown_mut(
        &mut self,
        mut visit: impl FnMut(DropdownSettingSpec, &mut UiFontFamily),
    ) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred: _,
            open_type_features_enabled: _,
            open_type_features_to_enable: _,
            ui_font_family,
            ui_font_size: _,
            ui_font_weight: _,
        } = self;

        visit(DropdownSettingId::UiFontFamily.spec(), ui_font_family);
    }

    pub fn for_each_float_mut(&mut self, mut visit: impl FnMut(FloatSettingSpec, &mut f32)) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred: _,
            open_type_features_enabled: _,
            open_type_features_to_enable: _,
            ui_font_family: _,
            ui_font_size,
            ui_font_weight: _,
        } = self;

        visit(FloatSettingId::UiFontSize.spec(), ui_font_size);
    }

    pub fn for_each_int_mut(&mut self, mut visit: impl FnMut(IntSettingSpec, &mut i32)) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred: _,
            open_type_features_enabled: _,
            open_type_features_to_enable: _,
            ui_font_family: _,
            ui_font_size: _,
            ui_font_weight,
        } = self;

        visit(IntSettingId::UiFontWeight.spec(), ui_font_weight);
    }

    pub fn for_each_text_mut(&mut self, mut visit: impl FnMut(TextSettingSpec, &mut String)) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred: _,
            open_type_features_enabled: _,
            open_type_features_to_enable,
            ui_font_family: _,
            ui_font_size: _,
            ui_font_weight: _,
        } = self;

        visit(
            TextSettingId::OpenTypeFeaturesToEnable.spec(),
            open_type_features_to_enable,
        );
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            low_power_gpu_preferred: true,
            open_type_features_enabled: true,
            open_type_features_to_enable: String::new(),
            ui_font_family: UiFontFamily::MapleMonoNf,
            ui_font_size: 18.0,
            ui_font_weight: 400,
        }
    }
}

#[derive(Clone, Debug)]
pub enum LoadConfigResult {
    Loaded(Config),
    Missing { default_format: ConfigFormat },
}

fn config_base_path() -> String {
    let config_path_no_ext = "config";
    match std::env::var("VERTEX_CONFIG_LOCATION") {
        Ok(dir) => format!("{dir}/{config_path_no_ext}"),
        Err(_) => config_path_no_ext.to_string(),
    }
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

fn construct_new_config(path: &str, conf: &Config) -> Result<(), IOError> {
    let format = if path.ends_with(".toml") {
        ConfigFormat::Toml
    } else {
        ConfigFormat::Json
    };

    match format {
        ConfigFormat::Json => {
            serde_json::to_writer(std::fs::File::create(path)?, conf)
                .map_err(|e| IOError::other(e.to_string()))?;
        }
        ConfigFormat::Toml => {
            let value = toml::to_string_pretty(conf).map_err(|e| IOError::other(e.to_string()))?;
            let mut file = std::fs::File::create(path)?;
            file.write_all(value.as_bytes())?;
        }
    };

    Ok(())
}

pub fn create_default_config(format: ConfigFormat) -> Result<Config, IOError> {
    let config_path = format!("{}.{}", config_base_path(), format.extension());
    let mut config = Config::default();
    config.normalize();
    construct_new_config(&config_path, &config)?;
    Ok(config)
}

pub fn save_config(config: &Config) -> Result<(), IOError> {
    let mut normalized = config.clone();
    normalized.normalize();

    let base = config_base_path();
    let path = find_existing_config_path(&base).unwrap_or_else(|| format!("{base}.json"));
    construct_new_config(&path, &normalized)
}

pub fn load_config() -> LoadConfigResult {
    let base = config_base_path();

    match find_existing_config_path(&base) {
        Some(path) => {
            let contents = std::fs::read_to_string(&path).unwrap_or_default();
            let mut parsed: Config = if path.ends_with(".json") {
                serde_json::from_str(&contents).unwrap_or_default()
            } else {
                toml::from_str(&contents).unwrap_or_default()
            };

            parsed.normalize();
            LoadConfigResult::Loaded(parsed)
        }
        None => LoadConfigResult::Missing {
            default_format: ConfigFormat::Json,
        },
    }
}
