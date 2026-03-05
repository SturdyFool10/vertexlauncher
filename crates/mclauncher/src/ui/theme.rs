use egui::{Color32, Context, CornerRadius, Stroke, Style, Visuals};

#[derive(Debug, Clone, Copy)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg_dark: Oklch,
    pub bg: Oklch,
    pub bg_light: Oklch,
    pub text: Oklch,
    pub text_muted: Oklch,
    pub highlight: Oklch,
    pub border: Oklch,
    pub border_muted: Oklch,
    pub primary: Oklch,
    pub secondary: Oklch,
    pub danger: Oklch,
    pub warning: Oklch,
    pub success: Oklch,
    pub info: Oklch,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg_dark: Oklch {
                l: 0.10,
                c: 0.0,
                h: 316.0,
            },
            bg: Oklch {
                l: 0.15,
                c: 0.0,
                h: 316.0,
            },
            bg_light: Oklch {
                l: 0.20,
                c: 0.0,
                h: 316.0,
            },
            text: Oklch {
                l: 0.96,
                c: 0.0,
                h: 316.0,
            },
            text_muted: Oklch {
                l: 0.76,
                c: 0.0,
                h: 316.0,
            },
            highlight: Oklch {
                l: 0.50,
                c: 0.0,
                h: 316.0,
            },
            border: Oklch {
                l: 0.40,
                c: 0.0,
                h: 316.0,
            },
            border_muted: Oklch {
                l: 0.30,
                c: 0.0,
                h: 316.0,
            },
            primary: Oklch {
                l: 0.76,
                c: 0.2,
                h: 150.0,
            },
            secondary: Oklch {
                l: 0.76,
                c: 0.2,
                h: 330.0,
            },
            danger: Oklch {
                l: 0.70,
                c: 0.2,
                h: 30.0,
            },
            warning: Oklch {
                l: 0.70,
                c: 0.2,
                h: 100.0,
            },
            success: Oklch {
                l: 0.70,
                c: 0.2,
                h: 160.0,
            },
            info: Oklch {
                l: 0.70,
                c: 0.2,
                h: 260.0,
            },
        }
    }
}

impl Theme {
    pub fn apply(&self, ctx: &Context) {
        let mut style: Style = (*ctx.style()).clone();
        let mut visuals = Visuals::dark();

        let bg_dark = self.bg_dark.to_color32();
        let bg = self.bg.to_color32();
        let bg_light = self.bg_light.to_color32();
        let text = self.text.to_color32();
        let text_muted = self.text_muted.to_color32();
        let highlight = self.highlight.to_color32();
        let border = self.border.to_color32();
        let border_muted = self.border_muted.to_color32();

        visuals.window_fill = bg_dark;
        visuals.panel_fill = bg;
        visuals.faint_bg_color = bg_light;
        visuals.extreme_bg_color = bg_dark;
        visuals.code_bg_color = bg_dark;
        visuals.override_text_color = Some(text);
        visuals.weak_text_color = Some(text_muted);
        visuals.window_stroke = Stroke::new(1.0, border_muted);
        visuals.widgets.noninteractive.bg_fill = bg;
        visuals.widgets.noninteractive.weak_bg_fill = bg;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text_muted);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border_muted);

        visuals.widgets.inactive.bg_fill = bg_light;
        visuals.widgets.inactive.weak_bg_fill = bg_light;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border_muted);

        visuals.widgets.hovered.bg_fill = bg_light;
        visuals.widgets.hovered.weak_bg_fill = bg_light;
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, text);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, border);

        visuals.widgets.active.bg_fill = highlight;
        visuals.widgets.active.weak_bg_fill = highlight;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, text);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, border);

        visuals.widgets.open.bg_fill = bg_light;
        visuals.widgets.open.weak_bg_fill = bg_light;
        visuals.widgets.open.fg_stroke = Stroke::new(1.0, text);
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, border);

        visuals.selection.bg_fill = self.primary.to_color32();
        visuals.selection.stroke = Stroke::new(1.0, self.secondary.to_color32());
        visuals.hyperlink_color = self.info.to_color32();
        visuals.warn_fg_color = self.warning.to_color32();
        visuals.error_fg_color = self.danger.to_color32();
        visuals.text_cursor.stroke = Stroke::new(2.0, self.success.to_color32());

        let rounding = CornerRadius::same(14);
        visuals.window_corner_radius = rounding;
        visuals.menu_corner_radius = rounding;
        visuals.widgets.noninteractive.corner_radius = rounding;
        visuals.widgets.inactive.corner_radius = rounding;
        visuals.widgets.hovered.corner_radius = rounding;
        visuals.widgets.active.corner_radius = rounding;
        visuals.widgets.open.corner_radius = rounding;

        style.visuals = visuals;
        ctx.set_style(style);
    }
}

impl Oklch {
    pub fn to_color32(self) -> Color32 {
        let h_rad = self.h.to_radians();
        let a = self.c * h_rad.cos();
        let b = self.c * h_rad.sin();

        let l_ = self.l + 0.396_337_78 * a + 0.215_803_76 * b;
        let m_ = self.l - 0.105_561_346 * a - 0.063_854_17 * b;
        let s_ = self.l - 0.089_484_18 * a - 1.291_485_5 * b;

        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;

        let r_lin = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
        let g_lin = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s;
        let b_lin = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;

        Color32::from_rgb(
            linear_to_srgb_u8(r_lin),
            linear_to_srgb_u8(g_lin),
            linear_to_srgb_u8(b_lin),
        )
    }
}

fn linear_to_srgb_u8(value: f32) -> u8 {
    let clamped = value.clamp(0.0, 1.0);
    let srgb = if clamped <= 0.0031308 {
        12.92 * clamped
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round().clamp(0.0, 255.0) as u8
}
