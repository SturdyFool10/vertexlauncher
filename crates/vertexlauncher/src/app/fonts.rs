use config::{Config, DropdownSettingId, UiEmojiFontFamily, UiFontFamily};
use eframe::egui;
use fontloader::{FontCatalog, FontSpec, Slant, Stretch, Weight};
use launcher_ui::console;
use std::hash::{Hash, Hasher};
use textui::TextUi;

const MAPLE_MONO_NF_REGULAR_TTF: &[u8] =
    include_bytes!("../included_fonts/MapleMono-NF-Regular.ttf");

const NOTO_COLOR_EMOJI_TTF: &[u8] = include_bytes!("../included_fonts/NotoColorEmoji.ttf");

#[path = "fonts/applied_font_signature.rs"]
mod applied_font_signature;
#[path = "fonts/applied_text_signature.rs"]
mod applied_text_signature;
#[path = "fonts/font_controller.rs"]
mod font_controller;

use self::applied_font_signature::AppliedFontSignature;
use self::applied_text_signature::AppliedTextSignature;
pub use self::font_controller::FontController;

fn install_included_maple_font(ctx: &egui::Context, size_pt: f32) {
    let font_key = font_key(&UiFontFamily::included_default());
    fontloader::egui_integration::install_font_as_primary(
        ctx,
        &font_key,
        MAPLE_MONO_NF_REGULAR_TTF.to_vec(),
        size_pt,
    );
}

fn detect_available_emoji_fonts(font_catalog: &FontCatalog) -> Vec<UiEmojiFontFamily> {
    let mut available = vec![UiEmojiFontFamily::included_default()];

    for family_name in font_catalog.deduplicated_family_names() {
        let family = UiEmojiFontFamily::new(family_name);
        if family.is_included_default()
            || available.iter().any(|existing| existing.matches(&family))
        {
            continue;
        }
        available.push(family);
    }

    available
}

fn detect_available_ui_fonts(font_catalog: &FontCatalog) -> Vec<UiFontFamily> {
    let mut available = vec![UiFontFamily::included_default()];

    for family_name in font_catalog.deduplicated_family_names() {
        let family = UiFontFamily::new(family_name);
        if family.is_included_default()
            || available.iter().any(|existing| existing.matches(&family))
        {
            continue;
        }
        available.push(family);
    }

    available
}

fn matching_available_font<'a>(
    available_ui_fonts: &'a [UiFontFamily],
    desired_font: &UiFontFamily,
) -> Option<&'a UiFontFamily> {
    available_ui_fonts
        .iter()
        .find(|candidate| candidate.matches(desired_font))
}

fn font_key(family: &UiFontFamily) -> String {
    if family.is_included_default() {
        return "ui_font_maple_mono_nf".to_owned();
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    family.hash(&mut hasher);
    let hash = hasher.finish();
    let sanitized = family
        .label()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_owned();

    if sanitized.is_empty() {
        format!("ui_font_system_{hash:016x}")
    } else {
        format!("ui_font_system_{sanitized}_{hash:016x}")
    }
}
