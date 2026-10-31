//! UI fonts on SE5: the launcher's font choices become one engine font stack registered under font id 0, the id every
//! widget uses. Registering a new stack under the same id is a live font change, so the Text settings apply at once.

use std::sync::mpsc;

use config::{Config, UiCjkFontFamily, UiEmojiFontFamily, UiFontFamily};
use vertex_shell::sturdy::ui::{FontSource, FontStack, FontVariation};

use super::super::fonts::{
    MAPLE_MONO_NF_CN_REGULAR_TTF, MAPLE_MONO_NF_REGULAR_TTF, NOTO_COLOR_EMOJI_TTF,
};

/// The installed font families the Text settings offer, found by a background scan (scanning can take a while).
#[derive(Clone, Debug, Default)]
pub struct FontOptions {
    pub ui: Vec<UiFontFamily>,
    pub ui_labels: Vec<String>,
    pub emoji: Vec<UiEmojiFontFamily>,
    pub emoji_labels: Vec<String>,
    pub cjk: Vec<UiCjkFontFamily>,
    pub cjk_labels: Vec<String>,
}

impl FontOptions {
    /// Only the bundled fonts, until the scan finishes.
    pub fn included_only() -> Self {
        Self::from_lists(
            vec![UiFontFamily::included_default()],
            vec![UiEmojiFontFamily::included_default()],
            vec![UiCjkFontFamily::included_default()],
        )
    }

    fn from_lists(
        ui: Vec<UiFontFamily>,
        emoji: Vec<UiEmojiFontFamily>,
        cjk: Vec<UiCjkFontFamily>,
    ) -> Self {
        Self {
            ui_labels: ui.iter().map(UiFontFamily::settings_label).collect(),
            emoji_labels: emoji
                .iter()
                .map(UiEmojiFontFamily::settings_label)
                .collect(),
            cjk_labels: cjk.iter().map(UiCjkFontFamily::settings_label).collect(),
            ui,
            emoji,
            cjk,
        }
    }

    /// Lists the installed font families on a worker thread (the first scan of the font directories is slow); the
    /// receiver yields the full lists once. The emoji and CJK pickers only offer families that have glyphs for
    /// those scripts.
    pub fn scan() -> mpsc::Receiver<FontOptions> {
        /// A Han ideograph (Chinese, Japanese), a hiragana (Japanese) and a Hangul syllable (Korean).
        const CJK_PROBES: [char; 3] = ['\u{4E2D}', '\u{3042}', '\u{D55C}'];
        /// The grinning face.
        const EMOJI_PROBES: [char; 1] = ['\u{1F600}'];
        let (tx, rx) = mpsc::channel();
        let _ = launcher_runtime::spawn_blocking_detached(move || {
            let mut catalog = fontloader::FontCatalog::new();
            catalog.load_system();
            let mut ui = vec![UiFontFamily::included_default()];
            for name in catalog.deduplicated_family_names() {
                let family = UiFontFamily::new(name);
                if !family.is_included_default()
                    && !ui.iter().any(|existing| existing.matches(&family))
                {
                    ui.push(family);
                }
            }
            let mut emoji = vec![UiEmojiFontFamily::included_default()];
            for name in catalog.families_covering_any(&EMOJI_PROBES) {
                let family = UiEmojiFontFamily::new(name);
                if !family.is_included_default()
                    && !emoji.iter().any(|existing| existing.matches(&family))
                {
                    emoji.push(family);
                }
            }
            let mut cjk = vec![UiCjkFontFamily::included_default()];
            for name in catalog.families_covering_any(&CJK_PROBES) {
                let family = UiCjkFontFamily::new(name);
                if !family.is_included_default()
                    && !cjk.iter().any(|existing| existing.matches(&family))
                {
                    cjk.push(family);
                }
            }
            let _ = tx.send(Self::from_lists(ui, emoji, cjk));
            vertex_shell::wake();
        });
        rx
    }
}

/// What the registered stack was built from; a different value means the stack must be registered again.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSignature {
    typography: config::TypographySettings,
    ui: String,
    emoji: String,
    cjk: String,
    weight: i32,
    features: String,
}

impl FontSignature {
    pub fn of(config: &Config) -> Self {
        Self {
            typography: config.typography().clone(),
            ui: config.ui_font_family().label().to_owned(),
            emoji: config.ui_emoji_font_family().family_name().to_owned(),
            cjk: config.ui_cjk_font_family().family_name().to_owned(),
            weight: config.ui_font_weight(),
            features: open_type_features(config),
        }
    }
}

/// The engine font id a text role draws with. Id 0 is the launcher-wide stack; each role has its own so its size
/// stays tied to its own family, weight and script fonts.
pub fn role_font_id(role: config::TextRole) -> u16 {
    1 + config::TextRole::ALL
        .iter()
        .position(|candidate| *candidate == role)
        .unwrap_or(0) as u16
}

/// The roles Minecraft-styled text is drawn in: the body text (names), the small text (MOTDs) and the console's code text.
pub const STYLED_ROLES: [config::TextRole; 3] =
    [config::TextRole::Body, config::TextRole::Caption, config::TextRole::Code];

/// The faces a styled role can have, in the order of their font ids: bold, italic, bold italic.
const STYLED_VARIANTS: [(bool, bool); 3] = [(true, false), (false, true), (true, true)];

/// Which of a styled role's three faces (bold, italic, bold italic) the current font really has. A face the font cannot
/// make is simply not there, and the text is drawn without that attribute (bold still has the game's own double-draw).
pub type StyledFaces = [[bool; 3]; 3];

fn styled_font_id(slot: usize, variant: usize) -> u16 {
    1 + config::TextRole::ALL.len() as u16 + (slot * STYLED_VARIANTS.len() + variant) as u16
}

/// The role slot (an index into [`STYLED_ROLES`]) whose regular font id is `font_id`, if it is one.
pub fn styled_slot(font_id: u16) -> Option<usize> {
    STYLED_ROLES.iter().position(|role| role_font_id(*role) == font_id)
}

/// The engine font id of `slot`'s `variant` face (0 bold, 1 italic, 2 bold italic).
pub fn styled_face_id(slot: usize, variant: usize) -> u16 {
    styled_font_id(slot, variant)
}

/// The stack for the bold and/or italic face of `role`, or `None` when its font cannot make it. Asked of the font, not
/// assumed: a variable font declares a weight axis (`wght`) and a slant (`slnt`) or italic (`ital`) axis, which are
/// used within their declared range; an installed family is asked for its own bold/italic face; a static embedded or
/// file font has neither, so it has no such face.
fn styled_stack(config: &Config, role: config::TextRole, bold: bool, italic: bool) -> Option<FontStack> {
    let mut stack = stack_for_role(config, role);
    let axes = vertex_shell::sturdy::ui::font_variation_axes(&stack.primary).unwrap_or_default();
    let axis = |tag: &str| axes.iter().find(|axis| axis.tag == tag);
    let (mut bold_done, mut italic_done) = (!bold, !italic);
    if bold && let Some(weight) = axis("wght") {
        stack.variations.retain(|variation| variation.tag != "wght");
        stack = stack.variation(FontVariation::weight(700.0_f32.clamp(weight.min, weight.max)));
        bold_done = weight.max >= 600.0;
    }
    if italic {
        if axis("ital").is_some_and(|axis| axis.max >= 1.0) {
            stack = stack.variation(FontVariation { tag: "ital".into(), value: 1.0 });
            italic_done = true;
        } else if let Some(slant) = axis("slnt").filter(|axis| axis.min < 0.0) {
            // Negative degrees lean right; 12 is a usual italic angle, held to what the font allows.
            stack = stack.variation(FontVariation::slant((-12.0_f32).clamp(slant.min, slant.max.min(0.0))));
            italic_done = true;
        }
    }
    // What the axes did not give, an installed family may have as a face of its own.
    if !(bold_done && italic_done)
        && let FontSource::System { family, .. } = &stack.primary
    {
        stack.primary = FontSource::System {
            family: family.clone(),
            bold: bold && !bold_done,
            italic: italic && !italic_done,
        };
        bold_done = true;
        italic_done = true;
    }
    (bold_done && italic_done).then_some(stack)
}

/// Registers the bold/italic faces the current font supports and says which those are. Faces from a previous font are
/// removed first, so changing from a variable font to a static one leaves nothing behind.
pub fn register_styled_faces(
    engine: &mut vertex_shell::sturdy::Engine<'_>,
    config: &Config,
) -> StyledFaces {
    let mut faces: StyledFaces = [[false; 3]; 3];
    for (slot, role) in STYLED_ROLES.iter().enumerate() {
        for (variant, (bold, italic)) in STYLED_VARIANTS.iter().enumerate() {
            let id = styled_font_id(slot, variant);
            engine.unregister_ui_font(id);
            let Some(stack) = styled_stack(config, *role, *bold, *italic) else {
                continue;
            };
            match engine.register_ui_font_stack(id, &stack) {
                Ok(()) => faces[slot][variant] = true,
                Err(err) => {
                    tracing::debug!(target: "vertexlauncher/fonts", "no {} face for {}: {err}", variant, role.label());
                }
            }
        }
    }
    faces
}

/// The stack for one text role: the role's own family/weight/script fonts where set, else the launcher-wide ones.
pub fn stack_for_role(config: &Config, role: config::TextRole) -> FontStack {
    let resolved = config.typography().resolve(role);
    let ui = config.ui_font_family();
    let primary = match resolved.font_family.as_deref() {
        Some(name) => FontSource::system(name),
        None if ui.is_included_default() => FontSource::Bytes(MAPLE_MONO_NF_REGULAR_TTF.to_vec()),
        None => FontSource::system(ui.label()),
    };
    let emoji = match resolved.emoji_font_family.as_deref() {
        Some(name) => FontSource::system(name),
        None => emoji_source(config),
    };
    let cjk = match resolved.cjk_font_family.as_deref() {
        Some(name) => FontSource::system(name),
        None => cjk_source(config),
    };
    let mut stack = FontStack::new(primary).emoji(emoji).fallback(cjk);
    stack = stack.fallback(FontSource::Bytes(MAPLE_MONO_NF_REGULAR_TTF.to_vec()));
    stack.features = open_type_features(config);
    // The UI-wide weight stacks on the role's own (400 is neutral).
    let weight = (i32::from(resolved.weight) + config.ui_font_weight() - 400).clamp(100, 900);
    stack.variation(FontVariation::weight(weight as f32))
}

fn emoji_source(config: &Config) -> FontSource {
    let family = config.ui_emoji_font_family();
    if family.is_included_default() {
        FontSource::Bytes(NOTO_COLOR_EMOJI_TTF.to_vec())
    } else {
        FontSource::system(family.family_name())
    }
}

fn cjk_source(config: &Config) -> FontSource {
    let family = config.ui_cjk_font_family();
    if family.is_included_default() {
        FontSource::Bytes(MAPLE_MONO_NF_CN_REGULAR_TTF.to_vec())
    } else {
        FontSource::system(family.family_name())
    }
}

/// The engine font stack for the user's font choices: primary face, colour emoji, then the CJK face as a fallback.
/// The bundled fonts are embedded, so they work without being installed.
pub fn stack_for(config: &Config) -> FontStack {
    let ui = config.ui_font_family();
    let primary = if ui.is_included_default() {
        FontSource::Bytes(MAPLE_MONO_NF_REGULAR_TTF.to_vec())
    } else {
        FontSource::system(ui.label())
    };
    let emoji_family = config.ui_emoji_font_family();
    let emoji = if emoji_family.is_included_default() {
        FontSource::Bytes(NOTO_COLOR_EMOJI_TTF.to_vec())
    } else {
        FontSource::system(emoji_family.family_name())
    };
    let cjk_family = config.ui_cjk_font_family();
    let cjk = if cjk_family.is_included_default() {
        FontSource::Bytes(MAPLE_MONO_NF_CN_REGULAR_TTF.to_vec())
    } else {
        FontSource::system(cjk_family.family_name())
    };
    let mut stack = FontStack::new(primary).emoji(emoji).fallback(cjk);
    if !ui.is_included_default() {
        // The bundled face backs up a system font that lacks a glyph.
        stack = stack.fallback(FontSource::Bytes(MAPLE_MONO_NF_REGULAR_TTF.to_vec()));
    }
    stack.features = open_type_features(config);
    stack.variation(FontVariation::weight(config.ui_font_weight() as f32))
}

/// The OpenType feature list in the engine's comma-separated form: the user's list, or the built-in one when theirs is
/// empty, or nothing (the font's own defaults) when the setting is off.
fn open_type_features(config: &Config) -> String {
    if !config.open_type_features_enabled() {
        return String::new();
    }
    let list = |text: &str| {
        text.split(|ch: char| ch == ',' || ch.is_whitespace())
            .map(str::trim)
            .filter(|feature| !feature.is_empty())
            .collect::<Vec<_>>()
            .join(",")
    };
    let user = list(config.open_type_features_to_enable());
    if user.is_empty() {
        list(config::DEFAULT_OPEN_TYPE_FEATURE_TAGS)
    } else {
        user
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_are_empty_unless_enabled() {
        let mut config = Config::default();
        config.for_each_text_mut(|spec, value| {
            if spec.id == config::TextSettingId::OpenTypeFeaturesToEnable {
                *value = "liga, ss01 -calt".to_owned();
            }
        });
        config.for_each_toggle_mut(|spec, value| {
            if spec.id == config::ToggleSettingId::OpenTypeFeaturesEnabled {
                *value = false;
            }
        });
        assert_eq!(open_type_features(&config), "");
        config.for_each_toggle_mut(|spec, value| {
            if spec.id == config::ToggleSettingId::OpenTypeFeaturesEnabled {
                *value = true;
            }
        });
        assert_eq!(open_type_features(&config), "liga,ss01,-calt");
    }
}
