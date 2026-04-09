use serde::{Deserialize, Serialize};

/// File format choice for config creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigFormat {
    /// JSON format (.json extension).
    Json,
    /// TOML format (.toml extension).
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

    /// File extension for this config format (without leading dot).
    pub fn extension(self) -> &'static str {
        match self {
            ConfigFormat::Json => "json",
            ConfigFormat::Toml => "toml",
        }
    }
}
