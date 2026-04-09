use serde::{Deserialize, Serialize};

/// Minecraft skin variant (model type) for character appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MinecraftSkinVariant {
    /// Classic Steve/Alex model with standard proportions.
    Classic,
    /// Slim model variant with thinner arms.
    Slim,
}

impl MinecraftSkinVariant {
    /// Returns the API string representation for this skin variant.
    pub fn as_api_str(self) -> &'static str {
        match self {
            MinecraftSkinVariant::Classic => "classic",
            MinecraftSkinVariant::Slim => "slim",
        }
    }
}
