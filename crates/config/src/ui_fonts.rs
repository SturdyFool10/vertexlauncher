use serde::{Deserialize, Deserializer, Serialize, Serializer};

const INCLUDED_DEFAULT_UI_FONT_FAMILY: &str = "Maple Mono NF";
const MAPLE_FONT_FAMILIES: &[&str] = &[
    INCLUDED_DEFAULT_UI_FONT_FAMILY,
    "Maple Mono",
    "Maple Mono Normal",
];
const INCLUDED_DEFAULT_EMOJI_FONT_FAMILY: &str = "Noto Color Emoji";
const INCLUDED_EMOJI_FONT_SETTINGS_LABEL: &str = "Noto Color Emoji (Included)";

#[path = "ui_fonts/ui_emoji_font_family.rs"]
mod ui_emoji_font_family;
#[path = "ui_fonts/ui_font_family.rs"]
mod ui_font_family;

pub use self::ui_emoji_font_family::UiEmojiFontFamily;
pub use self::ui_font_family::UiFontFamily;

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
