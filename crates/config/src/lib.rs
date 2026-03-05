use serde::{Deserialize, Serialize};
use std::io::{Error as IOError, Write};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToggleSettingId {
    LowPowerGpuPreferred,
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
                    "Uses integrated graphics when both integrated and discrete GPUs are available.(Requires Restart to take effect)",
                ),
            },
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    low_power_gpu_preferred: bool,
}

impl Config {
    pub fn low_power_gpu_preferred(&self) -> bool {
        self.low_power_gpu_preferred
    }

    pub fn for_each_toggle_mut(&mut self, mut visit: impl FnMut(ToggleSettingSpec, &mut bool)) {
        // Intentionally destructure all fields to force updates here when Config changes.
        let Self {
            low_power_gpu_preferred,
        } = self;

        visit(
            ToggleSettingId::LowPowerGpuPreferred.spec(),
            low_power_gpu_preferred,
        );
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            low_power_gpu_preferred: true, //this will not effect single GPU systems, but on systems with multiple valid GPUs it can make our app run on the less utilized integrated GPU
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
                .map_err(|e| IOError::new(std::io::ErrorKind::Other, e))?;
        }
        ConfigFormat::Toml => {
            let value = toml::to_string_pretty(conf)
                .map_err(|e| IOError::new(std::io::ErrorKind::Other, e))?;
            let mut file = std::fs::File::create(path)?;
            file.write_all(value.as_bytes())?;
        }
    };

    Ok(())
}

pub fn create_default_config(format: ConfigFormat) -> Result<Config, IOError> {
    let config_path = format!("{}.{}", config_base_path(), format.extension());
    let config = Config::default();
    construct_new_config(&config_path, &config)?;
    Ok(config)
}

pub fn load_config() -> LoadConfigResult {
    let base = config_base_path();

    match find_existing_config_path(&base) {
        Some(path) => {
            let contents = std::fs::read_to_string(&path).unwrap_or_default();
            let parsed = if path.ends_with(".json") {
                serde_json::from_str(&contents).unwrap_or_default()
            } else {
                toml::from_str(&contents).unwrap_or_default()
            };

            LoadConfigResult::Loaded(parsed)
        }
        None => LoadConfigResult::Missing {
            default_format: ConfigFormat::Json,
        },
    }
}
