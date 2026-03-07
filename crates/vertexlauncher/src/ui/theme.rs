use egui::{Color32, Context, CornerRadius, Stroke, Style, Visuals};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const DEFAULT_THEME_ID: &str = "matrix_oled";
const THEMES_DIR: &str = "themes";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
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

#[derive(Debug, Clone)]
pub struct ThemeCatalog {
    themes: Vec<Theme>,
    fallback: Theme,
}

impl ThemeCatalog {
    pub fn load() -> Self {
        let themes_dir = PathBuf::from(THEMES_DIR);
        ensure_themes_dir_and_defaults(&themes_dir);

        let fallback = Theme::matrix_oled();
        let mut themes = load_themes_from_dir(&themes_dir);
        let mut seen = HashSet::new();
        themes.retain(|theme| seen.insert(theme.id.clone()));

        if !themes.iter().any(|theme| theme.id == DEFAULT_THEME_ID) {
            themes.push(fallback.clone());
        }

        themes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Self { themes, fallback }
    }

    pub fn themes(&self) -> &[Theme] {
        &self.themes
    }

    pub fn contains(&self, id: &str) -> bool {
        self.themes.iter().any(|theme| theme.id == id)
    }

    pub fn default_theme_id(&self) -> &str {
        DEFAULT_THEME_ID
    }

    pub fn resolve(&self, id: &str) -> &Theme {
        self.themes
            .iter()
            .find(|theme| theme.id == id)
            .or_else(|| {
                self.themes
                    .iter()
                    .find(|theme| theme.id == DEFAULT_THEME_ID)
            })
            .or_else(|| self.themes.first())
            .unwrap_or(&self.fallback)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::matrix_oled()
    }
}

impl Theme {
    pub fn apply(&self, ctx: &Context, blur_enabled: bool) {
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

        let alpha_profile = if blur_enabled {
            ThemeAlphaProfile::TRANSPARENT
        } else {
            ThemeAlphaProfile::OPAQUE
        };

        visuals.window_fill = with_alpha(bg_dark, alpha_profile.window_fill);
        visuals.panel_fill = with_alpha(bg, alpha_profile.panel_fill);
        visuals.faint_bg_color = with_alpha(bg_light, alpha_profile.faint_bg);
        visuals.extreme_bg_color = with_alpha(bg_dark, alpha_profile.extreme_bg);
        visuals.code_bg_color = with_alpha(bg_dark, alpha_profile.code_bg);
        visuals.override_text_color = Some(text);
        visuals.weak_text_color = Some(text_muted);
        visuals.window_stroke = Stroke::new(1.0, border_muted);
        visuals.widgets.noninteractive.bg_fill = with_alpha(bg, alpha_profile.noninteractive_bg);
        visuals.widgets.noninteractive.weak_bg_fill =
            with_alpha(bg, alpha_profile.noninteractive_weak_bg);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text_muted);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border_muted);

        visuals.widgets.inactive.bg_fill = with_alpha(bg_light, alpha_profile.inactive_bg);
        visuals.widgets.inactive.weak_bg_fill =
            with_alpha(bg_light, alpha_profile.inactive_weak_bg);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border_muted);

        visuals.widgets.hovered.bg_fill = with_alpha(bg_light, alpha_profile.hovered_bg);
        visuals.widgets.hovered.weak_bg_fill = with_alpha(bg_light, alpha_profile.hovered_weak_bg);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, text);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, border);

        visuals.widgets.active.bg_fill = with_alpha(highlight, alpha_profile.active_bg);
        visuals.widgets.active.weak_bg_fill = with_alpha(highlight, alpha_profile.active_weak_bg);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, text);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, border);

        visuals.widgets.open.bg_fill = with_alpha(bg_light, alpha_profile.open_bg);
        visuals.widgets.open.weak_bg_fill = with_alpha(bg_light, alpha_profile.open_weak_bg);
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

    fn matrix_oled() -> Self {
        Self {
            id: "matrix_oled".to_owned(),
            name: "Matrix OLED".to_owned(),
            description: "Original dark matrix-inspired theme.".to_owned(),
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

    fn default_light() -> Self {
        Self {
            id: "default_light".to_owned(),
            name: "Default Light".to_owned(),
            description: "Light theme from RustServerController.".to_owned(),
            bg_dark: Oklch {
                l: 0.92,
                c: 0.055,
                h: 264.0,
            },
            bg: Oklch {
                l: 0.96,
                c: 0.055,
                h: 264.0,
            },
            bg_light: Oklch {
                l: 1.00,
                c: 0.055,
                h: 264.0,
            },
            text: Oklch {
                l: 0.15,
                c: 0.11,
                h: 264.0,
            },
            text_muted: Oklch {
                l: 0.40,
                c: 0.11,
                h: 264.0,
            },
            highlight: Oklch {
                l: 1.00,
                c: 0.11,
                h: 264.0,
            },
            border: Oklch {
                l: 0.60,
                c: 0.11,
                h: 264.0,
            },
            border_muted: Oklch {
                l: 0.70,
                c: 0.11,
                h: 264.0,
            },
            primary: Oklch {
                l: 0.40,
                c: 0.20,
                h: 264.0,
            },
            secondary: Oklch {
                l: 0.40,
                c: 0.20,
                h: 84.0,
            },
            danger: Oklch {
                l: 0.50,
                c: 0.20,
                h: 30.0,
            },
            warning: Oklch {
                l: 0.50,
                c: 0.20,
                h: 100.0,
            },
            success: Oklch {
                l: 0.50,
                c: 0.20,
                h: 160.0,
            },
            info: Oklch {
                l: 0.50,
                c: 0.20,
                h: 260.0,
            },
        }
    }

    fn high_contrast() -> Self {
        Self {
            id: "high_contrast".to_owned(),
            name: "High Contrast".to_owned(),
            description: "Accessibility-focused high-contrast theme from RustServerController."
                .to_owned(),
            bg_dark: Oklch {
                l: 0.05,
                c: 0.01,
                h: 256.0,
            },
            bg: Oklch {
                l: 0.08,
                c: 0.01,
                h: 256.0,
            },
            bg_light: Oklch {
                l: 0.12,
                c: 0.01,
                h: 256.0,
            },
            text: Oklch {
                l: 0.99,
                c: 0.03,
                h: 256.0,
            },
            text_muted: Oklch {
                l: 0.90,
                c: 0.03,
                h: 256.0,
            },
            highlight: Oklch {
                l: 0.70,
                c: 0.03,
                h: 256.0,
            },
            border: Oklch {
                l: 0.60,
                c: 0.03,
                h: 256.0,
            },
            border_muted: Oklch {
                l: 0.50,
                c: 0.03,
                h: 256.0,
            },
            primary: Oklch {
                l: 0.85,
                c: 0.25,
                h: 256.0,
            },
            secondary: Oklch {
                l: 0.85,
                c: 0.25,
                h: 76.0,
            },
            danger: Oklch {
                l: 0.80,
                c: 0.25,
                h: 30.0,
            },
            warning: Oklch {
                l: 0.80,
                c: 0.25,
                h: 100.0,
            },
            success: Oklch {
                l: 0.80,
                c: 0.25,
                h: 160.0,
            },
            info: Oklch {
                l: 0.80,
                c: 0.25,
                h: 260.0,
            },
        }
    }

    fn purple_dream() -> Self {
        Self {
            id: "purple_dream".to_owned(),
            name: "Purple Dream".to_owned(),
            description: "Purple theme from RustServerController.".to_owned(),
            bg_dark: Oklch {
                l: 0.10,
                c: 0.05,
                h: 290.0,
            },
            bg: Oklch {
                l: 0.15,
                c: 0.05,
                h: 290.0,
            },
            bg_light: Oklch {
                l: 0.20,
                c: 0.05,
                h: 290.0,
            },
            text: Oklch {
                l: 0.96,
                c: 0.02,
                h: 290.0,
            },
            text_muted: Oklch {
                l: 0.76,
                c: 0.02,
                h: 290.0,
            },
            highlight: Oklch {
                l: 0.70,
                c: 0.20,
                h: 290.0,
            },
            border: Oklch {
                l: 0.40,
                c: 0.02,
                h: 256.0,
            },
            border_muted: Oklch {
                l: 0.30,
                c: 0.02,
                h: 256.0,
            },
            primary: Oklch {
                l: 0.70,
                c: 0.25,
                h: 290.0,
            },
            secondary: Oklch {
                l: 0.70,
                c: 0.20,
                h: 250.0,
            },
            danger: Oklch {
                l: 0.70,
                c: 0.25,
                h: 30.0,
            },
            warning: Oklch {
                l: 0.70,
                c: 0.20,
                h: 80.0,
            },
            success: Oklch {
                l: 0.70,
                c: 0.20,
                h: 140.0,
            },
            info: Oklch {
                l: 0.70,
                c: 0.20,
                h: 210.0,
            },
        }
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

fn ensure_themes_dir_and_defaults(dir: &Path) {
    tracing::debug!(target: "vertexlauncher/io", op = "create_dir_all", path = %dir.display(), context = "ensure themes dir");
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }

    for theme in [
        Theme::matrix_oled(),
        Theme::default_light(),
        Theme::high_contrast(),
        Theme::purple_dream(),
    ] {
        let path = dir.join(format!("{}.toml", theme.id));
        if path.exists() {
            continue;
        }

        if let Ok(contents) = toml::to_string_pretty(&theme) {
            tracing::debug!(target: "vertexlauncher/io", op = "write", path = %path.display(), context = "write default theme");
            let _ = std::fs::write(path, contents);
        }
    }
}

fn load_themes_from_dir(dir: &Path) -> Vec<Theme> {
    tracing::debug!(target: "vertexlauncher/io", op = "read_dir", path = %dir.display(), context = "load themes");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut themes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if ext != "toml" {
            continue;
        }

        tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display(), context = "load theme file");
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut theme) = toml::from_str::<Theme>(&contents) else {
            continue;
        };

        let file_stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(DEFAULT_THEME_ID);
        if theme.id.trim().is_empty() {
            theme.id = file_stem.to_owned();
        }
        if theme.name.trim().is_empty() {
            theme.name = file_stem.replace('_', " ");
        }

        themes.push(theme);
    }

    themes
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

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

#[derive(Clone, Copy)]
struct ThemeAlphaProfile {
    window_fill: u8,
    panel_fill: u8,
    faint_bg: u8,
    extreme_bg: u8,
    code_bg: u8,
    noninteractive_bg: u8,
    noninteractive_weak_bg: u8,
    inactive_bg: u8,
    inactive_weak_bg: u8,
    hovered_bg: u8,
    hovered_weak_bg: u8,
    active_bg: u8,
    active_weak_bg: u8,
    open_bg: u8,
    open_weak_bg: u8,
}

impl ThemeAlphaProfile {
    const OPAQUE: Self = Self {
        window_fill: 255,
        panel_fill: 255,
        faint_bg: 255,
        extreme_bg: 255,
        code_bg: 255,
        noninteractive_bg: 255,
        noninteractive_weak_bg: 255,
        inactive_bg: 255,
        inactive_weak_bg: 255,
        hovered_bg: 255,
        hovered_weak_bg: 255,
        active_bg: 255,
        active_weak_bg: 255,
        open_bg: 255,
        open_weak_bg: 255,
    };

    const TRANSPARENT: Self = Self {
        window_fill: 95,
        panel_fill: 72,
        faint_bg: 58,
        extreme_bg: 118,
        code_bg: 130,
        noninteractive_bg: 62,
        noninteractive_weak_bg: 50,
        inactive_bg: 60,
        inactive_weak_bg: 48,
        hovered_bg: 92,
        hovered_weak_bg: 74,
        active_bg: 132,
        active_weak_bg: 108,
        open_bg: 76,
        open_weak_bg: 62,
    };
}
