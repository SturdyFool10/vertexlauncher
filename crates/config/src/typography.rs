use vertex_i18n::src;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const ROLE_FONT_SIZE_MIN: f32 = 8.0;
pub const ROLE_FONT_SIZE_MAX: f32 = 64.0;
pub const ROLE_FONT_SIZE_STEP: f32 = 0.5;

/// The kinds of text the launcher renders. Every label style is built from one of these,
/// so the size, weight and font of each kind can be changed in one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TextRole {
    PageHeading,
    SectionHeading,
    ModalTitle,
    Subtitle,
    StatLabel,
    Body,
    BodyStrong,
    Caption,
    Badge,
    Button,
    Input,
    Code,
}

/// Built-in look of a [`TextRole`] before any user override.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoleDefaults {
    pub size: f32,
    pub line_height: f32,
    pub weight: i32,
    pub letter_spacing: f32,
    pub monospace: bool,
}

impl TextRole {
    pub const ALL: [TextRole; 12] = [
        TextRole::PageHeading,
        TextRole::SectionHeading,
        TextRole::ModalTitle,
        TextRole::Subtitle,
        TextRole::StatLabel,
        TextRole::Body,
        TextRole::BodyStrong,
        TextRole::Caption,
        TextRole::Badge,
        TextRole::Button,
        TextRole::Input,
        TextRole::Code,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::PageHeading => "page_heading",
            Self::SectionHeading => "section_heading",
            Self::ModalTitle => "modal_title",
            Self::Subtitle => "subtitle",
            Self::StatLabel => "stat_label",
            Self::Body => "body",
            Self::BodyStrong => "body_strong",
            Self::Caption => "caption",
            Self::Badge => "badge",
            Self::Button => "button",
            Self::Input => "input",
            Self::Code => "code",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::PageHeading => src!("Page headings"),
            Self::SectionHeading => src!("Section headings"),
            Self::ModalTitle => src!("Dialog titles"),
            Self::Subtitle => src!("Subtitles"),
            Self::StatLabel => src!("Small headings and stats"),
            Self::Body => src!("Body text"),
            Self::BodyStrong => src!("Emphasized body text"),
            Self::Caption => src!("Captions"),
            Self::Badge => src!("Badges and tags"),
            Self::Button => src!("Button labels"),
            Self::Input => src!("Text fields"),
            Self::Code => src!("Console and code"),
        }
    }

    pub const fn defaults(self) -> RoleDefaults {
        const fn role(
            size: f32,
            line_height: f32,
            weight: i32,
            letter_spacing: f32,
            monospace: bool,
        ) -> RoleDefaults {
            RoleDefaults {
                size,
                line_height,
                weight,
                letter_spacing,
                monospace,
            }
        }
        match self {
            Self::PageHeading => role(34.0, 42.0, 700, 0.5, false),
            Self::SectionHeading => role(25.0, 33.0, 600, 0.3, false),
            Self::ModalTitle => role(26.0, 34.0, 700, 0.3, false),
            Self::Subtitle => role(22.0, 28.0, 700, 0.2, false),
            Self::StatLabel => role(16.0, 22.0, 600, 0.0, false),
            Self::Body => role(18.0, 27.0, 400, 0.0, false),
            Self::BodyStrong => role(18.0, 27.0, 600, 0.0, false),
            Self::Caption => role(13.0, 18.0, 400, -0.15, false),
            Self::Badge => role(11.0, 14.0, 600, 0.6, false),
            Self::Button => role(18.0, 24.0, 400, 0.0, false),
            Self::Input => role(17.0, 22.0, 400, 0.0, false),
            Self::Code => role(16.0, 22.0, 400, 0.0, true),
        }
    }
}

/// User overrides for one role. `None` fields keep the built-in value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<i32>,
    /// Named font family; `None` uses the launcher UI font (or monospace for `Code`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// Font for CJK characters in this group; `None` uses the launcher-wide CJK font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cjk_font_family: Option<String>,
    /// Font for emoji in this group; `None` uses the launcher-wide emoji font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji_font_family: Option<String>,
}

impl RoleOverride {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Fully resolved typography for a role.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTypography {
    pub size: f32,
    pub line_height: f32,
    pub weight: u16,
    pub letter_spacing: f32,
    pub monospace: bool,
    pub font_family: Option<String>,
    /// This group's own CJK font, or `None` to use the launcher-wide one.
    pub cjk_font_family: Option<String>,
    /// This group's own emoji font, or `None` to use the launcher-wide one.
    pub emoji_font_family: Option<String>,
}

/// Per-role typography overrides, stored by role slug.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TypographySettings {
    overrides: BTreeMap<String, RoleOverride>,
}

impl TypographySettings {
    pub fn override_for(&self, role: TextRole) -> RoleOverride {
        self.overrides.get(role.slug()).cloned().unwrap_or_default()
    }

    /// Replaces the override for `role`; an empty override removes the entry.
    pub fn set_override(&mut self, role: TextRole, value: RoleOverride) {
        if value.is_empty() {
            self.overrides.remove(role.slug());
        } else {
            self.overrides.insert(role.slug().to_owned(), value);
        }
    }

    pub fn reset(&mut self, role: TextRole) {
        self.overrides.remove(role.slug());
    }

    pub fn resolve(&self, role: TextRole) -> ResolvedTypography {
        let defaults = role.defaults();
        let user = self.override_for(role);
        let size = user
            .size
            .unwrap_or(defaults.size)
            .clamp(ROLE_FONT_SIZE_MIN, ROLE_FONT_SIZE_MAX);
        let named = |family: &Option<String>| {
            family
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        };
        let font_family = named(&user.font_family);
        let cjk_font_family = named(&user.cjk_font_family);
        let emoji_font_family = named(&user.emoji_font_family);
        ResolvedTypography {
            size,
            // Keep the built-in line-height ratio when the size changes.
            line_height: (defaults.line_height * size / defaults.size).round(),
            weight: user.weight.unwrap_or(defaults.weight).clamp(100, 900) as u16,
            letter_spacing: defaults.letter_spacing,
            monospace: defaults.monospace && font_family.is_none(),
            font_family,
            cjk_font_family,
            emoji_font_family,
        }
    }

    pub(crate) fn normalize(&mut self) {
        let known: Vec<&str> = TextRole::ALL.iter().map(|role| role.slug()).collect();
        self.overrides.retain(|slug, value| {
            if !known.contains(&slug.as_str()) {
                return false;
            }
            value.size = value
                .size
                .map(|size| size.clamp(ROLE_FONT_SIZE_MIN, ROLE_FONT_SIZE_MAX));
            value.weight = value.weight.map(|weight| weight.clamp(100, 900));
            for family in [
                &mut value.font_family,
                &mut value.cjk_font_family,
                &mut value.emoji_font_family,
            ] {
                *family = family
                    .take()
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty());
            }
            !value.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_builtin_styles_and_scale_line_height() {
        let mut settings = TypographySettings::default();
        let body = settings.resolve(TextRole::Body);
        assert_eq!(
            (body.size, body.line_height, body.weight),
            (18.0, 27.0, 400)
        );

        settings.set_override(
            TextRole::Body,
            RoleOverride {
                size: Some(36.0),
                ..RoleOverride::default()
            },
        );
        assert_eq!(settings.resolve(TextRole::Body).line_height, 54.0);
    }

    #[test]
    fn code_role_switches_off_monospace_when_family_is_named() {
        let mut settings = TypographySettings::default();
        assert!(settings.resolve(TextRole::Code).monospace);
        settings.set_override(
            TextRole::Code,
            RoleOverride {
                font_family: Some("JetBrains Mono".to_owned()),
                ..RoleOverride::default()
            },
        );
        let code = settings.resolve(TextRole::Code);
        assert!(!code.monospace);
        assert_eq!(code.font_family.as_deref(), Some("JetBrains Mono"));
    }

    #[test]
    fn script_fonts_default_to_the_launcher_wide_choice_and_can_be_set_per_group() {
        let mut settings = TypographySettings::default();
        let body = settings.resolve(TextRole::Body);
        assert_eq!((body.cjk_font_family, body.emoji_font_family), (None, None));

        settings.set_override(
            TextRole::Body,
            RoleOverride {
                cjk_font_family: Some("  Noto Sans CJK JP ".to_owned()),
                emoji_font_family: Some("Twemoji".to_owned()),
                ..RoleOverride::default()
            },
        );
        let body = settings.resolve(TextRole::Body);
        assert_eq!(body.cjk_font_family.as_deref(), Some("Noto Sans CJK JP"));
        assert_eq!(body.emoji_font_family.as_deref(), Some("Twemoji"));
        // Another group is untouched, and the main font is independent of both.
        let caption = settings.resolve(TextRole::Caption);
        assert_eq!(caption.cjk_font_family, None);
        assert_eq!(body.font_family, None);
        assert!(!body.monospace, "script fonts do not change the main font");
    }

    #[test]
    fn script_font_overrides_round_trip_and_blank_ones_are_dropped() {
        let mut settings = TypographySettings::default();
        settings.set_override(
            TextRole::Button,
            RoleOverride {
                cjk_font_family: Some("Noto Sans CJK JP".to_owned()),
                ..RoleOverride::default()
            },
        );
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("cjk_font_family") && !json.contains("emoji_font_family"));
        let mut back: TypographySettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, settings);

        back.set_override(
            TextRole::Button,
            RoleOverride {
                cjk_font_family: Some("   ".to_owned()),
                ..RoleOverride::default()
            },
        );
        back.normalize();
        assert_eq!(back.resolve(TextRole::Button).cjk_font_family, None);
    }
}
