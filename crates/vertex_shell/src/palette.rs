//! Maps a launcher [`Theme`] onto the engine's widget styles, so every screen asks one place for "what does a primary
//! button look like" instead of building colours by hand. This is the SE5 counterpart of `Theme::apply` for egui, and
//! keeps its rules: hover mixes text colour into the idle fill, the accent state draws `on_primary` text.

use sturdy::glam::Vec4;
use sturdy::widgets::{ButtonStyle, DropdownStyle, SliderStyle, TextInputStyle, ToggleStyle};
use vertex_theme::{Oklch, Theme};

/// How much of the text colour is mixed into a widget's idle fill to get its hovered fill (same as the egui build).
const HOVER_FILL_TEXT_MIX: f32 = 0.14;
const CORNER_RADIUS: f32 = 14.0;

fn rgb(color: Oklch, alpha: f32) -> Vec4 {
    let [r, g, b] = color.to_srgb8();
    Vec4::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, alpha)
}

fn mix(from: Vec4, to: Vec4, amount: f32) -> Vec4 {
    from.lerp(to, amount).with_w(from.w)
}

/// Every colour a screen needs, resolved from a theme and the window's UI opacity.
///
/// Colours are sRGB-encoded floats, which is what the engine's UI pass expects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg_dark: Vec4,
    pub bg: Vec4,
    pub bg_light: Vec4,
    pub text: Vec4,
    pub text_muted: Vec4,
    pub on_primary: Vec4,
    pub highlight: Vec4,
    pub border: Vec4,
    pub border_muted: Vec4,
    pub primary: Vec4,
    pub secondary: Vec4,
    pub danger: Vec4,
    pub warning: Vec4,
    pub success: Vec4,
    pub info: Vec4,
    /// Seconds for hover/press fades; zero when the theme asks for reduced motion.
    pub transition_seconds: f32,
}

impl Palette {
    /// `ui_opacity_percent` scales the alpha of the panel fills only (as the egui build does), so text stays solid
    /// while the window shows the desktop through it.
    pub fn new(theme: &Theme, ui_opacity_percent: u8) -> Self {
        let panel_alpha = f32::from(ui_opacity_percent.min(100)) / 100.0;
        let text = rgb(theme.text, 1.0);
        Self {
            bg_dark: rgb(theme.bg_dark, 1.0),
            bg: rgb(theme.bg, panel_alpha),
            bg_light: rgb(theme.bg_light, panel_alpha),
            text,
            text_muted: rgb(theme.text_muted, 1.0),
            on_primary: theme.on_primary.map_or(text, |color| rgb(color, 1.0)),
            highlight: rgb(theme.highlight, 1.0),
            border: rgb(theme.border, 1.0),
            border_muted: rgb(theme.border_muted, 1.0),
            primary: rgb(theme.primary, 1.0),
            secondary: rgb(theme.secondary, 1.0),
            danger: rgb(theme.danger, 1.0),
            warning: rgb(theme.warning, 1.0),
            success: rgb(theme.success, 1.0),
            info: rgb(theme.info, 1.0),
            transition_seconds: if theme.motion.reduced_motion {
                0.0
            } else {
                theme.motion.animation_time_seconds.clamp(0.0, 1.0)
            },
        }
    }

    /// The layout inspector's colours drawn from this palette, so the inspector looks like the launcher in whatever theme it is in.
    /// Everything is opaque (the UI opacity setting is for the launcher's own panels): the inspector sits on top of the window.
    pub fn debug_view_theme(&self) -> sturdy::ui::DebugViewTheme {
        let opaque = |color: Vec4| color.with_w(1.0);
        let surface = opaque(self.bg_dark);
        let surface_alt = opaque(mix(self.bg_dark, self.bg_light, 0.6));
        sturdy::ui::DebugViewTheme {
            surface,
            surface_alt,
            muted: opaque(self.text_muted),
            text: opaque(self.text),
            selected_row: opaque(mix(surface, self.primary, 0.45)),
            highlight: self.primary.with_w(0.35),
            close: opaque(self.danger),
            warning: opaque(self.warning),
            tag_border: opaque(self.border),
            tag_floating: opaque(self.secondary),
            tag_scroll: opaque(self.warning),
            tag_aspect: opaque(self.highlight),
            tag_image: opaque(self.success),
            tag_text: opaque(self.info),
            tag_custom: opaque(self.danger),
            tag_shared: opaque(self.primary),
        }
    }

    pub fn hovered_fill(&self) -> Vec4 {
        mix(self.bg_light, self.text, HOVER_FILL_TEXT_MIX)
    }

    fn disabled(&self, color: Vec4) -> Vec4 {
        color.with_w(color.w * 0.5)
    }

    /// A normal button: raised fill, accent while pressed.
    pub fn button(&self) -> ButtonStyle {
        ButtonStyle {
            idle: self.bg_light,
            hovered: self.hovered_fill(),
            pressed: self.highlight,
            disabled: self.disabled(self.bg_light),
            corner_radius: CORNER_RADIUS,
            transition_seconds: self.transition_seconds,
        }
    }

    /// The call-to-action button: filled with the accent, text drawn in `on_primary`.
    pub fn primary_button(&self) -> ButtonStyle {
        ButtonStyle {
            idle: self.primary,
            hovered: mix(self.primary, self.on_primary, HOVER_FILL_TEXT_MIX),
            pressed: self.highlight,
            disabled: self.disabled(self.primary),
            ..self.button()
        }
    }

    pub fn danger_button(&self) -> ButtonStyle {
        ButtonStyle {
            idle: self.danger,
            hovered: mix(self.danger, self.text, HOVER_FILL_TEXT_MIX),
            pressed: mix(self.danger, Vec4::new(0.0, 0.0, 0.0, 1.0), 0.25),
            disabled: self.disabled(self.danger),
            ..self.button()
        }
    }

    pub fn toggle(&self) -> ToggleStyle {
        ToggleStyle {
            idle: self.bg_light,
            hovered: self.hovered_fill(),
            checked: self.primary,
            disabled: self.disabled(self.bg_light),
            mark_color: self.on_primary,
            transition_seconds: self.transition_seconds,
        }
    }

    pub fn slider(&self) -> SliderStyle {
        SliderStyle {
            track: self.bg_light,
            fill: self.primary,
            thumb: self.text,
            thumb_hovered: self.on_primary,
            thumb_dragging: self.highlight,
            tick: self.text_muted,
            ..SliderStyle::default()
        }
    }

    /// `font_id`/`font_size` come from the typography role the input is used in.
    pub fn text_input(&self, font_id: u16, font_size: u16) -> TextInputStyle {
        TextInputStyle {
            idle: self.bg_light,
            hovered: self.hovered_fill(),
            focused: self.bg_dark,
            disabled: self.disabled(self.bg_light),
            border_idle: self.border_muted,
            border_focused: self.primary,
            text_color: self.text,
            placeholder_color: self.text_muted,
            selection_color: self.primary.with_w(0.45),
            caret_color: self.success,
            corner_radius: CORNER_RADIUS,
            font_id,
            font_size,
            ..TextInputStyle::default()
        }
    }

    pub fn dropdown(&self, arrow_font_id: u16) -> DropdownStyle {
        DropdownStyle {
            trigger: self.button(),
            list_background: self.bg_dark,
            option_hovered: self.hovered_fill(),
            corner_radius: CORNER_RADIUS,
            arrow_color: self.text_muted,
            arrow_font_id,
            ..DropdownStyle::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(opacity: u8) -> Palette {
        Palette::new(&Theme::default(), opacity)
    }

    #[test]
    fn the_inspector_theme_is_opaque_and_follows_the_palette() {
        let p = palette(40);
        let theme = p.debug_view_theme();
        assert_eq!(theme.surface.w, 1.0);
        assert_eq!(theme.surface_alt.w, 1.0);
        assert_eq!(theme.text, p.text.with_w(1.0));
        assert_eq!(theme.close, p.danger.with_w(1.0));
        assert!(theme.highlight.w < 1.0, "the row highlight lets the element underneath show");
        let mut other = Palette::new(&Theme::default(), 40);
        other.bg_dark = Vec4::new(0.9, 0.9, 0.9, 1.0);
        assert_ne!(other.debug_view_theme().surface, theme.surface);
    }

    #[test]
    fn ui_opacity_only_fades_panel_fills() {
        let p = palette(40);
        assert!((p.bg.w - 0.4).abs() < 1e-6);
        assert!((p.bg_light.w - 0.4).abs() < 1e-6);
        assert_eq!(p.text.w, 1.0);
        assert_eq!(p.primary.w, 1.0);
    }

    #[test]
    fn hover_is_visibly_different_from_idle() {
        let style = palette(100).button();
        assert_ne!(style.idle, style.hovered);
    }

    #[test]
    fn hover_keeps_the_fill_alpha() {
        let style = palette(50).button();
        assert_eq!(style.hovered.w, style.idle.w);
    }

    #[test]
    fn opacity_above_100_is_clamped() {
        assert_eq!(palette(250).bg.w, 1.0);
    }

    #[test]
    fn reduced_motion_turns_transitions_off() {
        let mut theme = Theme::default();
        theme.motion.reduced_motion = true;
        assert_eq!(Palette::new(&theme, 100).button().transition_seconds, 0.0);
    }
}
