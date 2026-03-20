use config::{Config, DropdownSettingId, UiFontFamily};
use eframe::egui;
use fontloader::{FontCatalog, FontSpec, Slant, Stretch, Weight};
use launcher_ui::console;
use std::hash::{Hash, Hasher};
use textui::TextUi;

const MAPLE_MONO_NF_REGULAR_TTF: &[u8] =
    include_bytes!("../included_fonts/MapleMono-NF-Regular.ttf");

#[derive(Clone, Debug, PartialEq)]
struct AppliedFontSignature {
    family: UiFontFamily,
    size: f32,
    weight: i32,
}

#[derive(Clone, Debug, PartialEq)]
struct AppliedTextSignature {
    family: UiFontFamily,
    size: f32,
    weight: i32,
    open_type_features_enabled: bool,
    open_type_features_to_enable: String,
}

pub struct FontController {
    font_catalog: FontCatalog,
    available_ui_fonts: Vec<UiFontFamily>,
    applied_font_signature: Option<AppliedFontSignature>,
    applied_text_signature: Option<AppliedTextSignature>,
    effective_ui_font_family: UiFontFamily,
}

impl FontController {
    pub fn new(initial_family: UiFontFamily) -> Self {
        let mut catalog = FontCatalog::new();
        catalog.load_system();

        Self {
            available_ui_fonts: detect_available_ui_fonts(&catalog),
            font_catalog: catalog,
            applied_font_signature: None,
            applied_text_signature: None,
            effective_ui_font_family: initial_family,
        }
    }

    pub fn register_included_fonts(text_ui: &mut TextUi) {
        text_ui.register_font_data(MAPLE_MONO_NF_REGULAR_TTF.to_vec());
    }

    pub fn available_ui_fonts(&self) -> &[UiFontFamily] {
        &self.available_ui_fonts
    }

    pub fn ensure_selected_font_is_available(&self, config: &mut Config) {
        let available_ui_fonts = &self.available_ui_fonts;
        config.for_each_dropdown_mut(|setting, value| {
            if setting.id != DropdownSettingId::UiFontFamily {
                return;
            }

            if let Some(matching_font) = matching_available_font(available_ui_fonts, value) {
                if matching_font != value {
                    *value = matching_font.clone();
                }
            } else {
                *value = UiFontFamily::included_default();
            }
        });
    }

    pub fn apply_from_config(
        &mut self,
        ctx: &egui::Context,
        config: &Config,
        text_ui: &mut TextUi,
    ) {
        let desired_font = AppliedFontSignature {
            family: config.ui_font_family(),
            size: config.ui_font_size(),
            weight: config.ui_font_weight(),
        };

        if self.applied_font_signature.as_ref() != Some(&desired_font) {
            let should_register_text_font = self
                .applied_font_signature
                .as_ref()
                .is_none_or(|previous| !previous.family.matches(&desired_font.family));
            let mut applied_family = desired_font.family.clone();
            if desired_font.family.is_included_default() {
                install_included_maple_font(ctx, desired_font.size);
            } else {
                let family_candidates = desired_font.family.query_families();
                let spec = FontSpec::new(&family_candidates)
                    .weight(Weight(desired_font.weight.clamp(100, 900) as u16))
                    .slant(Slant::Upright)
                    .stretch(Stretch::Normal);

                if let Ok((bytes, _face_index)) = self.font_catalog.query_bytes(&spec) {
                    if should_register_text_font {
                        text_ui.register_font_data(bytes.clone());
                    }
                    let font_key = font_key(&desired_font.family);
                    fontloader::egui_integration::install_font_as_primary(
                        ctx,
                        &font_key,
                        bytes,
                        desired_font.size,
                    );
                } else {
                    tracing::warn!(
                        target: "vertexlauncher/app/fonts",
                        configured_font = desired_font.family.label(),
                        "configured UI font not available; falling back to included default"
                    );
                    install_included_maple_font(ctx, desired_font.size);
                    applied_family = UiFontFamily::included_default();
                }
            }

            self.effective_ui_font_family = applied_family;
            self.applied_font_signature = Some(desired_font);
        }

        let desired_text = AppliedTextSignature {
            family: self.effective_ui_font_family.clone(),
            size: config.ui_font_size(),
            weight: config.ui_font_weight(),
            open_type_features_enabled: config.open_type_features_enabled(),
            open_type_features_to_enable: config.open_type_features_to_enable().to_owned(),
        };
        let open_type_features_changed =
            self.applied_text_signature
                .as_ref()
                .is_some_and(|previous| {
                    previous.open_type_features_enabled != desired_text.open_type_features_enabled
                        || previous.open_type_features_to_enable
                            != desired_text.open_type_features_to_enable
                });

        if self.applied_text_signature == Some(desired_text.clone()) {
            return;
        }

        let family_candidates = self.effective_ui_font_family.query_families();
        text_ui.apply_typography(&family_candidates, desired_text.size, desired_text.weight);
        text_ui.apply_open_type_features(
            desired_text.open_type_features_enabled,
            &desired_text.open_type_features_to_enable,
            &family_candidates,
        );
        self.applied_text_signature = Some(desired_text);
        if open_type_features_changed {
            console::mark_text_for_redraw();
            ctx.request_repaint();
        }
    }
}

fn install_included_maple_font(ctx: &egui::Context, size_pt: f32) {
    let font_key = font_key(&UiFontFamily::included_default());
    fontloader::egui_integration::install_font_as_primary(
        ctx,
        &font_key,
        MAPLE_MONO_NF_REGULAR_TTF.to_vec(),
        size_pt,
    );
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
