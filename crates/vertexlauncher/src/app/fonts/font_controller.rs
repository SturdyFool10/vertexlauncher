use super::*;

pub struct FontController {
    pub(super) font_catalog: FontCatalog,
    pub(super) available_ui_fonts: Vec<UiFontFamily>,
    pub(super) available_emoji_fonts: Vec<UiEmojiFontFamily>,
    pub(super) applied_font_signature: Option<AppliedFontSignature>,
    pub(super) applied_text_signature: Option<AppliedTextSignature>,
    pub(super) applied_emoji_font: Option<UiEmojiFontFamily>,
    pub(super) effective_ui_font_family: UiFontFamily,
}

impl FontController {
    pub fn new(initial_family: UiFontFamily) -> Self {
        let mut catalog = FontCatalog::new();
        catalog.load_system();

        Self {
            available_ui_fonts: detect_available_ui_fonts(&catalog),
            available_emoji_fonts: detect_available_emoji_fonts(&catalog),
            font_catalog: catalog,
            applied_font_signature: None,
            applied_text_signature: None,
            applied_emoji_font: None,
            effective_ui_font_family: initial_family,
        }
    }

    pub fn register_included_fonts(text_ui: &mut TextUi) {
        text_ui.register_font_data(MAPLE_MONO_NF_REGULAR_TTF.to_vec());
        text_ui.register_font_data(NOTO_COLOR_EMOJI_TTF.to_vec());
    }

    pub fn available_ui_fonts(&self) -> &[UiFontFamily] {
        &self.available_ui_fonts
    }

    pub fn available_emoji_fonts(&self) -> &[UiEmojiFontFamily] {
        &self.available_emoji_fonts
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

        let desired_emoji_font = config.ui_emoji_font_family();
        if self
            .applied_emoji_font
            .as_ref()
            .is_none_or(|prev| !prev.matches(&desired_emoji_font))
        {
            if !desired_emoji_font.is_included_default() {
                let family_name = desired_emoji_font.family_name().to_owned();
                let family_candidates: Vec<&str> = vec![family_name.as_str()];
                let spec = FontSpec::new(&family_candidates)
                    .weight(Weight(400))
                    .slant(Slant::Upright)
                    .stretch(Stretch::Normal);
                if let Ok((bytes, _)) = self.font_catalog.query_bytes(&spec) {
                    text_ui.register_font_data(bytes);
                } else {
                    tracing::warn!(
                        target: "vertexlauncher/app/fonts",
                        configured_font = desired_emoji_font.label(),
                        "configured emoji font not available; falling back to included default"
                    );
                }
            }
            self.applied_emoji_font = Some(desired_emoji_font);
        }
    }
}
