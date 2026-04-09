use super::*;

pub(crate) fn parse_feature_tag_list(feature_tags_csv: &str) -> Vec<[u8; 4]> {
    let mut tags = BTreeSet::new();
    for token in feature_tags_csv.split(',') {
        let raw = token.trim();
        if raw.len() != 4 || !raw.is_ascii() {
            continue;
        }

        let mut tag = [0_u8; 4];
        for (index, byte) in raw.as_bytes().iter().enumerate() {
            tag[index] = byte.to_ascii_lowercase();
        }
        tags.insert(tag);
    }

    tags.into_iter().collect()
}

fn build_font_features_from_settings(
    settings: impl IntoIterator<Item = ([u8; 4], u16)>,
) -> Option<FontFeatures> {
    let mut features = FontFeatures::new();
    let mut any = false;
    for (tag, value) in settings {
        features.set(cosmic_text::FeatureTag::new(&tag), value.into());
        any = true;
    }
    any.then_some(features)
}

pub(crate) fn compose_font_features(
    global_feature_tags: &[[u8; 4]],
    fundamentals: &TextFundamentals,
) -> Option<FontFeatures> {
    let mut settings = std::collections::BTreeMap::<[u8; 4], u16>::new();
    for tag in global_feature_tags {
        settings.insert(*tag, 1);
    }
    match fundamentals.kerning {
        TextKerning::Auto => {}
        TextKerning::Normal => {
            settings.insert(*b"kern", 1);
        }
        TextKerning::None => {
            settings.insert(*b"kern", 0);
        }
    }
    settings.insert(*b"liga", u16::from(fundamentals.standard_ligatures));
    settings.insert(*b"calt", u16::from(fundamentals.contextual_alternates));
    settings.insert(*b"dlig", u16::from(fundamentals.discretionary_ligatures));
    settings.insert(*b"hlig", u16::from(fundamentals.historical_ligatures));
    settings.insert(*b"case", u16::from(fundamentals.case_sensitive_forms));
    settings.insert(*b"zero", u16::from(fundamentals.slashed_zero));
    settings.insert(*b"tnum", u16::from(fundamentals.tabular_numbers));
    for feature in &fundamentals.feature_settings {
        settings.insert(feature.tag, feature.value);
    }
    build_font_features_from_settings(settings)
}

pub(crate) fn build_font_features(tags: &[[u8; 4]]) -> FontFeatures {
    build_font_features_from_settings(tags.iter().copied().map(|tag| (tag, 1)))
        .unwrap_or_else(FontFeatures::new)
}

#[inline]
fn should_hint(display_scale: f32) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = display_scale;
        false
    }

    #[cfg(target_os = "windows")]
    {
        display_scale < 1.5
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        display_scale < 1.5
    }
}

#[inline]
pub(crate) fn resolved_hinting_enabled(
    display_scale: f32,
    rasterization: TextRasterizationConfig,
) -> bool {
    match rasterization.hinting {
        TextHintingMode::Enabled => true,
        TextHintingMode::Disabled => false,
        TextHintingMode::Auto => should_hint(display_scale),
    }
}

#[inline]
pub(crate) fn resolved_stem_darkening_strength(
    ppem: f32,
    style_enabled: bool,
    rasterization: TextRasterizationConfig,
) -> f32 {
    let enabled = match rasterization.stem_darkening {
        TextStemDarkeningMode::Enabled => true,
        TextStemDarkeningMode::Disabled => false,
        TextStemDarkeningMode::Auto => style_enabled,
    };
    if !enabled {
        return 0.0;
    }

    let min_ppem = rasterization.stem_darkening_min_ppem.max(0.0);
    let max_ppem = rasterization.stem_darkening_max_ppem.max(min_ppem);
    let max_strength = rasterization.stem_darkening_max_strength.max(0.0);
    if ppem >= max_ppem {
        0.0
    } else if ppem <= min_ppem {
        max_strength
    } else {
        max_strength * (1.0 - (ppem - min_ppem) / (max_ppem - min_ppem))
    }
}

#[inline]
pub(crate) fn opsz_for_font_size(font_size_pt: f32) -> f32 {
    font_size_pt.clamp(8.0, 144.0)
}

fn font_family_available(db: &fontdb::Database, family: &str) -> bool {
    db.faces().any(|face| {
        face.families
            .iter()
            .any(|family_name| family_name.0.eq_ignore_ascii_case(family))
    })
}

fn choose_available_family<'a>(db: &fontdb::Database, families: &'a [&'a str]) -> Option<&'a str> {
    families
        .iter()
        .copied()
        .find(|family| font_family_available(db, family))
}

pub(crate) fn configure_text_font_defaults(font_system: &mut FontSystem) {
    let db = font_system.db_mut();

    #[cfg(target_os = "macos")]
    {
        if let Some(family) =
            choose_available_family(db, &["SF Pro Text", ".SF NS", "Helvetica Neue"])
        {
            db.set_sans_serif_family(family);
        }
        if let Some(family) = choose_available_family(db, &["SF Mono", "Menlo", "Monaco"]) {
            db.set_monospace_family(family);
        }
        if let Some(family) = choose_available_family(db, &["Times New Roman", "Times"]) {
            db.set_serif_family(family);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(family) =
            choose_available_family(db, &["Segoe UI Variable", "Segoe UI", "Arial"])
        {
            db.set_sans_serif_family(family);
        }
        if let Some(family) =
            choose_available_family(db, &["Cascadia Mono", "Consolas", "Courier New"])
        {
            db.set_monospace_family(family);
        }
        if let Some(family) = choose_available_family(db, &["Times New Roman", "Georgia"]) {
            db.set_serif_family(family);
        }
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        if let Some(family) = choose_available_family(
            db,
            &["Inter", "Noto Sans", "Cantarell", "Ubuntu", "DejaVu Sans"],
        ) {
            db.set_sans_serif_family(family);
        }
        if let Some(family) = choose_available_family(
            db,
            &["Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono"],
        ) {
            db.set_monospace_family(family);
        }
        if let Some(family) =
            choose_available_family(db, &["Noto Serif", "DejaVu Serif", "Liberation Serif"])
        {
            db.set_serif_family(family);
        }
    }
}
