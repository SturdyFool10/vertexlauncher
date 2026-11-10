use vertex_i18n::src;
use serde::{Deserialize, Serialize};

/// NSVisualEffectView material selection for macOS blur/backdrop effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MacosVisualEffectMaterial {
    AppearanceBased,
    Light,
    Dark,
    Titlebar,
    Selection,
    Menu,
    Popover,
    Sidebar,
    HeaderView,
    Sheet,
    WindowBackground,
    HudWindow,
    FullScreenUi,
    Tooltip,
    ContentBackground,
    UnderWindowBackground,
    UnderPageBackground,
}

impl MacosVisualEffectMaterial {
    pub const ALL: [MacosVisualEffectMaterial; 17] = [
        MacosVisualEffectMaterial::AppearanceBased,
        MacosVisualEffectMaterial::Light,
        MacosVisualEffectMaterial::Dark,
        MacosVisualEffectMaterial::Titlebar,
        MacosVisualEffectMaterial::Selection,
        MacosVisualEffectMaterial::Menu,
        MacosVisualEffectMaterial::Popover,
        MacosVisualEffectMaterial::Sidebar,
        MacosVisualEffectMaterial::HeaderView,
        MacosVisualEffectMaterial::Sheet,
        MacosVisualEffectMaterial::WindowBackground,
        MacosVisualEffectMaterial::HudWindow,
        MacosVisualEffectMaterial::FullScreenUi,
        MacosVisualEffectMaterial::Tooltip,
        MacosVisualEffectMaterial::ContentBackground,
        MacosVisualEffectMaterial::UnderWindowBackground,
        MacosVisualEffectMaterial::UnderPageBackground,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            MacosVisualEffectMaterial::AppearanceBased => src!("Appearance Based"),
            MacosVisualEffectMaterial::Light => src!("Light"),
            MacosVisualEffectMaterial::Dark => src!("Dark"),
            MacosVisualEffectMaterial::Titlebar => src!("Titlebar"),
            MacosVisualEffectMaterial::Selection => src!("Selection"),
            MacosVisualEffectMaterial::Menu => src!("Menu"),
            MacosVisualEffectMaterial::Popover => src!("Popover"),
            MacosVisualEffectMaterial::Sidebar => src!("Sidebar"),
            MacosVisualEffectMaterial::HeaderView => src!("Header View"),
            MacosVisualEffectMaterial::Sheet => src!("Sheet"),
            MacosVisualEffectMaterial::WindowBackground => src!("Window Background"),
            MacosVisualEffectMaterial::HudWindow => src!("HUD Window"),
            MacosVisualEffectMaterial::FullScreenUi => src!("Full Screen UI"),
            MacosVisualEffectMaterial::Tooltip => src!("Tooltip"),
            MacosVisualEffectMaterial::ContentBackground => src!("Content Background"),
            MacosVisualEffectMaterial::UnderWindowBackground => src!("Under Window Background"),
            MacosVisualEffectMaterial::UnderPageBackground => src!("Under Page Background"),
        }
    }
}
