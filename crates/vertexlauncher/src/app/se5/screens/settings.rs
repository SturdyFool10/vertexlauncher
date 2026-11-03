//! Settings, in the same sections, labels and help text as the egui screen. Every control edits `Config` directly;
//! the app saves the change and the engine appliers apply it on the next frame. Nothing needs a restart.

use std::path::Path;

use config::{
    Config, FloatSettingId, GraphicsAdapterPreferenceType, GraphicsAdapterProfile,
    GraphicsApiPreference, IntSettingId, JavaRuntimeVersion, NotificationEasing, RoleOverride,
    SkinPreviewAaMode, SkinPreviewTexelAaMode, SvgAaMode, TextRenderingPath, TextSettingId,
    ToggleSettingId, WindowTransparency,
};
use launcher_ui::java_actions::{self, JavaActionKind};
use launcher_ui::notification;
use vertex_shell::settings_apply::adapter_hash;
use vertex_shell::sturdy::ui::{Sizing, TextStyle, TextWrap, Ui};

use super::super::super::better_blur::BetterBlurDx;
use super::super::kit::{self, ButtonKind, Kit, TextRole};
use super::super::{AppState, Dialog};

const INSECURE_STORAGE_WARNING: &str = "Warning: this keeps your sign-in tokens in a plain-text file instead of your system's secure keyring. Doing so can improve compatibility with the Steam Deck, but it also means you need to be extra careful with your security, because it can make it really easy for your login details to be compromised: anyone or anything that can read that file can sign in as you.";

/// Per-screen caches that are expensive to rebuild every frame.
#[derive(Default)]
pub struct SettingsView {
    /// Engine adapters as (display name, persisted hash).
    adapters: Option<Vec<(String, u64)>>,
    /// Result line of the last "Purge metadata cache" run, and whether one is running.
    purge_status: Option<String>,
    purge_rx: Option<std::sync::mpsc::Receiver<String>>,
    /// The Windows blur tint colour as typed, so a half-typed value is not reset under the cursor.
    tint_text: Option<String>,
    /// The graphics settings the System info was last read for; a change re-reads which API and GPU are active.
    graphics_signature: Option<(GraphicsApiPreference, u64, u8)>,
    /// A device switch lands a frame or two after the setting changes, so the info is re-read until this passes.
    graphics_refresh_until: Option<std::time::Instant>,
}

pub fn render(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let signature = (
        state.config.graphics_api_preference(),
        state.config.graphics_adapter_explicit_hash().unwrap_or(0),
        state.config.graphics_adapter_profile() as u8 + 16 * state.config.graphics_adapter_preference_type() as u8,
    );
    if state.settings.graphics_signature != Some(signature) {
        // The first time through this only sets the baseline; later it means the user changed a graphics setting.
        if state.settings.graphics_signature.is_some() {
            state.settings.graphics_refresh_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
        }
        state.settings.graphics_signature = Some(signature);
    }
    let refreshing = state.settings.graphics_refresh_until.is_some_and(|until| std::time::Instant::now() < until);
    if !refreshing {
        state.settings.graphics_refresh_until = None;
    }
    if state.settings.adapters.is_none() || refreshing {
        let adapters = ui.engine().available_adapters();
        // The GPU the engine is running on, for the System section.
        if let Some((adapter, offer)) = adapters.iter().find_map(|adapter| {
            adapter
                .backends
                .iter()
                .find(|offer| offer.active)
                .map(|offer| (adapter, offer))
        }) {
            super::super::super::app_metadata::record_graphics_adapter(
                &adapter.name,
                &format!("{:?}", offer.backend),
                &adapter.vendor,
                &offer.driver_version,
            );
        }
        if state.settings.adapters.is_none() {
            state.settings.adapters = Some(
                adapters
                    .into_iter()
                    .map(|adapter| (adapter.name, adapter_hash(&adapter.stable_id)))
                    .collect(),
            );
        }
        if refreshing {
            kit.keep_awake_for(std::time::Duration::from_millis(100));
        }
    }
    poll_purge(state);
    let _scroll = kit.scroll(ui, "settings::scroll", kit::SPACE_LG, kit::SPACE_XL);
    kit.text(ui, TextRole::Title, "Settings");

    section(
        ui,
        kit,
        "appearance",
        "Appearance",
        "Theme and launcher chrome settings.",
        |ui, kit| appearance(ui, kit, state),
    );
    section(
        ui,
        kit,
        "privacy",
        "Privacy",
        "On-stream safety and account security settings.",
        |ui, kit| privacy(ui, kit, state),
    );
    section(
        ui,
        kit,
        "notifications",
        "Notifications",
        "How notification popups expire and fade away.",
        |ui, kit| {
            toggles(
                ui,
                kit,
                &mut state.config,
                &[ToggleSettingId::NotificationExpiryBarsEmptyLeft],
            );
            floats(
                ui,
                kit,
                &mut state.config,
                &[FloatSettingId::NotificationFadeOutSeconds],
            );
            if let Some(easing) = kit.choice_setting(
            ui,
            "settings::notification-easing",
            "Notification Fade Out Easing",
            Some("The curve of the fade-out animation. Linear fades at a constant rate; Cubic and Exponential ease in, out, or both, and the Exponential curves are the most abrupt. Changes apply immediately. Default: Cubic In/Out."),
            state.config.notification_easing(),
            &NotificationEasing::ALL,
            |easing| easing.label().to_owned(),
        ) {
            state.config.set_notification_easing(easing);
        }
            if kit.button(
                ui,
                "settings::test-notification",
                "Test notification",
                ButtonKind::Neutral,
                true,
            ) {
                notification::emit(
                    notification::Severity::Info,
                    "Settings",
                    "This is a test notification.",
                );
            }
        },
    );
    section(
        ui,
        kit,
        "text",
        "Text",
        "Font, shaping, and glyph rendering settings that affect launcher text.",
        |ui, kit| text(ui, kit, state),
    );
    section(
        ui,
        kit,
        "discord",
        "Discord Rich Presence",
        "Discord integration and launcher-owned activity reporting.",
        |ui, kit| {
            toggles(
                ui,
                kit,
                &mut state.config,
                &[ToggleSettingId::DiscordRichPresenceEnabled],
            );
        },
    );
    section(
        ui,
        kit,
        "sync",
        "Instance Sync",
        "Keep multiplayer servers, command history, and creative hotbars in step across your instances. Sync runs before an instance launches and after it closes; right-click a server on Home to choose which instances it goes to.",
        |ui, kit| {
            toggles(
                ui,
                kit,
                &mut state.config,
                &[
                    ToggleSettingId::SyncServersEnabled,
                    ToggleSettingId::SyncServersToAllInstancesByDefault,
                    ToggleSettingId::SyncCommandHistoryEnabled,
                    ToggleSettingId::SyncHotbarsEnabled,
                ],
            );
        },
    );
    section(
        ui,
        kit,
        "graphics",
        "Graphics & Performance",
        "GPU, skin preview rendering, frame pacing, and download throughput behavior.",
        |ui, kit| graphics(ui, kit, state),
    );
    section(
        ui,
        kit,
        "minecraft",
        "Minecraft",
        "Version catalog behavior and content provider access.",
        |ui, kit| {
            kit.text(ui, TextRole::Heading, "Minecraft Version Groups");
            kit.text(ui, TextRole::Muted, "Controls which non-default Minecraft version categories appear in version pickers.");
            toggles(
                ui,
                kit,
                &mut state.config,
                &[
                    ToggleSettingId::SnapshotsAndBetasEnabled,
                    ToggleSettingId::AlphaVersionsEnabled,
                    ToggleSettingId::ExperimentalVersionsEnabled,
                ],
            );
            let mut key = state.config.curseforge_api_key().to_owned();
            if kit.text_setting(
            ui,
            "settings::curseforge-key",
            "CurseForge API Key",
            Some("Required for CurseForge modpack zip imports. Overrides VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY and enables CurseForge browsing/download metadata."),
            &mut key,
            "",
            false,
        ) {
            *state.config.curseforge_api_key_mut() = key;
        }
        },
    );
    section(
        ui,
        kit,
        "launching",
        "Launching",
        "What the launcher does when you start a game.",
        |ui, kit| {
            toggles(
                ui,
                kit,
                &mut state.config,
                &[
                    ToggleSettingId::FollowConsoleOnLaunch,
                    ToggleSettingId::SteamDeckHandoff,
                ],
            );
        },
    );
    section(
        ui,
        kit,
        "java",
        "Java",
        "Shared Java behavior for launches that use managed runtime selection.",
        |ui, kit| {
            toggles(
                ui,
                kit,
                &mut state.config,
                &[ToggleSettingId::ForceJava21Minimum],
            );
            java_paths(ui, kit, state);
        },
    );
    section(
        ui,
        kit,
        "defaults",
        "Instance Defaults",
        "Used when creating new instances. You can still override values per instance.",
        |ui, kit| instance_defaults(ui, kit, state),
    );
    platform_section(ui, kit, state);
    info_section(ui, kit);
}

fn section(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    heading: &str,
    description: &str,
    body: impl FnOnce(&mut Ui<'_>, &mut Kit),
) {
    let _card = kit.card(ui, &format!("settings::section::{id}"), kit::SPACE_MD);
    kit.section_header(ui, heading, description);
    body(ui, kit);
}

fn toggles(ui: &mut Ui<'_>, kit: &mut Kit, config: &mut Config, ids: &[ToggleSettingId]) {
    let mut frame_limit = f64::from(config.frame_limit_fps());
    let mut limit_changed = false;
    for wanted in ids {
        config.for_each_toggle_mut(|spec, value| {
            if spec.id != *wanted {
                return;
            }
            kit.toggle_setting(
                ui,
                &format!("settings::toggle::{:?}", spec.id),
                spec.label,
                spec.info_tooltip,
                value,
            );
            // The frame limiter's rate shows right under its switch while it is on.
            if spec.id == ToggleSettingId::FrameLimiterEnabled && *value {
                let limit = IntSettingId::FrameLimitFps.spec();
                if let Some(next) = kit.slider_setting(
                    ui,
                    &format!("settings::int::{:?}", limit.id),
                    limit.label,
                    limit.info_tooltip,
                    frame_limit,
                    f64::from(limit.min),
                    f64::from(limit.max),
                    f64::from(limit.step),
                    |value| format!("{value:.0}"),
                ) {
                    frame_limit = next;
                    limit_changed = true;
                }
            }
        });
    }
    if limit_changed {
        config.set_frame_limit_fps(frame_limit.round() as i32);
    }
}

fn floats(ui: &mut Ui<'_>, kit: &mut Kit, config: &mut Config, ids: &[FloatSettingId]) {
    for wanted in ids {
        config.for_each_float_mut(|spec, value| {
            if spec.id != *wanted {
                return;
            }
            if let Some(next) = kit.slider_setting(
                ui,
                &format!("settings::float::{:?}", spec.id),
                spec.label,
                spec.info_tooltip,
                f64::from(*value),
                f64::from(spec.min),
                f64::from(spec.max),
                f64::from(spec.step),
                |value| format!("{value:.2}"),
            ) {
                *value = next as f32;
            }
        });
    }
}

fn ints(ui: &mut Ui<'_>, kit: &mut Kit, config: &mut Config, ids: &[IntSettingId]) {
    for wanted in ids {
        config.for_each_int_mut(|spec, value| {
            if spec.id != *wanted {
                return;
            }
            if let Some(next) = kit.slider_setting(
                ui,
                &format!("settings::int::{:?}", spec.id),
                spec.label,
                spec.info_tooltip,
                f64::from(*value),
                f64::from(spec.min),
                f64::from(spec.max),
                f64::from(spec.step),
                |value| format!("{value:.0}"),
            ) {
                *value = next.round() as i32;
            }
        });
    }
}

fn texts(ui: &mut Ui<'_>, kit: &mut Kit, config: &mut Config, ids: &[TextSettingId]) {
    for wanted in ids {
        config.for_each_text_mut(|spec, value| {
            if spec.id == *wanted {
                kit.text_setting(
                    ui,
                    &format!("settings::text::{:?}", spec.id),
                    spec.label,
                    spec.info_tooltip,
                    value,
                    "",
                    false,
                );
            }
        });
    }
}

// -- Appearance ---------------------------------------------------------------------------------------------------

fn appearance(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let labels = state.theme_catalog.theme_labels().to_vec();
    let current = state
        .theme_catalog
        .themes()
        .iter()
        .position(|theme| theme.id == state.config.theme_id())
        .unwrap_or(0);
    let tooltip = format!(
        "Themes are loaded from {}. Add or edit a theme file there and it appears here without restarting.",
        app_paths::themes_dir().display()
    );
    let mut chosen = None;
    kit.setting_row(
        ui,
        "settings::theme-row",
        "Theme",
        Some(&tooltip),
        |kit, ui| {
            chosen = kit.dropdown(ui, "settings::theme", current, &labels, 280.0);
        },
    );
    if let Some(index) = chosen
        && let Some(theme) = state.theme_catalog.themes().get(index)
    {
        state.config.set_theme_id(theme.id.clone());
    }
    if kit.button(
        ui,
        "settings::theme-export",
        "Export theme as JSON",
        ButtonKind::Neutral,
        true,
    ) {
        export_theme(&state.theme);
    }
    kit.tooltip(
        ui,
        "settings::theme-export",
        "Saves the selected theme as a JSON file in the themes folder, to use as a starting point for your own theme.",
    );

    let spec = config::DropdownSettingId::WindowTransparency.spec();
    if let Some(value) = kit.choice_setting(
        ui,
        "settings::transparency",
        spec.label,
        spec.info_tooltip,
        state.config.window_transparency(),
        &WindowTransparency::ALL,
        |value| value.label().to_owned(),
    ) {
        state.config.set_window_transparency(value);
    }
    if let Some(style) = kit.choice_setting(
        ui,
        "settings::fullscreen-style",
        "Fullscreen Style",
        Some("How F11 and the title bar's fullscreen button take the window fullscreen. Borderless covers the screen without changing the display mode, so it switches instantly. Exclusive takes over the display and its video mode where the system supports it. Default: Borderless."),
        state.config.fullscreen_style(),
        &config::FullscreenStyle::ALL,
        |style| style.label().to_owned(),
    ) {
        state.config.set_fullscreen_style(style);
        // Already fullscreen: switch to the new style right away.
        if vertex_shell::fullscreen() != vertex_shell::Fullscreen::Off {
            vertex_shell::request_fullscreen(match style {
                config::FullscreenStyle::Borderless => vertex_shell::Fullscreen::Borderless,
                config::FullscreenStyle::Exclusive => vertex_shell::Fullscreen::Exclusive,
            });
        }
    }
    toggles(
        ui,
        kit,
        &mut state.config,
        &[ToggleSettingId::StartFullscreen],
    );
    let translucent = state.config.window_transparency().is_translucent();
    if let Some(value) = kit.slider_setting_ex(
        ui,
        "settings::ui-opacity",
        "UI Opacity",
        Some("Only active when Window Transparency is Transparent or Blurred. 100% is fully opaque UI chrome and 0% is fully transparent. Default: 100%."),
        f64::from(state.config.ui_opacity_percent()),
        f64::from(config::UI_OPACITY_PERCENT_MIN),
        f64::from(config::UI_OPACITY_PERCENT_MAX),
        1.0,
        translucent,
        |value| format!("{value:.0}%"),
    ) {
        state.config.set_ui_opacity_percent(value.round() as u8);
    }
    if let Some(value) = kit.slider_setting_ex(
        ui,
        "settings::corner-radius",
        "Window Corner Radius",
        Some("How round the window's corners are, in pixels (0 is square). Works with every Window Transparency mode; set it to 0 on an Opaque window to keep your OS's own corners. Default: 12."),
        f64::from(state.config.window_corner_radius()),
        f64::from(config::WINDOW_CORNER_RADIUS_MIN),
        f64::from(config::WINDOW_CORNER_RADIUS_MAX),
        1.0,
        true,
        |value| format!("{value:.0} px"),
    ) {
        state.config.set_window_corner_radius(value.round() as u8);
    }
    toggles(
        ui,
        kit,
        &mut state.config,
        &[ToggleSettingId::DragScrollEnabled],
    );
}

/// Saves the theme as JSON in the themes folder and offers to open that folder.
fn export_theme(theme: &vertex_theme::Theme) {
    match theme.export_json_to_themes_dir() {
        Ok(path) => {
            let folder = path.parent().map(Path::to_path_buf);
            notification::emit_with_actions(
                notification::Severity::Info,
                "Settings",
                format!(
                    "Exported {} to {}. Give the copy a new \"id\" and it loads as its own theme.",
                    theme.name,
                    path.display()
                ),
                vec![notification::NotificationAction::new(
                    "Open folder",
                    move || {
                        if let Some(folder) = folder.as_deref()
                            && let Err(err) = launcher_ui::desktop::open_in_file_manager(folder)
                        {
                            notification::error!(
                                "Settings",
                                "Failed to open the themes folder: {}",
                                err
                            );
                        }
                    },
                )],
            );
        }
        Err(err) => notification::error!("Settings", "Failed to export theme: {}", err),
    }
}

// -- Privacy --------------------------------------------------------------------------------------------------------

fn privacy(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    toggles(
        ui,
        kit,
        &mut state.config,
        &[
            ToggleSettingId::StreamerModeEnabled,
            ToggleSettingId::ObsDetectionEnabled,
        ],
    );
    kit::label(
        ui,
        INSECURE_STORAGE_WARNING,
        &TextStyle {
            wrap: TextWrap::Words,
            ..kit.style(TextRole::Danger)
        },
    );
    let tooltip = format!(
        "Stores sign-in tokens in {} instead of the system keyring. The file is not encrypted. Turn this off to move them back into the keyring. Default: Off.",
        auth::insecure_token_file_path().display()
    );
    let current = state.config.insecure_file_account_storage();
    let mut requested = current;
    kit.toggle_setting(
        ui,
        "settings::insecure-storage",
        "Use Unsecured File Account Storage",
        Some(&tooltip),
        &mut requested,
    );
    if requested && !current {
        // Turning it on only opens the confirmation; turning it off applies immediately.
        state.dialog = Some(Dialog::ConfirmInsecureStorage);
    } else if !requested && current {
        state.config.set_insecure_file_account_storage(false);
        auth::set_insecure_file_storage(false);
    }
}

// -- Text -----------------------------------------------------------------------------------------------------------

fn text(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let options = state.font_options.clone();
    let spec = config::DropdownSettingId::UiFontFamily.spec();
    let current = options
        .ui
        .iter()
        .position(|family| family.matches(&state.config.ui_font_family()))
        .unwrap_or(0);
    let mut chosen = None;
    kit.setting_row(
        ui,
        "settings::ui-font-row",
        spec.label,
        spec.info_tooltip,
        |kit, ui| {
            chosen = kit.dropdown(ui, "settings::ui-font", current, &options.ui_labels, 320.0);
        },
    );
    if let Some(family) = chosen.and_then(|index| options.ui.get(index).cloned()) {
        state
            .config
            .for_each_dropdown_mut(|_, value| *value = family.clone());
    }
    floats(ui, kit, &mut state.config, &[FloatSettingId::UiFontSize]);
    ints(ui, kit, &mut state.config, &[IntSettingId::UiFontWeight]);

    let current = options
        .emoji
        .iter()
        .position(|family| family.matches(&state.config.ui_emoji_font_family()))
        .unwrap_or(0);
    let mut chosen = None;
    kit.setting_row(
        ui,
        "settings::emoji-font-row",
        "Emoji Font",
        Some("The font colour emoji are drawn with."),
        |kit, ui| {
            chosen = kit.dropdown(
                ui,
                "settings::emoji-font",
                current,
                &options.emoji_labels,
                320.0,
            );
        },
    );
    if let Some(family) = chosen.and_then(|index| options.emoji.get(index).cloned()) {
        state.config.set_ui_emoji_font_family(family);
    }
    let current = options
        .cjk
        .iter()
        .position(|family| family.matches(&state.config.ui_cjk_font_family()))
        .unwrap_or(0);
    let mut chosen = None;
    kit.setting_row(
        ui,
        "settings::cjk-font-row",
        "CJK Font",
        Some("The font Chinese, Japanese and Korean text falls back to."),
        |kit, ui| {
            chosen = kit.dropdown(
                ui,
                "settings::cjk-font",
                current,
                &options.cjk_labels,
                320.0,
            );
        },
    );
    if let Some(family) = chosen.and_then(|index| options.cjk.get(index).cloned()) {
        state.config.set_ui_cjk_font_family(family);
    }
    if let Some(path) = kit.choice_setting(
        ui,
        "settings::text-path",
        "Text Rendering Path",
        Some("Controls how glyphs are rasterized for the launcher UI. Auto keeps the default path. SDF/MSDF are mainly useful for testing the new text pipeline. Default: Auto."),
        state.config.text_rendering_path(),
        &TextRenderingPath::ALL,
        |path| path.label().to_owned(),
    ) {
        state.config.set_text_rendering_path(path);
    }

    typography(ui, kit, state);

    toggles(
        ui,
        kit,
        &mut state.config,
        &[ToggleSettingId::OpenTypeFeaturesEnabled],
    );
    texts(
        ui,
        kit,
        &mut state.config,
        &[TextSettingId::OpenTypeFeaturesToEnable],
    );
}

fn typography(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    kit.collapsible(ui, "settings::text-styles", "Text styles", |ui, kit| {
        typography_body(ui, kit, state)
    });
}

fn typography_body(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    kit.text(ui, TextRole::Muted, "Size, weight, and font for each kind of text. These stack with the UI font size and weight above.");
    let options = state.font_options.clone();
    let mut font_labels = vec!["Launcher UI font (default)".to_owned()];
    font_labels.extend(options.ui_labels.iter().cloned());
    let mut cjk_labels = vec!["Use general CJK font".to_owned()];
    cjk_labels.extend(options.cjk_labels.iter().cloned());
    let mut emoji_labels = vec!["Use general emoji font".to_owned()];
    emoji_labels.extend(options.emoji_labels.iter().cloned());

    for role in config::TextRole::ALL {
        let defaults = role.defaults();
        let current = state.config.typography().override_for(role);
        let mut next = current.clone();
        let slug = role.slug();
        kit.text(ui, TextRole::Body, role.label());
        if let Some(size) = kit.slider_setting(
            ui,
            &format!("settings::type::{slug}::size"),
            "Size",
            Some("Font size for this text role, in points."),
            f64::from(current.size.unwrap_or(defaults.size)),
            f64::from(config::ROLE_FONT_SIZE_MIN),
            f64::from(config::ROLE_FONT_SIZE_MAX),
            f64::from(config::ROLE_FONT_SIZE_STEP),
            |value| format!("{value:.1}"),
        ) {
            next.size = Some(size as f32);
        }
        if let Some(weight) = kit.slider_setting(
            ui,
            &format!("settings::type::{slug}::weight"),
            "Weight",
            Some("Font weight from 100 (thin) to 900 (black), in steps of 100; 400 is regular and 700 is bold. Fonts that lack the chosen weight may look different. Default: 400."),
            f64::from(current.weight.unwrap_or(defaults.weight)),
            f64::from(config::UI_FONT_WEIGHT_MIN),
            f64::from(config::UI_FONT_WEIGHT_MAX),
            f64::from(config::UI_FONT_WEIGHT_STEP),
            |value| format!("{value:.0}"),
        ) {
            next.weight = Some(weight.round() as i32);
        }
        let font_index = current
            .font_family
            .as_deref()
            .and_then(|name| {
                options
                    .ui
                    .iter()
                    .position(|font| font.matches_name(name))
                    .map(|index| index + 1)
            })
            .unwrap_or(0);
        let mut chosen = None;
        kit.setting_row(
            ui,
            &format!("settings::type::{slug}::font-row"),
            "Font",
            Some("Font family used for this text role."),
            |kit, ui| {
                chosen = kit.dropdown(
                    ui,
                    &format!("settings::type::{slug}::font"),
                    font_index,
                    &font_labels,
                    300.0,
                );
            },
        );
        if let Some(index) = chosen {
            next.font_family = index
                .checked_sub(1)
                .and_then(|i| options.ui.get(i))
                .map(|font| font.label().to_owned());
        }
        let cjk_names: Vec<&str> = options
            .cjk
            .iter()
            .map(config::UiCjkFontFamily::family_name)
            .collect();
        let cjk_index = script_index(current.cjk_font_family.as_deref(), &cjk_names);
        let mut chosen = None;
        kit.setting_row(
            ui,
            &format!("settings::type::{slug}::cjk-row"),
            "CJK font",
            Some("Font for Chinese, Japanese and Korean characters in this kind of text. Leave on the general setting to follow the CJK Font choice above."),
            |kit, ui| chosen = kit.dropdown(ui, &format!("settings::type::{slug}::cjk"), cjk_index, &cjk_labels, 300.0),
        );
        if let Some(index) = chosen {
            next.cjk_font_family = index
                .checked_sub(1)
                .and_then(|i| cjk_names.get(i))
                .map(|name| (*name).to_owned());
        }
        let emoji_names: Vec<&str> = options
            .emoji
            .iter()
            .map(config::UiEmojiFontFamily::family_name)
            .collect();
        let emoji_index = script_index(current.emoji_font_family.as_deref(), &emoji_names);
        let mut chosen = None;
        kit.setting_row(
            ui,
            &format!("settings::type::{slug}::emoji-row"),
            "Emoji font",
            Some("Font for emoji in this kind of text. Leave on the general setting to follow the Emoji Font choice above."),
            |kit, ui| chosen = kit.dropdown(ui, &format!("settings::type::{slug}::emoji"), emoji_index, &emoji_labels, 300.0),
        );
        if let Some(index) = chosen {
            next.emoji_font_family = index
                .checked_sub(1)
                .and_then(|i| emoji_names.get(i))
                .map(|name| (*name).to_owned());
        }
        if kit.button(
            ui,
            &format!("settings::type::{slug}::reset"),
            "Reset to default",
            ButtonKind::Neutral,
            true,
        ) {
            state.config.typography_mut().reset(role);
        } else if next != current {
            state
                .config
                .typography_mut()
                .set_override(role, normalize_override(next, defaults));
        }
        kit.separator(ui);
    }
}

/// An override equal to the built-in value is no override.
fn normalize_override(mut value: RoleOverride, defaults: config::RoleDefaults) -> RoleOverride {
    if value
        .size
        .is_some_and(|size| (size - defaults.size).abs() < f32::EPSILON)
    {
        value.size = None;
    }
    if value.weight == Some(defaults.weight) {
        value.weight = None;
    }
    value
}

fn script_index(current: Option<&str>, names: &[&str]) -> usize {
    current
        .and_then(|name| {
            names
                .iter()
                .position(|candidate| candidate.eq_ignore_ascii_case(name))
        })
        .map_or(0, |index| index + 1)
}

// -- Graphics -------------------------------------------------------------------------------------------------------

fn graphics(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    if let Some(mode) = kit.choice_setting(
        ui,
        "settings::texel-aa",
        "Skin Preview Texel Edge AA",
        Some("Smooths the hard pixel edges between skin texels inside the skin shader, on top of the post anti-aliasing below. Off keeps crisp pixel-art edges. Default: Off."),
        state.config.skin_preview_texel_aa_mode(),
        &SkinPreviewTexelAaMode::ALL,
        |mode| mode.label().to_owned(),
    ) {
        state.config.set_skin_preview_texel_aa_mode(mode);
    }
    if let Some(mode) = kit.choice_setting(
        ui,
        "settings::skin-aa",
        "Skin Preview Post Anti-Aliasing",
        Some("How the skin preview is smoothed. MSAA samples the scene itself (see MSAA Samples); SMAA and FXAA are post-process filters; TAA blends frames over time; Off is fastest. Changes apply immediately. Default: FXAA (Post)."),
        state.config.skin_preview_aa_mode(),
        &SkinPreviewAaMode::ALL,
        |mode| mode.label().to_owned(),
    ) {
        state.config.set_skin_preview_aa_mode(mode);
    }
    if state.config.skin_preview_aa_mode() == SkinPreviewAaMode::Msaa {
        ints(
            ui,
            kit,
            &mut state.config,
            &[IntSettingId::SkinPreviewMsaaSamples],
        );
    }
    if let Some(mode) = kit.choice_setting(
        ui,
        "settings::svg-aa",
        "SVG Anti-Aliasing",
        Some("Controls supersampled SVG rasterization for launcher icons. Changes apply immediately. Default: Balanced (SSAA 2x)."),
        state.config.svg_aa_mode(),
        &SvgAaMode::ALL,
        |mode| mode.label().to_owned(),
    ) {
        state.config.set_svg_aa_mode(mode);
    }
    toggles(
        ui,
        kit,
        &mut state.config,
        &[
            ToggleSettingId::SkinPreviewFreshFormatEnabled,
            ToggleSettingId::SkinPreview3dLayersEnabled,
        ],
    );

    let mut blur = state.config.skin_preview_motion_blur_enabled();
    if kit.toggle_setting(
        ui,
        "settings::motion-blur",
        "Enable Skin Preview Motion Blur",
        Some("Blurs the moving 3D skin preview by blending several moments across the shutter interval. Applies immediately. Default: Off."),
        &mut blur,
    ) {
        state.config.set_skin_preview_motion_blur_enabled(blur);
    }
    if state.config.skin_preview_motion_blur_enabled() {
        let amount = FloatSettingId::SkinPreviewMotionBlurAmount.spec();
        if let Some(value) = kit.slider_setting(
            ui,
            "settings::motion-blur-amount",
            amount.label,
            Some("Longer shutter intervals produce stronger blur but more trailing. Best paired with SMAA, FXAA, or TAA."),
            f64::from(state.config.skin_preview_motion_blur_amount()),
            f64::from(amount.min),
            f64::from(amount.max),
            f64::from(amount.step),
            |value| format!("{:.0}%", value * 100.0),
        ) {
            state.config.set_skin_preview_motion_blur_amount(value as f32);
        }
        let shutter = FloatSettingId::SkinPreviewMotionBlurShutterFrames.spec();
        if let Some(value) = kit.slider_setting(
            ui,
            "settings::motion-blur-shutter",
            shutter.label,
            Some("Total shutter interval measured in 60 FPS frames. Higher values create longer streaks."),
            f64::from(state.config.skin_preview_motion_blur_shutter_frames()),
            f64::from(shutter.min),
            f64::from(shutter.max),
            f64::from(shutter.step),
            |value| format!("{value:.2}"),
        ) {
            state.config.set_skin_preview_motion_blur_shutter_frames(value as f32);
        }
        let samples = IntSettingId::SkinPreviewMotionBlurSampleCount.spec();
        if let Some(value) = kit.slider_setting(
            ui,
            "settings::motion-blur-samples",
            samples.label,
            Some("How many snapshots of the skin preview are blended across the shutter interval (2\u{2013}16). More samples make smoother blur and cost more GPU work. Default: 5."),
            f64::from(state.config.skin_preview_motion_blur_sample_count()),
            f64::from(samples.min),
            f64::from(samples.max),
            f64::from(samples.step),
            |value| format!("{value:.0}"),
        ) {
            state.config.set_skin_preview_motion_blur_sample_count(value.round() as i32);
        }
    }

    adapters(ui, kit, state);

    let api_spec = config::DropdownSettingId::GraphicsApiPreference.spec();
    let supported = supported_graphics_apis();
    if !supported.contains(&state.config.graphics_api_preference()) {
        state
            .config
            .set_graphics_api_preference(GraphicsApiPreference::Auto);
    }
    if let Some(value) = kit.choice_setting(
        ui,
        "settings::graphics-api",
        api_spec.label,
        api_spec.info_tooltip,
        state.config.graphics_api_preference(),
        supported,
        graphics_api_label,
    ) {
        state.config.set_graphics_api_preference(value);
    }
    let mut hdr = state.config.hdr_when_available();
    if kit.toggle_setting(
        ui,
        "settings::hdr",
        "HDR When Available",
        Some("Render the launcher in HDR on displays that support it. Default: Off."),
        &mut hdr,
    ) {
        state.config.set_hdr_when_available(hdr);
    }
    if state.config.hdr_when_available() {
        hdr_controls(ui, kit, &mut state.config);
    }
    toggles(
        ui,
        kit,
        &mut state.config,
        &[ToggleSettingId::FrameLimiterEnabled],
    );
    downloads(ui, kit, state);
}

/// Every HDR control, shown while HDR is on.
fn hdr_controls(ui: &mut Ui<'_>, kit: &mut Kit, config: &mut Config) {
    if let Some(value) = kit.choice_setting(
        ui,
        "settings::hdr-color-space",
        "HDR Color Space",
        Some("The signal sent to the display while HDR is on. HDR10 (PQ) suits most monitors; scRGB is Windows' native HDR format; HLG and Dolby Vision need display and platform support. Default: HDR10 (PQ)."),
        config.hdr_color_space(),
        &config::HdrColorSpace::ALL,
        |value| value.label().to_owned(),
    ) {
        config.set_hdr_color_space(value);
    }
    if let Some(value) = kit.slider_setting(
        ui,
        "settings::hdr-ui-white",
        "HDR UI Brightness",
        Some("How bright the launcher's interface is drawn relative to the display's SDR white (1.00 = the same). Raise it if the UI looks dim next to HDR content. Default: 1.00."),
        f64::from(config.hdr_ui_white_scale()),
        f64::from(config::HDR_UI_WHITE_SCALE_MIN),
        f64::from(config::HDR_UI_WHITE_SCALE_MAX),
        0.05,
        |value| format!("{value:.2}x"),
    ) {
        config.set_hdr_ui_white_scale(value as f32);
    }
    // Only an HDR10 (PQ) swapchain carries light-level metadata, so the sliders are for that mode alone.
    if config.hdr_color_space() == config::HdrColorSpace::Hdr10St2084 {
        if let Some(value) = kit.slider_setting(
            ui,
            "settings::hdr-max-cll",
            "HDR Max Content Light Level",
            Some("MaxCLL: the brightest pixel the launcher reports to the display, in nits. Displays use it to decide how to tone-map. Default: 1000."),
            f64::from(config.hdr_max_cll_nits()),
            f64::from(config::HDR_LIGHT_LEVEL_NITS_MIN),
            f64::from(config::HDR_LIGHT_LEVEL_NITS_MAX),
            50.0,
            |value| format!("{value:.0} nits"),
        ) {
            config.set_hdr_max_cll_nits(value.round() as i32);
        }
        if let Some(value) = kit.slider_setting(
            ui,
            "settings::hdr-max-fall",
            "HDR Max Frame-Average Light Level",
            Some("MaxFALL: the brightest average frame the launcher reports to the display, in nits (never above Max Content Light Level). Default: 400."),
            f64::from(config.hdr_max_fall_nits()),
            f64::from(config::HDR_LIGHT_LEVEL_NITS_MIN),
            f64::from(config::HDR_LIGHT_LEVEL_NITS_MAX),
            50.0,
            |value| format!("{value:.0} nits"),
        ) {
            config.set_hdr_max_fall_nits(value.round() as i32);
        }
    }
}

fn adapters(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let type_spec = config::DropdownSettingId::GraphicsAdapterPreferenceType.spec();
    if let Some(value) = kit.choice_setting(
        ui,
        "settings::adapter-type",
        type_spec.label,
        type_spec.info_tooltip,
        state.config.graphics_adapter_preference_type(),
        &GraphicsAdapterPreferenceType::ALL,
        |value| value.settings_label().to_owned(),
    ) {
        state.config.set_graphics_adapter_preference_type(value);
    }
    let pref_spec = config::DropdownSettingId::GraphicsAdapterPreference.spec();
    match state.config.graphics_adapter_preference_type() {
        GraphicsAdapterPreferenceType::PerformanceProfile => {
            if let Some(value) = kit.choice_setting(
                ui,
                "settings::adapter-profile",
                pref_spec.label,
                pref_spec.info_tooltip,
                state.config.graphics_adapter_profile(),
                &GraphicsAdapterProfile::ALL,
                |value| value.settings_label().to_owned(),
            ) {
                state.config.set_graphics_adapter_profile(value);
            }
        }
        GraphicsAdapterPreferenceType::ExplicitAdapter => {
            let adapters = state.settings.adapters.clone().unwrap_or_default();
            let mut labels: Vec<String> = adapters.iter().map(|(name, _)| name.clone()).collect();
            let saved = state.config.graphics_adapter_explicit_hash();
            let found = saved.and_then(|hash| {
                adapters
                    .iter()
                    .position(|(_, candidate)| *candidate == hash)
            });
            // A saved adapter that is gone is shown as such; the engine falls back to the High Performance profile.
            let missing = saved.is_some() && found.is_none();
            if missing {
                labels.insert(
                    0,
                    "Missing adapter (falls back to High Performance)".to_owned(),
                );
            }
            if labels.is_empty() {
                labels.push("No graphics adapters detected".to_owned());
            }
            let selected = found.map_or(0, |index| index + usize::from(missing));
            let mut chosen = None;
            kit.setting_row(
                ui,
                "settings::adapter-row",
                pref_spec.label,
                pref_spec.info_tooltip,
                |kit, ui| {
                    chosen = kit.dropdown(ui, "settings::adapter", selected, &labels, 360.0);
                },
            );
            if let Some(index) = chosen
                && !(missing && index == 0)
                && let Some((_, hash)) = adapters.get(index.saturating_sub(usize::from(missing)))
            {
                state.config.set_graphics_adapter_explicit_hash(Some(*hash));
            }
        }
    }
}

/// The graphics APIs this platform's engine build can run on. macOS is WebGPU-only; elsewhere WebGPU (Dawn) is the
/// fallback for GPUs that neither Vulkan nor DirectX 12 can reach.
fn supported_graphics_apis() -> &'static [GraphicsApiPreference] {
    if cfg!(target_os = "windows") {
        &[
            GraphicsApiPreference::Auto,
            GraphicsApiPreference::Vulkan,
            GraphicsApiPreference::Dx12,
            GraphicsApiPreference::WebGpu,
        ]
    } else if cfg!(target_os = "macos") {
        &[GraphicsApiPreference::Auto, GraphicsApiPreference::Metal]
    } else {
        &[
            GraphicsApiPreference::Auto,
            GraphicsApiPreference::Vulkan,
            GraphicsApiPreference::WebGpu,
        ]
    }
}

fn graphics_api_label(value: GraphicsApiPreference) -> String {
    match value {
        // The engine drives macOS through WebGPU; the stored value keeps its old name.
        GraphicsApiPreference::Metal => "WebGPU".to_owned(),
        other => other.settings_label().to_owned(),
    }
}

fn downloads(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let concurrency = f64::from(state.config.download_max_concurrent());
    if let Some(value) = kit.slider_setting(
        ui,
        "settings::downloads",
        "Max Concurrent Downloads",
        Some("How many files the launcher downloads at the same time (1\u{2013}128) for game files, libraries and mods. Higher can be faster on fast connections but may trigger rate limits from download servers. Default: 8."),
        concurrency,
        f64::from(config::DOWNLOAD_CONCURRENCY_MIN),
        f64::from(config::DOWNLOAD_CONCURRENCY_MAX),
        1.0,
        |value| format!("{value:.0}"),
    ) {
        state.config.set_download_max_concurrent(value.round().max(1.0) as u32);
    }
    let mut limit = state.config.download_speed_limit_enabled();
    if kit.toggle_setting(
        ui,
        "settings::speed-limit-on",
        "Enable Download Speed Limit",
        Some("Limits how fast the launcher downloads. Turn it on to set the speed; if the speed is empty or not valid, no limit is applied. Default: Off."),
        &mut limit,
    ) {
        state.config.set_download_speed_limit_enabled(limit);
    }
    // The limit's value only matters while the limit is on, so the box (and its feedback) appear with the toggle.
    if state.config.download_speed_limit_enabled() {
        let mut speed = state.config.download_speed_limit().to_owned();
        if kit.text_setting(
            ui,
            "settings::speed-limit",
            "Download Speed Limit",
            Some("How fast downloads may go, for example 250Mbps, 250 Mbps, 30MB/s or 1.5 Gbps. Lowercase b (Mbps, Mbit/s) is bits and uppercase B (MB/s, MiB/s) is bytes, so 80Mbps is about 10 MB/s. A bare number is Mbps. Default: empty (no limit)."),
            &mut speed,
            "250Mbps",
            false,
        ) {
            // Digits, a decimal point, the unit's letters and its "/" ("30MB/s"): nothing else can be part of a speed.
            kit.restrict_text("settings::speed-limit", &mut speed, |ch| {
                ch.is_ascii_alphanumeric() || matches!(ch, '.' | '/' | ' ')
            });
            *state.config.download_speed_limit_mut() = speed;
        }
        let value = state.config.download_speed_limit().trim().to_owned();
        if value.is_empty() {
            kit.text(
                ui,
                TextRole::Muted,
                "Speed limit is on but empty, so no limit is applied. Enter something like 250Mbps or 30MB/s.",
            );
        } else if let Some(bps) = config::parse_bitrate_to_bps(&value) {
            kit.text(
                ui,
                TextRole::Muted,
                &format!("Downloads limited to {}.", config::describe_bitrate(bps)),
            );
        } else {
            kit.text(
                ui,
                TextRole::Danger,
                "Not a speed this understands; try 250Mbps or 30MB/s. No limit is applied until it is.",
            );
        }
    }
}

// -- Java -----------------------------------------------------------------------------------------------------------

fn java_paths(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    // Finished downloads are applied even while the section is closed.
    let mut views = Vec::new();
    for runtime in JavaRuntimeVersion::ALL {
        let (view, downloaded) = java_actions::poll(runtime);
        if let Some(path) = downloaded {
            state
                .config
                .set_java_runtime_path_ref(runtime, Some(path.as_path()));
        }
        if view.in_flight.is_some() {
            kit.keep_awake_for(std::time::Duration::from_millis(100));
        }
        views.push((runtime, view));
    }
    kit.collapsible(
        ui,
        "settings::java-paths",
        "Java JVM Paths (Optional)",
        |ui, kit| java_paths_body(ui, kit, state, views),
    );
}

fn java_paths_body(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    state: &mut AppState,
    views: Vec<(JavaRuntimeVersion, java_actions::JavaActionView)>,
) {
    kit.text(
        ui,
        TextRole::Muted,
        "Leave blank for None. Configure only versions you actually use.",
    );
    for (runtime, view) in views {
        let mut path = state
            .config
            .java_runtime_path(runtime)
            .unwrap_or_default()
            .to_owned();
        if kit.text_setting(
            ui,
            &format!("settings::java::{}", runtime.major()),
            runtime.label(),
            Some(runtime.info_tooltip()),
            &mut path,
            "",
            false,
        ) {
            let trimmed = path.trim().to_owned();
            state.config.set_java_runtime_path_ref(
                runtime,
                (!trimmed.is_empty()).then(|| Path::new(&trimmed)),
            );
        }
        let current = (!path.trim().is_empty()).then(|| path.trim().to_owned());
        let busy = view.in_flight.is_some();
        let download_label = if view.in_flight == Some(JavaActionKind::Download) {
            "Downloading...".to_owned()
        } else {
            format!("Download OpenJDK {}", runtime.major())
        };
        let validate_label = if view.in_flight == Some(JavaActionKind::Validate) {
            "Testing..."
        } else {
            "Test Java"
        };
        {
            let _row = kit.row(ui, kit::SPACE_SM);
            if kit.button(
                ui,
                &format!("settings::java::{}::download", runtime.major()),
                &download_label,
                ButtonKind::Neutral,
                !busy,
            ) {
                java_actions::request_download(runtime);
            }
            if kit.button(
                ui,
                &format!("settings::java::{}::test", runtime.major()),
                validate_label,
                ButtonKind::Neutral,
                !busy && current.is_some(),
            ) && let Some(path) = current.clone()
            {
                java_actions::request_validation(runtime, path);
            }
        }
        if current.is_none() && view.in_flight != Some(JavaActionKind::Download) {
            kit.text(
                ui,
                TextRole::Muted,
                "Set a Java executable path before testing, or download OpenJDK.",
            );
        }
        if let Some(message) = &view.message {
            kit::label(
                ui,
                message,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Muted)
                },
            );
        }
    }
}

// -- Instance defaults ----------------------------------------------------------------------------------------------

fn poll_purge(state: &mut AppState) {
    if let Some(rx) = &state.settings.purge_rx
        && let Ok(message) = rx.try_recv()
    {
        state.settings.purge_status = Some(message);
        state.settings.purge_rx = None;
    }
}

fn instance_defaults(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let mut root = state
        .config
        .minecraft_installations_root_path()
        .as_os_str()
        .to_string_lossy()
        .into_owned();
    if kit.text_setting(
        ui,
        "settings::installations-root",
        "Minecraft installations folder",
        Some("New instances are created under this folder as <folder>/<instance.minecraft_root>. Relative paths are resolved from the launcher working directory."),
        &mut root,
        "",
        false,
    ) {
        state.config.set_minecraft_installations_root(root);
    }
    let purging = state.settings.purge_rx.is_some();
    if purging {
        kit.keep_awake_for(std::time::Duration::from_millis(100));
    }
    if kit.button(
        ui,
        "settings::purge-cache",
        if purging {
            "Purging..."
        } else {
            "Purge metadata cache"
        },
        ButtonKind::Neutral,
        !purging,
    ) {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        state.settings.purge_rx = Some(rx);
        launcher_runtime::spawn_detached(async move {
            let message = match installation::purge_cache_async().await {
                Ok(()) => {
                    "Purged local metadata cache. Version lists will be re-fetched on next refresh."
                        .to_owned()
                }
                Err(err) => format!("Failed to purge metadata cache: {err}"),
            };
            let _ = tx.send(message);
            vertex_shell::wake();
        });
    }
    if let Some(message) = &state.settings.purge_status {
        kit::label(
            ui,
            message,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
    }

    let limits = launcher_ui::screens::instance_api::settings::override_limits(&state.config);
    if limits.memory_pending {
        kit.keep_awake_for(std::time::Duration::from_millis(50));
    }
    let max_memory = limits.memory_max_mib;
    let mut memory = state.config.default_instance_max_memory_mib();
    if memory > max_memory {
        memory = max_memory;
        state.config.set_default_instance_max_memory_mib(memory);
    }
    if let Some(value) = kit.slider_setting(
        ui,
        "settings::default-memory",
        "Default max memory allocation (MiB)",
        Some("Maximum memory (Java -Xmx) for every instance that does not set its own memory in its instance settings. Changing it affects those instances the next time they launch. The slider stops at the RAM this computer has. Default: 4096 MiB."),
        memory as f64,
        config::INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN as f64,
        max_memory.max(config::INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN + 1) as f64,
        config::INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP as f64,
        |value| format!("{value:.0} MiB"),
    ) {
        state.config.set_default_instance_max_memory_mib(value as u128);
    }
    let mut args = state.config.default_instance_cli_args().to_owned();
    if kit.text_setting(
        ui,
        "settings::default-args",
        "Default CLI args",
        Some("Extra JVM arguments for every instance that does not set its own in its instance settings (for example: -XX:+UseZGC). An instance's own arguments replace these rather than adding to them. Applied the next time the instance launches. Default: empty (no extra arguments)."),
        &mut args,
        "",
        false,
    ) {
        *state.config.default_instance_cli_args_mut() = args;
    }
}

// -- Platform -------------------------------------------------------------------------------------------------------

fn platform_section(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    use config::{
        LinuxBlurProtocol, MacosVisualEffectBlendingMode, MacosVisualEffectMaterial,
        MacosVisualEffectState, WindowsBackdropType,
    };
    if cfg!(target_os = "linux") {
        section(
            ui,
            kit,
            "linux",
            "Linux",
            "Linux-specific settings that apply across the launcher.",
            |ui, kit| {
                // Better Blur DX is offered, first, only where the effect exists: otherwise it is not an option at all.
                let blur_choices: Vec<LinuxBlurProtocol> = LinuxBlurProtocol::ALL
                    .into_iter()
                    .filter(|choice| *choice != LinuxBlurProtocol::BetterBlurDx)
                    .collect();
                let blur_choices = if state.better_blur.available() {
                    [&[LinuxBlurProtocol::BetterBlurDx][..], &blur_choices].concat()
                } else {
                    blur_choices
                };
                if let Some(value) = kit.choice_setting(
                ui,
                "settings::linux-blur",
                "Blur Protocol",
                Some("Linux compositor blur protocol used when Window Transparency is Blurred. Auto prefers ext_background_effect_v1 and falls back to KDE blur. Better Blur DX, offered only where that KWin effect is installed, is not a protocol: Vertex asks for no blur and the effect blurs the window by its window class."),
                state.config.linux_blur_protocol(),
                &blur_choices,
                |value| value.label().to_owned(),
            ) {
                state.config.set_linux_blur_protocol(value);
            }
                let own_path = state.config.linux_blur_protocol() == LinuxBlurProtocol::BetterBlurDx;
                match state.better_blur.status() {
                    BetterBlurDx::Active if own_path => {
                        kit.text(
                            ui,
                            TextRole::Muted,
                            "Better Blur DX blurs this window itself, by window class: Vertex asks the compositor for no blur. Its strength, noise, saturation, brightness, contrast, corner radius and refraction are settings of the effect, not of any one window.",
                        );
                        if state.better_blur.blurs_vertex() {
                            kit.text(
                                ui,
                                TextRole::Muted,
                                "Vertex's window class (org.sturdyfool10.VertexLauncher) is on the effect's list, so the window is blurred.",
                            );
                        } else {
                            kit.text(
                                ui,
                                TextRole::Muted,
                                "Vertex's window class (org.sturdyfool10.VertexLauncher) is not on the effect's list, so the window stays plain transparent. Vertex adds it when this is chosen; if that did not work, add it here.",
                            );
                            if kit.button(ui, "settings::cover-better-blur", "Add Vertex to Better Blur DX's window classes", ButtonKind::Primary, true) {
                                state.better_blur.cover_vertex();
                            }
                        }
                        if kit.button(ui, "settings::open-better-blur", "Open Desktop Effects (Better Blur DX settings)", ButtonKind::Neutral, true) {
                            super::super::super::better_blur::open_settings();
                        }
                    }
                    BetterBlurDx::Active => {
                        kit.text(
                            ui,
                            TextRole::Muted,
                            "Better Blur DX is on, so KWin blurs this window through the protocol above. Choose Better Blur DX as the protocol to let the effect blur it by window class instead.",
                        );
                    }
                    BetterBlurDx::Installed => kit.text(
                        ui,
                        TextRole::Muted,
                        "Better Blur DX is installed but not switched on. Turn it on in System Settings > Window Management > Desktop Effects (and turn KWin's own Blur off); Vertex then needs nothing else.",
                    ),
                    BetterBlurDx::NotFound => kit.text(
                        ui,
                        TextRole::Muted,
                        "Linux blur protocols expose only blur regions to clients; Vertex requests a full-window region. Blur strength, radius, saturation, and noise are controlled by the compositor.",
                    ),
                }
                let mut opengl = state.config.linux_set_opengl_driver();
                if kit.toggle_setting(
                ui,
                "settings::linux-opengl",
                "Set Linux OpenGL Driver",
                Some("Linux-only. Vertex will explicitly manage OpenGL driver environment variables for launched games. This affects all launched versions by default; versions using Vulkan directly should ignore it."),
                &mut opengl,
            ) {
                state.config.set_linux_set_opengl_driver(opengl);
            }
                let mut zink = state.config.linux_use_zink_driver();
                if kit.toggle_setting_ex(
                ui,
                "settings::linux-zink",
                "Use Zink Driver (Experimental)",
                Some("Linux-only. Experimental. When the setting above is enabled, forces Mesa Zink so OpenGL runs over Vulkan. Disable it to keep Mesa's default OpenGL driver selection. Versions using Vulkan directly should ignore it."),
                &mut zink,
                state.config.linux_set_opengl_driver(),
            ) {
                state.config.set_linux_use_zink_driver(zink);
            }
            },
        );
    } else if cfg!(target_os = "windows") {
        section(
            ui,
            kit,
            "windows",
            "Windows",
            "Windows-specific window composition and compatibility settings.",
            |ui, kit| {
                if let Some(value) = kit.choice_setting(
                ui,
                "settings::windows-backdrop",
                "Window Backdrop Type",
                Some("Windows DWM system backdrop material used when Window Transparency is Blurred. Auto tries modern backdrops before legacy blur; None requests DWMSBT_NONE."),
                state.config.windows_backdrop_type(),
                &WindowsBackdropType::ALL,
                |value| value.label().to_owned(),
            ) {
                state.config.set_windows_backdrop_type(value);
            }
                let tintable = matches!(
                    state.config.windows_backdrop_type(),
                    WindowsBackdropType::LegacyBlur | WindowsBackdropType::LegacyAcrylic
                );
                if let Some(value) = kit.slider_setting_ex(
                    ui,
                    "settings::windows-blur-tint-opacity",
                    "Blur Tint Strength",
                    Some("How strongly the blur is tinted with the colour below. 0 is no tint. Only the Legacy Blur and Legacy Acrylic backdrops can be tinted."),
                    f64::from(state.config.windows_blur_tint_opacity_percent()),
                    f64::from(config::WINDOWS_BLUR_TINT_OPACITY_MIN),
                    f64::from(config::WINDOWS_BLUR_TINT_OPACITY_MAX),
                    1.0,
                    tintable,
                    |value| format!("{value:.0}%"),
                ) {
                    state.config.set_windows_blur_tint_opacity_percent(value.round() as u8);
                }
                let tint_text = state
                    .settings
                    .tint_text
                    .get_or_insert_with(|| format!("#{:06x}", state.config.windows_blur_tint_rgb()));
                if tintable
                    && kit.text_setting(
                        ui,
                        "settings::windows-blur-tint",
                        "Blur Tint Colour",
                        Some("The tint colour as #rrggbb. Ignored until it is a complete colour."),
                        tint_text,
                        "#000000",
                        false,
                    )
                    && let Some(color) = launcher_ui::account_colors::parse_color(tint_text)
                {
                    let rgb = u32::from(color.r()) << 16 | u32::from(color.g()) << 8 | u32::from(color.b());
                    state.config.set_windows_blur_tint_rgb(rgb);
                }
                kit.text(
                ui,
                TextRole::Muted,
                "Applied only when Window Transparency is Blurred. Mica, Mica Alt and the system Acrylic are drawn by Windows with no strength or tint control; the two Legacy backdrops accept a tint.",
            );
            },
        );
    } else if cfg!(target_os = "macos") {
        section(
            ui,
            kit,
            "macos",
            "macOS",
            "macOS-specific settings.",
            |ui, kit| {
                if let Some(value) = kit.choice_setting(
                    ui,
                    "settings::mac-material",
                    "Visual Effect Material",
                    Some("NSVisualEffectView material used when macOS native blur is active."),
                    state.config.macos_visual_effect_material(),
                    &MacosVisualEffectMaterial::ALL,
                    |value| value.label().to_owned(),
                ) {
                    state.config.set_macos_visual_effect_material(value);
                }
                if let Some(value) = kit.choice_setting(
                    ui,
                    "settings::mac-blending",
                    "Visual Effect Blending Mode",
                    Some("NSVisualEffectView blending mode used when macOS native blur is active."),
                    state.config.macos_visual_effect_blending_mode(),
                    &MacosVisualEffectBlendingMode::ALL,
                    |value| value.label().to_owned(),
                ) {
                    state.config.set_macos_visual_effect_blending_mode(value);
                }
                if let Some(value) = kit.choice_setting(
                    ui,
                    "settings::mac-state",
                    "Visual Effect State",
                    Some("NSVisualEffectView state used when macOS native blur is active."),
                    state.config.macos_visual_effect_state(),
                    &MacosVisualEffectState::ALL,
                    |value| value.label().to_owned(),
                ) {
                    state.config.set_macos_visual_effect_state(value);
                }
                let mut emphasized = state.config.macos_visual_effect_emphasized();
                if kit.toggle_setting(
                ui,
                "settings::mac-emphasized",
                "Visual Effect Emphasized",
                Some("Controls NSVisualEffectView.isEmphasized when macOS native blur is active."),
                &mut emphasized,
            ) {
                state.config.set_macos_visual_effect_emphasized(emphasized);
            }
                kit.text(
                    ui,
                    TextRole::Muted,
                    "Applied only when Window Transparency is Blurred.",
                );
            },
        );
    }
}

fn info_section(ui: &mut Ui<'_>, kit: &mut Kit) {
    section(
        ui,
        kit,
        "info",
        "Info",
        "Runtime and hardware information for this installation.",
        |ui, kit| {
            let Some(info) = super::super::super::app_metadata::try_settings_info() else {
                kit.text(ui, TextRole::Muted, "Collecting system information…");
                kit.keep_awake_for(std::time::Duration::from_millis(250));
                return;
            };
            for (label, value) in [
                ("CPU", info.cpu.as_str()),
                ("GPU", info.gpu.as_str()),
                ("Memory", info.memory.as_str()),
                ("Graphics API", info.graphics_api.as_str()),
                ("Graphics Driver + Version", info.graphics_driver.as_str()),
                ("App Version + Commit Hash", info.app_version.as_str()),
            ] {
                let _row = kit.row(ui, kit::SPACE_LG);
                {
                    let _label = ui.element(
                        &vertex_shell::sturdy::ui::ElementDesc::new()
                            .width(Sizing::Fixed(220.0))
                            .height(Sizing::Fit),
                    );
                    kit.text(ui, TextRole::Strong, label);
                }
                kit::label(
                    ui,
                    value,
                    &TextStyle {
                        wrap: TextWrap::Words,
                        ..kit.style(TextRole::Code)
                    },
                );
            }
        },
    );
}

/// The confirmation shown before unsecured account storage is switched on. Returns false once it should close.
pub fn confirm_insecure_storage(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) -> bool {
    let style = vertex_shell::sturdy::ui_kit::ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(560.0),
        ..vertex_shell::sturdy::ui_kit::ModalStyle::default()
    };
    let mut confirmed = false;
    let mut cancelled = false;
    let response = kit::modal(
        ui,
        "settings-insecure-confirm",
        &style,
        true,
        |ui| {
            let _column = kit.column(ui, kit::SPACE_MD);
            kit.text(ui, TextRole::Heading, "Use unsecured account storage?");
            kit::label(
                ui,
                INSECURE_STORAGE_WARNING,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Danger)
                },
            );
            kit::label(
                ui,
                &format!(
                    "Tokens will be written, unencrypted, to {}. Keep that file private, never share it, and leave it out of backups and screenshots.",
                    auth::insecure_token_file_path().display()
                ),
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Body)
                },
            );
            let _row = kit.row(ui, kit::SPACE_SM);
            kit.fill(ui);
            // Cancel is the prominent action, so a stray click never enables this.
            cancelled = kit.button(
                ui,
                "settings-insecure::cancel",
                "Cancel",
                ButtonKind::Primary,
                true,
            );
            confirmed = kit.button(
                ui,
                "settings-insecure::confirm",
                "Enable anyway",
                ButtonKind::Danger,
                true,
            );
        },
    );
    if confirmed {
        state.config.set_insecure_file_account_storage(true);
        auth::set_insecure_file_storage(true);
    }
    kit.forget("settings-insecure::");
    !(confirmed || cancelled || (response.dismissed || kit.back_pressed()))
}
