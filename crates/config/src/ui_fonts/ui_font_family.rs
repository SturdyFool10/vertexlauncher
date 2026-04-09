use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UiFontFamily(pub(super) String);

impl UiFontFamily {
    pub fn included_default() -> Self {
        Self(INCLUDED_DEFAULT_UI_FONT_FAMILY.to_owned())
    }

    pub fn new(name: impl Into<String>) -> Self {
        Self(normalize_ui_font_family_name(name.into()))
    }

    pub fn is_included_default(&self) -> bool {
        self.matches_name(INCLUDED_DEFAULT_UI_FONT_FAMILY)
    }

    pub fn label(&self) -> &str {
        &self.0
    }

    pub fn settings_label(&self) -> String {
        if self.is_included_default() {
            format!("{} (Included default)", self.label())
        } else {
            self.label().to_owned()
        }
    }

    pub fn query_families(&self) -> Vec<&str> {
        if self.is_included_default() {
            MAPLE_FONT_FAMILIES.to_vec()
        } else {
            vec![self.label()]
        }
    }

    pub fn matches(&self, other: &Self) -> bool {
        normalized_ui_font_family_key(self.label()) == normalized_ui_font_family_key(other.label())
    }

    pub fn matches_name(&self, other: &str) -> bool {
        normalized_ui_font_family_key(self.label()) == normalized_ui_font_family_key(other)
    }

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
