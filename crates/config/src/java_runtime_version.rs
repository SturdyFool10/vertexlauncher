use serde::{Deserialize, Serialize};

/// Java runtime version selection for Minecraft instance configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JavaRuntimeVersion {
    /// Java 8 runtime (legacy support).
    Java8,
    /// Java 16 runtime.
    Java16,
    /// Java 17 runtime (LTS).
    Java17,
    /// Java 21 runtime (LTS).
    Java21,
    /// Java 25 runtime (latest LTS).
    Java25,
}

impl JavaRuntimeVersion {
    /// Array of all available Java runtime versions in preference order.
    pub const ALL: [JavaRuntimeVersion; 5] = [
        JavaRuntimeVersion::Java8,
        JavaRuntimeVersion::Java16,
        JavaRuntimeVersion::Java17,
        JavaRuntimeVersion::Java21,
        JavaRuntimeVersion::Java25,
    ];

    /// Returns the major version number for this Java runtime.
    pub const fn major(self) -> u8 {
        match self {
            JavaRuntimeVersion::Java8 => 8,
            JavaRuntimeVersion::Java16 => 16,
            JavaRuntimeVersion::Java17 => 17,
            JavaRuntimeVersion::Java21 => 21,
            JavaRuntimeVersion::Java25 => 25,
        }
    }

    /// Human-readable label for UI display.
    pub const fn label(self) -> &'static str {
        match self {
            JavaRuntimeVersion::Java8 => "Java 8",
            JavaRuntimeVersion::Java16 => "Java 16",
            JavaRuntimeVersion::Java17 => "Java 17 (LTS)",
            JavaRuntimeVersion::Java21 => "Java 21 (LTS)",
            JavaRuntimeVersion::Java25 => "Java 25 (LTS)",
        }
    }

    /// Tooltip text providing additional information about this Java version.
    pub const fn info_tooltip(self) -> &'static str {
        match self {
            JavaRuntimeVersion::Java8 => "Legacy support for older modpacks",
            JavaRuntimeVersion::Java16 => "Required for Minecraft 1.17-1.19",
            JavaRuntimeVersion::Java17 => "Recommended for most modern instances",
            JavaRuntimeVersion::Java21 => "Latest LTS - best performance",
            JavaRuntimeVersion::Java25 => "Newest LTS version",
        }
    }
}
