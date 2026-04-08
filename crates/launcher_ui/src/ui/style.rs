use egui::{Color32, Ui, Vec2};
use textui::TextFundamentals;
use textui_egui::prelude::*;

pub const SPACE_XS: f32 = 4.0;
pub const SPACE_SM: f32 = 6.0;
pub const SPACE_MD: f32 = 8.0;
pub const SPACE_LG: f32 = 10.0;
pub const SPACE_XL: f32 = 12.0;

pub const CONTROL_HEIGHT: f32 = 30.0;
pub const CONTROL_HEIGHT_LG: f32 = 34.0;
pub const CORNER_RADIUS_SM: u8 = 8;
pub const CORNER_RADIUS_MD: u8 = 10;

pub fn page_heading(ui: &Ui) -> LabelOptions {
    LabelOptions {
        font_size: 34.0,
        line_height: 42.0,
        weight: 700,
        color: ui.visuals().text_color(),
        wrap: false,
        fundamentals: TextFundamentals {
            letter_spacing_points: 0.5,
            ..TextFundamentals::default()
        },
        ..LabelOptions::default()
    }
}

pub fn section_heading(ui: &Ui) -> LabelOptions {
    LabelOptions {
        font_size: 25.0,
        line_height: 33.0,
        weight: 600,
        color: ui.visuals().text_color(),
        wrap: false,
        fundamentals: TextFundamentals {
            letter_spacing_points: 0.3,
            ..TextFundamentals::default()
        },
        ..LabelOptions::default()
    }
}

pub fn heading(ui: &Ui, font_size: f32, line_height: f32) -> LabelOptions {
    heading_color(ui, font_size, line_height, ui.visuals().text_color())
}

pub fn heading_color(_ui: &Ui, font_size: f32, line_height: f32, color: Color32) -> LabelOptions {
    LabelOptions {
        font_size,
        line_height,
        weight: 700,
        color,
        wrap: false,
        ..LabelOptions::default()
    }
}

pub fn body(ui: &Ui) -> LabelOptions {
    LabelOptions {
        color: ui.visuals().text_color(),
        wrap: true,
        ..LabelOptions::default()
    }
}

pub fn caption(ui: &Ui) -> LabelOptions {
    LabelOptions {
        font_size: 13.0,
        line_height: 18.0,
        weight: 400,
        color: ui.visuals().weak_text_color(),
        wrap: false,
        fundamentals: TextFundamentals {
            letter_spacing_points: -0.15,
            ..TextFundamentals::default()
        },
        ..LabelOptions::default()
    }
}

pub fn badge_label(ui: &Ui) -> LabelOptions {
    LabelOptions {
        font_size: 11.0,
        line_height: 14.0,
        weight: 600,
        color: ui.visuals().text_color(),
        wrap: false,
        fundamentals: TextFundamentals {
            letter_spacing_points: 0.6,
            case_sensitive_forms: true,
            ..TextFundamentals::default()
        },
        ..LabelOptions::default()
    }
}

/// Subtitle — used for modal/dialog titles and mid-level headings.
/// 22pt, weight 700, +0.2 tracking.
pub fn subtitle(ui: &Ui) -> LabelOptions {
    LabelOptions {
        font_size: 22.0,
        line_height: 28.0,
        weight: 700,
        color: ui.visuals().text_color(),
        wrap: false,
        fundamentals: TextFundamentals {
            letter_spacing_points: 0.2,
            ..TextFundamentals::default()
        },
        ..LabelOptions::default()
    }
}

/// Modal/dialog title — 26pt, weight 700, +0.3 tracking.
pub fn modal_title(ui: &Ui) -> LabelOptions {
    LabelOptions {
        font_size: 26.0,
        line_height: 34.0,
        weight: 700,
        color: ui.visuals().text_color(),
        wrap: false,
        fundamentals: TextFundamentals {
            letter_spacing_points: 0.3,
            ..TextFundamentals::default()
        },
        ..LabelOptions::default()
    }
}

/// Small heading / stat label — 16pt, weight 600.
pub fn stat_label(ui: &Ui) -> LabelOptions {
    LabelOptions {
        font_size: 16.0,
        line_height: 22.0,
        weight: 600,
        color: ui.visuals().text_color(),
        wrap: false,
        ..LabelOptions::default()
    }
}

/// Error text — body-sized, error foreground color.
pub fn error_text(ui: &Ui) -> LabelOptions {
    LabelOptions {
        color: ui.visuals().error_fg_color,
        wrap: true,
        ..LabelOptions::default()
    }
}

/// Warning text — body-sized, warn foreground color.
pub fn warning_text(ui: &Ui) -> LabelOptions {
    LabelOptions {
        color: ui.visuals().warn_fg_color,
        wrap: true,
        ..LabelOptions::default()
    }
}

/// Body text in a specific color (for bold-body, tinted body, etc.).
pub fn body_strong(ui: &Ui) -> LabelOptions {
    LabelOptions {
        color: ui.visuals().text_color(),
        weight: 600,
        wrap: true,
        ..LabelOptions::default()
    }
}

pub fn muted(ui: &Ui) -> LabelOptions {
    LabelOptions {
        color: ui.visuals().weak_text_color(),
        wrap: true,
        ..LabelOptions::default()
    }
}

pub fn muted_single_line(ui: &Ui) -> LabelOptions {
    let mut style = muted(ui);
    style.wrap = false;
    style
}

pub fn neutral_button(ui: &Ui) -> ButtonOptions {
    ButtonOptions {
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..ButtonOptions::default()
    }
}

pub fn neutral_button_with_min_size(ui: &Ui, min_size: Vec2) -> ButtonOptions {
    ButtonOptions {
        min_size,
        ..neutral_button(ui)
    }
}
