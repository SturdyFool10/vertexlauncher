use super::*;

pub struct CallbackPageColors {
    pub bg: String,
    pub panel: String,
    pub border: String,
    pub text: String,
    pub muted: String,
    pub success: String,
    pub danger: String,
    pub primary_tint: String,
}

impl CallbackPageColors {
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            bg: fmt_oklch(theme.bg_dark),
            panel: fmt_oklch(theme.bg),
            border: fmt_oklch(theme.border_muted),
            text: fmt_oklch(theme.text),
            muted: fmt_oklch(theme.text_muted),
            success: fmt_oklch(theme.success),
            danger: fmt_oklch(theme.danger),
            primary_tint: fmt_oklch_alpha(theme.primary, 0.09),
        }
    }
}
