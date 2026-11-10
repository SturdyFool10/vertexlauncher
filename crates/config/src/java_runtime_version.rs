use vertex_i18n::src;
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

    /// The runtime with exactly this major version, if launcher-managed.
    pub const fn from_major(major: u8) -> Option<Self> {
        match major {
            8 => Some(JavaRuntimeVersion::Java8),
            16 => Some(JavaRuntimeVersion::Java16),
            17 => Some(JavaRuntimeVersion::Java17),
            21 => Some(JavaRuntimeVersion::Java21),
            25 => Some(JavaRuntimeVersion::Java25),
            _ => None,
        }
    }

    /// Java major version Minecraft `game_version` needs, or `None` if the id can't be read.
    ///
    /// Covers both numbering schemes: `1.x` (Java 8 through 21 by release) and the year-based
    /// `26.x` onward, which needs Java `major - 1`. Weekly snapshots of the new scheme
    /// (`26w05a`) count as their release line.
    pub fn required_major_for_game(game_version: &str) -> Option<u8> {
        let (major, minor, patch) = parse_game_version_parts(game_version)?;
        if major != 1 {
            return major
                .checked_sub(1)
                .and_then(|value| u8::try_from(value).ok());
        }
        Some(match minor {
            0..=16 => 8,
            17 => 16,
            21.. => u8::try_from(minor).ok()?,
            _ if minor > 20 || (minor == 20 && patch >= 5) => 21,
            _ => 17,
        })
    }

    /// The managed runtime to use for `game_version`: the oldest one that satisfies it.
    pub fn recommended_for_game(game_version: &str) -> Option<Self> {
        let required = Self::required_major_for_game(game_version)?;
        Self::ALL
            .into_iter()
            .find(|runtime| runtime.major() >= required)
            .or(Self::ALL.last().copied())
    }

    /// Human-readable label for UI display.
    pub const fn label(self) -> &'static str {
        match self {
            JavaRuntimeVersion::Java8 => src!("Java 8"),
            JavaRuntimeVersion::Java16 => src!("Java 16"),
            JavaRuntimeVersion::Java17 => src!("Java 17 (LTS)"),
            JavaRuntimeVersion::Java21 => src!("Java 21 (LTS)"),
            JavaRuntimeVersion::Java25 => src!("Java 25 (LTS)"),
        }
    }

    /// Tooltip text providing additional information about this Java version.
    pub const fn info_tooltip(self) -> &'static str {
        match self {
            JavaRuntimeVersion::Java8 => src!("Used by Minecraft 1.16.5 and older"),
            JavaRuntimeVersion::Java16 => src!("Used by Minecraft 1.17 to 1.17.1"),
            JavaRuntimeVersion::Java17 => src!("Used by Minecraft 1.18 to 1.20.4"),
            JavaRuntimeVersion::Java21 => src!("Used by Minecraft 1.20.5 to 1.21.x"),
            JavaRuntimeVersion::Java25 => src!("Used by Minecraft 26.1 and newer"),
        }
    }
}

/// `(major, minor, patch)` of a game version id, tolerant of `-pre1`, `-snapshot-3` suffixes.
fn parse_game_version_parts(game_version: &str) -> Option<(u32, u32, u32)> {
    fn number_prefix(value: &str) -> Option<u32> {
        let digits = value.bytes().take_while(u8::is_ascii_digit).count();
        value.get(..digits)?.parse().ok()
    }
    let trimmed = game_version.trim();
    if let Some((year, week)) = trimmed.split_once('w')
        && let Some(major) = number_prefix(year)
        && major >= 26
        && number_prefix(week).is_some()
    {
        return Some((major, 0, 0));
    }
    let mut parts = trimmed.split(['.', '-']);
    let major = number_prefix(parts.next()?)?;
    let minor = number_prefix(parts.next()?)?;
    let patch = parts.next().and_then(number_prefix).unwrap_or(0);
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_game_versions_to_java() {
        let required = JavaRuntimeVersion::required_major_for_game;
        assert_eq!(required("1.12.2"), Some(8));
        assert_eq!(required("1.16.5"), Some(8));
        assert_eq!(required("1.17.1"), Some(16));
        assert_eq!(required("1.18.2"), Some(17));
        assert_eq!(required("1.20.4"), Some(17));
        assert_eq!(required("1.20.5"), Some(21));
        assert_eq!(required("1.21.4"), Some(21));
        assert_eq!(required("26.3"), Some(25));
        assert_eq!(required("26w05a"), Some(25));
        assert_eq!(required("26.3-snapshot-10"), Some(25));
        assert_eq!(required("nonsense"), None);
    }

    #[test]
    fn recommends_the_oldest_runtime_that_fits() {
        let rec = JavaRuntimeVersion::recommended_for_game;
        assert_eq!(rec("1.16.5"), Some(JavaRuntimeVersion::Java8));
        assert_eq!(rec("1.17"), Some(JavaRuntimeVersion::Java16));
        assert_eq!(rec("1.19.4"), Some(JavaRuntimeVersion::Java17));
        assert_eq!(rec("1.21"), Some(JavaRuntimeVersion::Java21));
        assert_eq!(rec("27.1"), Some(JavaRuntimeVersion::Java25));
        assert_eq!(rec("30.1"), Some(JavaRuntimeVersion::Java25));
    }
}
