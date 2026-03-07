use config::{Config, DropdownSettingId, UiFontFamily};
use eframe::egui;
use fontloader::{FontCatalog, FontSpec, Slant, Stretch, Weight};
use textui::TextUi;

const MAPLE_MONO_NF_REGULAR_TTF: &[u8] =
    include_bytes!("../included_fonts/MapleMono-NF-Regular.ttf");

#[derive(Clone, Copy, Debug, PartialEq)]
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
            if matches!(setting.id, DropdownSettingId::UiFontFamily)
                && !available_ui_fonts.contains(value)
            {
                *value = UiFontFamily::MapleMonoNf;
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

        if self.applied_font_signature != Some(desired_font) {
            let mut applied_family = desired_font.family;
            if desired_font.family.is_included_default() {
                install_included_maple_font(ctx, desired_font.size);
            } else {
                let spec = FontSpec::new(desired_font.family.query_families())
                    .weight(Weight(desired_font.weight.clamp(100, 900) as u16))
                    .slant(Slant::Upright)
                    .stretch(Stretch::Normal);

                if let Ok((bytes, _face_index)) = self.font_catalog.query_bytes(&spec) {
                    fontloader::egui_integration::install_font_as_primary(
                        ctx,
                        font_key(desired_font.family),
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
                    applied_family = UiFontFamily::MapleMonoNf;
                }
            }

            self.effective_ui_font_family = applied_family;
            self.applied_font_signature = Some(desired_font);
        }

        let desired_text = AppliedTextSignature {
            family: self.effective_ui_font_family,
            size: config.ui_font_size(),
            weight: config.ui_font_weight(),
            open_type_features_enabled: config.open_type_features_enabled(),
            open_type_features_to_enable: config.open_type_features_to_enable().to_owned(),
        };

        if self.applied_text_signature == Some(desired_text.clone()) {
            return;
        }

        text_ui.apply_typography(
            self.effective_ui_font_family.query_families(),
            desired_text.size,
            desired_text.weight,
        );
        text_ui.apply_open_type_features(
            desired_text.open_type_features_enabled,
            &desired_text.open_type_features_to_enable,
            self.effective_ui_font_family.query_families(),
        );
        self.applied_text_signature = Some(desired_text);
    }
}

fn install_included_maple_font(ctx: &egui::Context, size_pt: f32) {
    fontloader::egui_integration::install_font_as_primary(
        ctx,
        font_key(UiFontFamily::MapleMonoNf),
        MAPLE_MONO_NF_REGULAR_TTF.to_vec(),
        size_pt,
    );
}

fn detect_available_ui_fonts(font_catalog: &FontCatalog) -> Vec<UiFontFamily> {
    let mut available = vec![UiFontFamily::MapleMonoNf];

    for candidate in UiFontFamily::system_options() {
        let spec = FontSpec::new(candidate.query_families())
            .weight(Weight::REGULAR)
            .slant(Slant::Upright)
            .stretch(Stretch::Normal);

        if font_catalog.query(&spec).is_ok() {
            available.push(*candidate);
        }
    }

    available
}

fn font_key(family: UiFontFamily) -> &'static str {
    match family {
        UiFontFamily::MapleMonoNf => "ui_font_maple_mono_nf",
        UiFontFamily::JetBrainsMono => "ui_font_jetbrains_mono",
        UiFontFamily::FiraCode => "ui_font_fira_code",
        UiFontFamily::CascadiaCode => "ui_font_cascadia_code",
        UiFontFamily::Iosevka => "ui_font_iosevka",
    }
}
