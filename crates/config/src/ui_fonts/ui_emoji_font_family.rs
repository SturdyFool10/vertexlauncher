use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UiEmojiFontFamily(pub(super) String);

impl UiEmojiFontFamily {
    pub fn included_default() -> Self {
        Self(INCLUDED_EMOJI_FONT_SETTINGS_LABEL.to_owned())
    }

    pub fn new(name: impl Into<String>) -> Self {
        Self(normalize_emoji_font_family_name(name.into()))
    }

    pub fn is_included_default(&self) -> bool {
        normalize_emoji_font_family_key(&self.0)
            == normalize_emoji_font_family_key(INCLUDED_EMOJI_FONT_SETTINGS_LABEL)
            || normalize_emoji_font_family_key(&self.0)
                == normalize_emoji_font_family_key(INCLUDED_DEFAULT_EMOJI_FONT_FAMILY)
    }

    pub fn label(&self) -> &str {
        &self.0
    }

    pub fn settings_label(&self) -> String {
        if self.is_included_default() {
            INCLUDED_EMOJI_FONT_SETTINGS_LABEL.to_owned()
        } else {
            self.0.clone()
        }
    }

    pub fn family_name(&self) -> &str {
        if self.is_included_default() {
            INCLUDED_DEFAULT_EMOJI_FONT_FAMILY
        } else {
            &self.0
        }
    }

    pub fn matches(&self, other: &Self) -> bool {
        normalize_emoji_font_family_key(self.family_name())
            == normalize_emoji_font_family_key(other.family_name())
    }

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
