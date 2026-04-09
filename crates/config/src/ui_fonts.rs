use serde::{Deserialize, Deserializer, Serialize, Serializer};

const INCLUDED_DEFAULT_UI_FONT_FAMILY: &str = "Maple Mono NF";
const MAPLE_FONT_FAMILIES: &[&str] = &[
    INCLUDED_DEFAULT_UI_FONT_FAMILY,
    "Maple Mono",
    "Maple Mono Normal",
];
const INCLUDED_DEFAULT_EMOJI_FONT_FAMILY: &str = "Noto Color Emoji";
const INCLUDED_EMOJI_FONT_SETTINGS_LABEL: &str = "Noto Color Emoji (Included)";

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
        normalize_emoji_font_family_key(&self.0)
            == normalize_emoji_font_family_key(INCLUDED_EMOJI_FONT_SETTINGS_LABEL)
            || normalize_emoji_font_family_key(&self.0)
                == normalize_emoji_font_family_key(INCLUDED_DEFAULT_EMOJI_FONT_FAMILY)
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
