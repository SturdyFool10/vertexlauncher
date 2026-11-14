//! Settings, in the same sections, labels and help text as the egui screen. Every control edits `Config` directly;
//! the app saves the change and the engine appliers apply it on the next frame. Nothing needs a restart.

use vertex_i18n::{tr, trn};
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

const INSECURE_STORAGE_WARNING: &str = vertex_i18n::src!("Warning: this keeps your sign-in tokens in a plain-text file instead of your system's secure keyring. Doing so can improve compatibility with the Steam Deck, but it also means you need to be extra careful with your security, because it can make it really easy for your login details to be compromised: anyone or anything that can read that file can sign in as you.");

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
    /// What is typed in the search box at the top.
    search: String,
}

thread_local! {
    /// The search being applied while the page is drawn, and how many sections it left showing.
    static SEARCH: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static SHOWN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
        }
    }
    poll_purge(state);
    let _scroll = kit.scroll(ui, "settings::scroll", kit::SPACE_LG, kit::SPACE_XL);
    kit.text(ui, TextRole::Title, &tr!("Settings"));
    kit.text_field(
        ui,
        "settings::search",
        &mut state.settings.search,
        &tr!("Search settings"),
        false,
        Sizing::Grow,
    );
    SEARCH.with(|search| *search.borrow_mut() = state.settings.search.trim().to_owned());
    SHOWN.with(|shown| shown.set(0));

    section(
        ui,
        kit,
        "language",
        &tr!("Language"),
        &tr!("The language the launcher's interface is shown in."),
        |ui, kit| language(ui, kit, &mut state.config),
    );
    section(
        ui,
        kit,
        "appearance",
        &tr!("Appearance"),
        &tr!("Theme and launcher chrome settings."),
        |ui, kit| appearance(ui, kit, state),
    );
    section(
        ui,
        kit,
        "privacy",
        &tr!("Privacy"),
        &tr!("On-stream safety and account security settings."),
        |ui, kit| privacy(ui, kit, state),
    );
    section(
        ui,
        kit,
        "notifications",
        &tr!("Notifications"),
        &tr!("How notification popups expire and fade away."),
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
            &tr!("Notification Fade Out Easing"),
            Some(&tr!("The curve of the fade-out animation. Linear fades at a constant rate; Cubic and Exponential ease in, out, or both, and the Exponential curves are the most abrupt. Changes apply immediately. Default: Cubic In/Out.")),
            state.config.notification_easing(),
            &NotificationEasing::ALL,
            |easing| easing.label().to_owned(),
        ) {
            state.config.set_notification_easing(easing);
        }
            if kit.button(
                ui,
                "settings::test-notification",
                &tr!("Test notification"),
                ButtonKind::Neutral,
                true,
            ) {
                notification::emit(
                    notification::Severity::Info,
                    tr!("Settings"),
                    tr!("This is a test notification."),
                );
            }
        },
    );
    section(
        ui,
        kit,
        "text",
        &tr!("Text"),
        &tr!("Font, shaping, and glyph rendering settings that affect launcher text."),
        |ui, kit| text(ui, kit, state),
    );
    section(
        ui,
        kit,
        "discord",
        &tr!("Discord Rich Presence"),
        &tr!("Discord integration and launcher-owned activity reporting."),
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
        &tr!("Instance Sync"),
        &tr!("Keep multiplayer servers, command history, and creative hotbars in step across your instances. Sync runs before an instance launches and after it closes; right-click a server on Home to choose which instances it goes to."),
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
        &tr!("Graphics & Performance"),
        &tr!("GPU, skin preview rendering, frame pacing, and download throughput behavior."),
        |ui, kit| graphics(ui, kit, state),
    );
    section(
        ui,
        kit,
        "minecraft",
        &tr!("Minecraft"),
        &tr!("Version catalog behavior and content provider access."),
        |ui, kit| {
            kit.text(ui, TextRole::Heading, &tr!("Minecraft Version Groups"));
            kit.text(ui, TextRole::Muted, &tr!("Controls which non-default Minecraft version categories appear in version pickers."));
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
            &tr!("CurseForge API Key"),
            Some(&tr!("Required for CurseForge modpack zip imports. Overrides VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY and enables CurseForge browsing/download metadata.")),
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
        &tr!("Launching"),
        &tr!("What the launcher does when you start a game."),
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
        "logging",
        &tr!("Logging"),
        &tr!("How much the console shows. The console keeps its recent history, so choosing a more verbose level also shows lines that were hidden when they were logged."),
        |ui, kit| {
            if let Some(level) = kit.choice_setting(
                ui,
                "settings::vertex-log-level",
                &tr!("Vertex Log Level"),
                Some(&tr!("The least severe line shown in Vertex's own console. Debug lines are always kept; trace lines (the engine's progress and timings) are only kept while Trace is chosen. Default: Info.")),
                state.config.console_log_level(),
                &config::ConsoleLogLevel::ALL,
                |level| level.label().to_owned(),
            ) {
                state.config.set_console_log_level(level);
                apply_log_levels(&state.config);
            }
            if let Some(level) = kit.choice_setting(
                ui,
                "settings::game-log-level",
                &tr!("Game Log Level"),
                Some(&tr!("The least severe line shown in a game's console. Default: Info.")),
                state.config.game_console_log_level(),
                &config::ConsoleLogLevel::ALL,
                |level| level.label().to_owned(),
            ) {
                state.config.set_game_console_log_level(level);
                apply_log_levels(&state.config);
            }
        },
    );
    section(
        ui,
        kit,
        "java",
        &tr!("Java"),
        &tr!("Shared Java behavior for launches that use managed runtime selection."),
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
        &tr!("Instance Defaults"),
        &tr!("Used when creating new instances. You can still override values per instance."),
        |ui, kit| instance_defaults(ui, kit, state),
    );
    platform_section(ui, kit, state);
    section(
        ui,
        kit,
        "displayed-time",
        &tr!("Displayed Time"),
        &tr!("How dates, times and \"how long ago\" are written, such as on screenshots."),
        |ui, kit| displayed_time(ui, kit, &mut state.config),
    );
    section(
        ui,
        kit,
        "debug",
        &tr!("Debug"),
        &tr!("Live performance readouts drawn over the launcher."),
        |ui, kit| {
            toggles(
                ui,
                kit,
                &mut state.config,
                &[ToggleSettingId::DebugFpsCounter, ToggleSettingId::DebugFramegraph],
            );
        },
    );
    info_section(ui, kit);
    if SHOWN.with(|shown| shown.get()) == 0 {
        kit.text(ui, TextRole::Muted, &tr!("No settings match your search."));
    }
}

/// The interface language picker. Languages are the files in the launcher's `languages` folder, plus English.
fn language(ui: &mut Ui<'_>, kit: &mut Kit, config: &mut Config) {
    let languages = vertex_i18n::available_languages();
    // Choice 0 follows the system language; choice `n` is `languages[n - 1]`.
    let current = if config.language().is_empty() {
        0
    } else {
        languages
            .iter()
            .position(|language| language.tag.eq_ignore_ascii_case(config.language()))
            .map_or(0, |index| index + 1)
    };
    let choices: Vec<usize> = (0..=languages.len()).collect();
    if let Some(choice) = kit.choice_setting(
        ui,
        "settings::language",
        &tr!("Interface Language"),
        Some(&tr!("The language the launcher's text is shown in. Automatic follows your system's language and shows English when the launcher has no translation for it. More languages appear here when you add a translation file to the launcher's languages folder. Default: Automatic.")),
        current,
        &choices,
        |index| match index {
            0 => tr!("Automatic (system language)").into_owned(),
            n => languages[n - 1].native_name.clone(),
        },
    ) {
        match choice {
            0 => config.set_language(""),
            n => config.set_language(languages[n - 1].tag.clone()),
        }
    }
}

/// Every date and time format setting, with a live example under them.
fn displayed_time(ui: &mut Ui<'_>, kit: &mut Kit, config: &mut Config) {
    use config::{
        ClockStyle, DateFormat, DateOrder, DateSeparator, DisplayTimeZone, MeridiemStyle, MonthStyle,
        RELATIVE_MAX_UNITS_MAX, RELATIVE_MAX_UNITS_MIN, TimeSeparator,
    };
    let mut settings = config.displayed_time().clone();
    kit.toggle_setting(
        ui,
        "settings::time::screenshot-info",
        &tr!("Show Screenshot Time Card"),
        Some(&tr!("Shows when a screenshot was taken, and how long ago, in the corner of the screenshot viewer. The time comes from the screenshot's file name when it has one, otherwise from the file's modified time. Default: On.")),
        &mut settings.show_screenshot_info,
    );
    if let Some(value) = kit.choice_setting(
        ui,
        "settings::time::zone",
        &tr!("Time Zone"),
        Some(&tr!("Which time zone times are shown in. Default: this computer's.")),
        settings.time_zone,
        DisplayTimeZone::ALL,
        |value| value.label().to_owned(),
    ) {
        settings.time_zone = value;
    }
    if let Some(value) = kit.choice_setting(
        ui,
        "settings::time::date-format",
        &tr!("Date Format"),
        Some(&tr!("A ready-made date layout from around the world, or Custom to build your own from the settings below. Default: United States (mm/dd/yyyy).")),
        settings.date_format,
        DateFormat::ALL,
        |value| value.label().to_owned(),
    ) {
        settings.date_format = value;
    }
    if settings.date_format == DateFormat::Custom {
        if let Some(value) = kit.choice_setting(
            ui,
            "settings::time::date-order",
            &tr!("Date Order"),
            Some(&tr!("The order the day, month and year come in. Default: Month, day, year.")),
            settings.date_order,
            DateOrder::ALL,
            |value| value.label().to_owned(),
        ) {
            settings.date_order = value;
        }
        if let Some(value) = kit.choice_setting(
            ui,
            "settings::time::month-style",
            &tr!("Month Style"),
            Some(&tr!("Write the month as a number or as a name. Default: Number.")),
            settings.month_style,
            MonthStyle::ALL,
            |value| value.label().to_owned(),
        ) {
            settings.month_style = value;
        }
        if settings.month_style == MonthStyle::Numeric {
            if let Some(value) = kit.choice_setting(
                ui,
                "settings::time::date-separator",
                &tr!("Date Separator"),
                Some(&tr!("What goes between the numbers of a date. Default: Dash.")),
                settings.date_separator,
                DateSeparator::ALL,
                |value| value.label().to_owned(),
            ) {
                settings.date_separator = value;
            }
        }
        kit.toggle_setting(
            ui,
            "settings::time::four-digit-year",
            &tr!("Four-Digit Year"),
            Some(&tr!("2026 rather than 26. Default: On.")),
            &mut settings.four_digit_year,
        );
        kit.toggle_setting(
            ui,
            "settings::time::pad-date",
            &tr!("Pad Day and Month With Zeros"),
            Some(&tr!("03 rather than 3 (for a numeric month and the day). Default: On.")),
            &mut settings.pad_date,
        );
    }
    if settings.date_format.keeps_gregorian_year() {
        kit.toggle_setting(
            ui,
            "settings::time::human-era",
            &tr!("Human Era Years"),
            Some(&tr!("Counts years in the Human Era (Holocene) calendar and marks them HE, so 2026 becomes 12026 HE. It adds 10,000 to the year, a big commitment for a calendar whose main feature is a leading 1. Also a nice way to show some love for your favorite infographics channel with the cheerful little birds, the one that makes humanity's whole story look like a tidy timeline. Not available for layouts that already count years their own way (Thailand, Taiwan). Default: Off.")),
            &mut settings.human_era,
        );
    }
    kit.toggle_setting(
        ui,
        "settings::time::weekday",
        &tr!("Show Day of the Week"),
        Some(&tr!("Puts Mon, Tue... in front of the date. Default: Off.")),
        &mut settings.show_weekday,
    );
    if let Some(value) = kit.choice_setting(
        ui,
        "settings::time::clock",
        &tr!("Clock"),
        Some(&tr!("A 12-hour clock with AM and PM, or a 24-hour clock. Default: 12-hour.")),
        settings.clock,
        ClockStyle::ALL,
        |value| value.label().to_owned(),
    ) {
        settings.clock = value;
    }
    if settings.clock == ClockStyle::Hour12 {
        if let Some(value) = kit.choice_setting(
            ui,
            "settings::time::meridiem",
            &tr!("AM / PM Style"),
            Some(&tr!("How AM and PM are written. Default: AM / PM.")),
            settings.meridiem,
            MeridiemStyle::ALL,
            |value| value.label().to_owned(),
        ) {
            settings.meridiem = value;
        }
    }
    kit.toggle_setting(
        ui,
        "settings::time::pad-hour",
        &tr!("Pad Hour With a Zero"),
        Some(&tr!("09:05 rather than 9:05. Default: On.")),
        &mut settings.pad_hour,
    );
    kit.toggle_setting(
        ui,
        "settings::time::seconds",
        &tr!("Show Seconds"),
        Some(&tr!("Include seconds in the time. Default: On.")),
        &mut settings.show_seconds,
    );
    if let Some(value) = kit.choice_setting(
        ui,
        "settings::time::time-separator",
        &tr!("Time Separator"),
        Some(&tr!("What goes between hours, minutes and seconds. Default: Colon.")),
        settings.time_separator,
        TimeSeparator::ALL,
        |value| value.label().to_owned(),
    ) {
        settings.time_separator = value;
    }
    kit.toggle_setting(
        ui,
        "settings::time::relative",
        &tr!("Show How Long Ago"),
        Some(&tr!("A second line saying how long ago it was, or how far in the future if the clock was set ahead. Default: On.")),
        &mut settings.show_relative,
    );
    if settings.show_relative {
        if let Some(value) = kit.slider_setting(
            ui,
            "settings::time::relative-units",
            &tr!("Most Units Shown"),
            Some(&tr!("The most units the \"how long ago\" line shows, largest first: years, days, hours, minutes, seconds. Default: 5.")),
            f64::from(settings.relative_max_units),
            f64::from(RELATIVE_MAX_UNITS_MIN),
            f64::from(RELATIVE_MAX_UNITS_MAX),
            1.0,
            |value| format!("{value:.0}"),
        ) {
            settings.relative_max_units = value.round() as u8;
        }
        kit.toggle_setting(
            ui,
            "settings::time::relative-hide-zero",
            &tr!("Hide Zero Units"),
            Some(&tr!("Leave out units that are zero, so 1 hour 4 seconds is not 1 hour 0 minutes 4 seconds. Default: On.")),
            &mut settings.relative_hide_zero,
        );
        kit.toggle_setting(
            ui,
            "settings::time::relative-abbreviate",
            &tr!("Abbreviate Units"),
            Some(&tr!("1y 2d 3h rather than 1 year 2 days 3 hours. Default: Off.")),
            &mut settings.relative_abbreviate,
        );
    }
    // A fixed example moment, so the effect of each setting is visible without a screenshot.
    let now = config::unix_now();
    let example = now - (400 * 86_400 + 3 * 3_600 + 25 * 60 + 12);
    kit.text(ui, TextRole::Muted, &tr!("Example"));
    kit.text(ui, TextRole::Body, &settings.taken_at(example));
    if settings.show_relative {
        kit.text(ui, TextRole::Body, &settings.time_ago(example, now));
    }
    if settings != *config.displayed_time() {
        *config.displayed_time_mut() = settings;
    }
}

fn apply_log_levels(config: &config::Config) {
    crate::app::tracing_setup::apply_console_log_levels(
        config.console_log_level(),
        config.game_console_log_level(),
    );
}

/// A settings card. Its header folds the body away; while a search is typed, only the cards it matches are listed, open,
/// and a card matched through a setting inside it lists just the settings that match. What matched is lit.
fn section(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    heading: &str,
    description: &str,
    body: impl FnOnce(&mut Ui<'_>, &mut Kit),
) {
    let query = SEARCH.with(|search| search.borrow().clone());
    let searching = !query.is_empty();
    // A search that names the section lists all of it; one that names settings inside it lists only those.
    let mut filter = None;
    let mut heading_found = false;
    if searching {
        let mut names = vec![heading.to_owned()];
        names.extend(section_aliases(id));
        heading_found = kit::fuzzy_matches(&query, &names);
        if !heading_found {
            let found = section_keywords(id)
                .into_iter()
                .any(|keyword| kit::fuzzy_matches(&query, &[keyword]));
            if !found {
                return;
            }
            filter = Some(query);
        }
    }
    SHOWN.with(|shown| shown.set(shown.get() + 1));
    kit.collapsible_card(ui, &format!("settings::section::{id}"), heading, description, searching, heading_found, |ui, kit| {
        kit.set_setting_filter(filter);
        body(ui, kit);
        kit.set_setting_filter(None);
    });
}

/// Other names a section is found by. The language section is found by the word for "language" in any language the
/// launcher has, so someone who cannot read the current one can still get back to it.
fn section_aliases(id: &str) -> Vec<String> {
    if id != "language" {
        return Vec::new();
    }
    let mut names = vertex_i18n::translations_everywhere("Language");
    names.extend(vertex_i18n::translations_everywhere("Interface Language"));
    names
}

/// The names of the settings in the Graphics group of the Graphics & Performance section.
fn graphics_keywords() -> Vec<String> {
    use vertex_i18n::t;
    vec![
        t(IntSettingId::SkinPreviewMsaaSamples.spec().label).into_owned(),
        t(ToggleSettingId::SkinPreviewFreshFormatEnabled.spec().label).into_owned(),
        t(ToggleSettingId::SkinPreview3dLayersEnabled.spec().label).into_owned(),
        t(FloatSettingId::SkinPreviewMotionBlurAmount.spec().label).into_owned(),
        t(FloatSettingId::SkinPreviewMotionBlurShutterFrames.spec().label).into_owned(),
        t(IntSettingId::SkinPreviewMotionBlurSampleCount.spec().label).into_owned(),
        t(config::DropdownSettingId::GraphicsAdapterPreferenceType.spec().label).into_owned(),
        t(config::DropdownSettingId::GraphicsAdapterPreference.spec().label).into_owned(),
        t(config::DropdownSettingId::GraphicsApiPreference.spec().label).into_owned(),
        t(ToggleSettingId::FrameLimiterEnabled.spec().label).into_owned(),
        t(IntSettingId::FrameLimitFps.spec().label).into_owned(),
        tr!("Skin Preview Texel Edge AA").into_owned(),
        tr!("Skin Preview Post Anti-Aliasing").into_owned(),
        tr!("SVG Anti-Aliasing").into_owned(),
        tr!("Enable Skin Preview Motion Blur").into_owned(),
        tr!("Bindless UI Textures").into_owned(),
    ]
}

/// The names of the settings in the HDR group.
fn hdr_keywords() -> Vec<String> {
    [
        tr!("HDR When Available"),
        tr!("HDR Color Space"),
        tr!("HDR UI Brightness"),
        tr!("HDR UI Paper White"),
        tr!("HDR Max Content Light Level"),
        tr!("HDR Max Frame-Average Light Level"),
    ]
    .map(std::borrow::Cow::into_owned)
    .to_vec()
}

/// The names of the settings in the Downloads group.
fn download_keywords() -> Vec<String> {
    [tr!("Max Concurrent Downloads"), tr!("Enable Download Speed Limit"), tr!("Download Speed Limit")]
        .map(std::borrow::Cow::into_owned)
        .to_vec()
}

/// The names of the settings inside a section, so a search finds a section by what is in it.
fn section_keywords(id: &str) -> Vec<String> {
    use vertex_i18n::t;
    let toggle = |id: ToggleSettingId| t(id.spec().label).into_owned();
    let float = |id: FloatSettingId| t(id.spec().label).into_owned();
    let int = |id: IntSettingId| t(id.spec().label).into_owned();
    match id {
        "language" => vec![tr!("Interface Language").into_owned()],
        "appearance" => vec![
            tr!("Theme").into_owned(),
            tr!("Fullscreen Style").into_owned(),
            tr!("Window Controls").into_owned(),
            tr!("UI Opacity").into_owned(),
            tr!("Window Corner Radius").into_owned(),
            t(config::DropdownSettingId::WindowTransparency.spec().label).into_owned(),
            toggle(ToggleSettingId::StartFullscreen),
            toggle(ToggleSettingId::DragScrollEnabled),
        ],
        "privacy" => vec![
            toggle(ToggleSettingId::StreamerModeEnabled),
            toggle(ToggleSettingId::ObsDetectionEnabled),
            toggle(ToggleSettingId::LazyAccountRefresh),
            tr!("Use Unsecured File Account Storage").into_owned(),
        ],
        "notifications" => vec![
            toggle(ToggleSettingId::NotificationExpiryBarsEmptyLeft),
            float(FloatSettingId::NotificationFadeOutSeconds),
            tr!("Notification Fade Out Easing").into_owned(),
        ],
        "text" => vec![
            t(config::DropdownSettingId::UiFontFamily.spec().label).into_owned(),
            float(FloatSettingId::UiFontSize),
            int(IntSettingId::UiFontWeight),
            toggle(ToggleSettingId::OpenTypeFeaturesEnabled),
            t(TextSettingId::OpenTypeFeaturesToEnable.spec().label).into_owned(),
            tr!("Emoji Font").into_owned(),
            tr!("CJK Font").into_owned(),
            tr!("Text Rendering Path").into_owned(),
            tr!("Size").into_owned(),
            tr!("Weight").into_owned(),
            tr!("Font").into_owned(),
            tr!("CJK font").into_owned(),
            tr!("Emoji font").into_owned(),
        ],
        "discord" => vec![toggle(ToggleSettingId::DiscordRichPresenceEnabled)],
        "sync" => vec![
            toggle(ToggleSettingId::SyncServersEnabled),
            toggle(ToggleSettingId::SyncServersToAllInstancesByDefault),
            toggle(ToggleSettingId::SyncCommandHistoryEnabled),
            toggle(ToggleSettingId::SyncHotbarsEnabled),
        ],
        "graphics" => {
            let mut all = graphics_keywords();
            all.extend(hdr_keywords());
            all.extend(download_keywords());
            all
        }
        "minecraft" => vec![
            tr!("Minecraft Version Groups").into_owned(),
            toggle(ToggleSettingId::SnapshotsAndBetasEnabled),
            toggle(ToggleSettingId::AlphaVersionsEnabled),
            toggle(ToggleSettingId::ExperimentalVersionsEnabled),
            tr!("CurseForge API Key").into_owned(),
        ],
        "launching" => vec![
            toggle(ToggleSettingId::FollowConsoleOnLaunch),
            toggle(ToggleSettingId::SteamDeckHandoff),
        ],
        "logging" => vec![tr!("Vertex Log Level").into_owned(), tr!("Game Log Level").into_owned()],
        "java" => vec![
            toggle(ToggleSettingId::ForceJava21Minimum),
            tr!("Java JVM Paths (Optional)").into_owned(),
        ],
        "defaults" => vec![
            tr!("Minecraft installations folder").into_owned(),
            tr!("Default max memory allocation (MiB)").into_owned(),
            tr!("Default CLI args").into_owned(),
        ],
        "linux" => vec![
            tr!("Blur Protocol").into_owned(),
            tr!("Set Linux OpenGL Driver").into_owned(),
            tr!("Use Zink Driver (Experimental)").into_owned(),
        ],
        "windows" => vec![
            tr!("Window Backdrop Type").into_owned(),
            tr!("Blur Tint Strength").into_owned(),
            tr!("Blur Tint Colour").into_owned(),
        ],
        "macos" => vec![
            tr!("Visual Effect Material").into_owned(),
            tr!("Visual Effect Blending Mode").into_owned(),
            tr!("Visual Effect State").into_owned(),
            tr!("Visual Effect Emphasized").into_owned(),
        ],
        "displayed-time" => vec![
            tr!("Show Screenshot Time Card").into_owned(),
            tr!("Time Zone").into_owned(),
            tr!("Date Format").into_owned(),
            tr!("Date Order").into_owned(),
            tr!("Month Style").into_owned(),
            tr!("Date Separator").into_owned(),
            tr!("Four-Digit Year").into_owned(),
            tr!("Pad Day and Month With Zeros").into_owned(),
            tr!("Human Era Years").into_owned(),
            tr!("Show Day of the Week").into_owned(),
            tr!("Clock").into_owned(),
            tr!("AM / PM Style").into_owned(),
            tr!("Pad Hour With a Zero").into_owned(),
            tr!("Show Seconds").into_owned(),
            tr!("Time Separator").into_owned(),
            tr!("Show How Long Ago").into_owned(),
            tr!("Most Units Shown").into_owned(),
            tr!("Hide Zero Units").into_owned(),
            tr!("Abbreviate Units").into_owned(),
        ],
        "debug" => vec![
            toggle(ToggleSettingId::DebugFpsCounter),
            toggle(ToggleSettingId::DebugFramegraph),
        ],
        "info" => vec![
            tr!("CPU").into_owned(),
            tr!("GPU").into_owned(),
            tr!("Memory").into_owned(),
            tr!("Graphics Driver + Version").into_owned(),
            tr!("App Version + Commit Hash").into_owned(),
        ],
        _ => Vec::new(),
    }
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
    let tooltip = tr!("Themes are loaded from {folder}. Add or edit a theme file there and it appears here without restarting.", folder = app_paths::themes_dir().display()).into_owned();
    let mut chosen = None;
    kit.setting_row(
        ui,
        "settings::theme-row",
        &tr!("Theme"),
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
        &tr!("Export theme as JSON"),
        ButtonKind::Neutral,
        true,
    ) {
        export_theme(&state.theme);
    }
    kit.tooltip(
        ui,
        "settings::theme-export",
        &tr!("Saves the selected theme as a JSON file in the themes folder, to use as a starting point for your own theme."),
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
        &tr!("Fullscreen Style"),
        Some(&tr!("How F11 and the title bar's fullscreen button take the window fullscreen. Borderless covers the screen without changing the display mode, so it switches instantly. Exclusive takes over the display and its video mode where the system supports it. Default: Borderless.")),
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
    if let Some(style) = kit.choice_setting(
        ui,
        "settings::window-controls-style",
        &tr!("Window Controls"),
        Some(&tr!("The look of the title bar's minimize, maximize and restore buttons. Windows uses a dash, a window outline and overlapping squares; Linux uses a down arrow, an up arrow and a diamond. Default: the style of your operating system.")),
        state.config.window_controls_style(),
        &config::WindowControlsStyle::ALL,
        |style| style.label().to_owned(),
    ) {
        state.config.set_window_controls_style(style);
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
        &tr!("UI Opacity"),
        Some(&tr!("Only active when Window Transparency is Transparent or Blurred. 100% is fully opaque UI chrome and 0% is fully transparent. Default: 100%.")),
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
        &tr!("Window Corner Radius"),
        Some(&tr!("How round the window's corners are, in pixels (0 is square). Works with every Window Transparency mode; set it to 0 on an Opaque window to keep your OS's own corners. Default: 12.")),
        f64::from(state.config.window_corner_radius()),
        f64::from(config::WINDOW_CORNER_RADIUS_MIN),
        f64::from(config::WINDOW_CORNER_RADIUS_MAX),
        1.0,
        true,
        |value| tr!("{value:.0} px", value = value).into_owned(),
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
                tr!("Settings"),
                tr!("Exported {name} to {path}. Give the copy a new \"id\" and it loads as its own theme.", name = theme.name, path = path.display()).into_owned(),
                vec![notification::NotificationAction::new(
                    tr!("Open folder"),
                    move || {
                        if let Some(folder) = folder.as_deref()
                            && let Err(err) = launcher_ui::desktop::open_in_file_manager(folder)
                        {
                            notification::error!("Settings", "Failed to open the themes folder: {err}", err = err);
                        }
                    },
                )],
            );
        }
        Err(err) => notification::error!("Settings", "Failed to export theme: {err}", err = err),
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
            ToggleSettingId::LazyAccountRefresh,
        ],
    );
    kit::label(
        ui,
        &vertex_i18n::t(INSECURE_STORAGE_WARNING),
        &TextStyle {
            wrap: TextWrap::Words,
            ..kit.style(TextRole::Danger)
        },
    );
    let tooltip = tr!("Stores sign-in tokens in {path} instead of the system keyring. The file is not encrypted. Turn this off to move them back into the keyring. Default: Off.", path = auth::insecure_token_file_path().display()).into_owned();
    let current = state.config.insecure_file_account_storage();
    let mut requested = current;
    kit.toggle_setting(
        ui,
        "settings::insecure-storage",
        &tr!("Use Unsecured File Account Storage"),
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
        &tr!("Emoji Font"),
        Some(&tr!("The font colour emoji are drawn with.")),
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
        &tr!("CJK Font"),
        Some(&tr!("The font Chinese, Japanese and Korean text falls back to.")),
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
        &tr!("Text Rendering Path"),
        Some(&tr!("Controls how glyphs are rasterized for the launcher UI. Auto keeps the default path. SDF/MSDF are mainly useful for testing the new text pipeline. Default: Auto.")),
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
    let keywords = [tr!("Size"), tr!("Weight"), tr!("Font"), tr!("CJK font"), tr!("Emoji font")]
        .map(std::borrow::Cow::into_owned);
    group(ui, kit, "settings::text-styles", &tr!("Text styles"), &keywords, |ui, kit| {
        typography_body(ui, kit, state)
    });
}

/// A folding group inside a section. While a search lists only matching settings, a group with none is left out and
/// one with some is open, so what matched is not hidden in a folded group.
fn group(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    label: &str,
    keywords: &[String],
    body: impl FnOnce(&mut Ui<'_>, &mut Kit),
) {
    let searching = match kit.setting_filter() {
        Some(query) => {
            if !keywords.iter().any(|keyword| kit::fuzzy_matches(query, &[keyword.clone()])) {
                return;
            }
            true
        }
        None => false,
    };
    kit.collapsible_ex(ui, id, label, searching, body);
}

fn typography_body(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    kit.text(ui, TextRole::Muted, &tr!("Size, weight, and font for each kind of text. These stack with the UI font size and weight above."));
    let options = state.font_options.clone();
    let mut font_labels = vec![tr!("Launcher UI font (default)").into_owned()];
    font_labels.extend(options.ui_labels.iter().cloned());
    let mut cjk_labels = vec![tr!("Use general CJK font").into_owned()];
    cjk_labels.extend(options.cjk_labels.iter().cloned());
    let mut emoji_labels = vec![tr!("Use general emoji font").into_owned()];
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
            &tr!("Size"),
            Some(&tr!("Font size for this text role, in points.")),
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
            &tr!("Weight"),
            Some(&tr!("Font weight from 100 (thin) to 900 (black), in steps of 100; 400 is regular and 700 is bold. Fonts that lack the chosen weight may look different. Default: 400.")),
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
            &tr!("Font"),
            Some(&tr!("Font family used for this text role.")),
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
            &tr!("CJK font"),
            Some(&tr!("Font for Chinese, Japanese and Korean characters in this kind of text. Leave on the general setting to follow the CJK Font choice above.")),
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
            &tr!("Emoji font"),
            Some(&tr!("Font for emoji in this kind of text. Leave on the general setting to follow the Emoji Font choice above.")),
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
            &tr!("Reset to default"),
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
    group(ui, kit, "settings::graphics-general", &tr!("Graphics"), &graphics_keywords(), |ui, kit| {
        graphics_general(ui, kit, state)
    });
    group(ui, kit, "settings::graphics-hdr", &tr!("HDR"), &hdr_keywords(), |ui, kit| hdr_group(ui, kit, state));
    group(ui, kit, "settings::graphics-downloads", &tr!("Downloads"), &download_keywords(), |ui, kit| {
        downloads(ui, kit, state)
    });
}

fn graphics_general(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    if let Some(mode) = kit.choice_setting(
        ui,
        "settings::texel-aa",
        &tr!("Skin Preview Texel Edge AA"),
        Some(&tr!("Smooths the hard pixel edges between skin texels inside the skin shader, on top of the post anti-aliasing below. Off keeps crisp pixel-art edges. Default: Off.")),
        state.config.skin_preview_texel_aa_mode(),
        &SkinPreviewTexelAaMode::ALL,
        |mode| mode.label().to_owned(),
    ) {
        state.config.set_skin_preview_texel_aa_mode(mode);
    }
    if let Some(mode) = kit.choice_setting(
        ui,
        "settings::skin-aa",
        &tr!("Skin Preview Post Anti-Aliasing"),
        Some(&tr!("How the skin preview is smoothed. MSAA samples the scene itself (see MSAA Samples); SMAA and FXAA are post-process filters; TAA blends frames over time; Off is fastest. Changes apply immediately. Default: FXAA (Post).")),
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
        &tr!("SVG Anti-Aliasing"),
        Some(&tr!("Controls supersampled SVG rasterization for launcher icons. Changes apply immediately. Default: Balanced (SSAA 2x).")),
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

    let mut bindless = state.config.ui_bindless_textures();
    if kit.toggle_setting(
        ui,
        "settings::bindless-ui",
        &tr!("Bindless UI Textures"),
        Some(&tr!("Draws the launcher's interface through one shared texture table, which needs fewer draw calls and descriptor updates. Only used when your graphics driver supports it; otherwise the launcher falls back automatically. Turn it off if you see missing or wrong images in the interface. Default: On.")),
        &mut bindless,
    ) {
        state.config.set_ui_bindless_textures(bindless);
    }

    let mut blur = state.config.skin_preview_motion_blur_enabled();
    if kit.toggle_setting(
        ui,
        "settings::motion-blur",
        &tr!("Enable Skin Preview Motion Blur"),
        Some(&tr!("Blurs the moving 3D skin preview by blending several moments across the shutter interval. Applies immediately. Default: Off.")),
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
            Some(&tr!("Longer shutter intervals produce stronger blur but more trailing. Best paired with SMAA, FXAA, or TAA.")),
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
            Some(&tr!("Total shutter interval measured in 60 FPS frames. Higher values create longer streaks.")),
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
            Some(&tr!("How many snapshots of the skin preview are blended across the shutter interval (2–16). More samples make smoother blur and cost more GPU work. Default: 5.")),
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
    toggles(
        ui,
        kit,
        &mut state.config,
        &[ToggleSettingId::FrameLimiterEnabled],
    );
}

/// HDR on or off, and its controls while it is on.
fn hdr_group(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let mut hdr = state.config.hdr_when_available();
    if kit.toggle_setting(
        ui,
        "settings::hdr",
        &tr!("HDR When Available"),
        Some(&tr!("Render the launcher in HDR on displays that support it. Default: Off.")),
        &mut hdr,
    ) {
        state.config.set_hdr_when_available(hdr);
    }
    if state.config.hdr_when_available() {
        hdr_controls(ui, kit, &mut state.config);
    }
}

/// Every HDR control, shown while HDR is on.
fn hdr_controls(ui: &mut Ui<'_>, kit: &mut Kit, config: &mut Config) {
    if let Some(value) = kit.choice_setting(
        ui,
        "settings::hdr-color-space",
        &tr!("HDR Color Space"),
        Some(&tr!("The signal sent to the display while HDR is on. HDR10 (PQ) suits most monitors; scRGB is Windows' native HDR format; HLG and Dolby Vision need display and platform support. Default: HDR10 (PQ).")),
        config.hdr_color_space(),
        &config::HdrColorSpace::ALL,
        |value| value.label().to_owned(),
    ) {
        config.set_hdr_color_space(value);
    }
    if let Some(value) = kit.slider_setting(
        ui,
        "settings::hdr-ui-white",
        &tr!("HDR UI Brightness"),
        Some(&tr!("A multiplier on the launcher's interface brightness, applied on top of the UI paper white below (1.00 = unchanged). Lower it if the interface looks blown out. Default: 1.00.")),
        f64::from(config.hdr_ui_white_scale()),
        f64::from(config::HDR_UI_WHITE_SCALE_MIN),
        f64::from(config::HDR_UI_WHITE_SCALE_MAX),
        0.05,
        |value| format!("{value:.2}x"),
    ) {
        config.set_hdr_ui_white_scale(value as f32);
    }
    if let Some(value) = kit.slider_setting(
        ui,
        "settings::hdr-ui-paper-white",
        &tr!("HDR UI Paper White"),
        Some(&tr!("The brightness, in nits, of the interface's white. 0 uses the display's own SDR white level, which can be too bright when the system does not report it; try 80-200 if the interface looks blown out. Default: 0 (automatic).")),
        f64::from(config.hdr_ui_paper_white_nits()),
        f64::from(config::HDR_UI_PAPER_WHITE_NITS_MIN),
        f64::from(config::HDR_UI_PAPER_WHITE_NITS_MAX),
        5.0,
        |value| if value < 1.0 { "Auto".to_owned() } else { tr!("{value:.0} nits", value = value).into_owned() },
    ) {
        config.set_hdr_ui_paper_white_nits(value.round() as i32);
    }
    // Only an HDR10 (PQ) swapchain carries light-level metadata, so the sliders are for that mode alone.
    if config.hdr_color_space() == config::HdrColorSpace::Hdr10St2084 {
        if let Some(value) = kit.slider_setting(
            ui,
            "settings::hdr-max-cll",
            &tr!("HDR Max Content Light Level"),
            Some(&tr!("MaxCLL: the brightest pixel the launcher reports to the display, in nits. Displays use it to decide how to tone-map. Default: 1000.")),
            f64::from(config.hdr_max_cll_nits()),
            f64::from(config::HDR_LIGHT_LEVEL_NITS_MIN),
            f64::from(config::HDR_LIGHT_LEVEL_NITS_MAX),
            50.0,
            |value| tr!("{value:.0} nits", value = value).into_owned(),
        ) {
            config.set_hdr_max_cll_nits(value.round() as i32);
        }
        if let Some(value) = kit.slider_setting(
            ui,
            "settings::hdr-max-fall",
            &tr!("HDR Max Frame-Average Light Level"),
            Some(&tr!("MaxFALL: the brightest average frame the launcher reports to the display, in nits (never above Max Content Light Level). Default: 400.")),
            f64::from(config.hdr_max_fall_nits()),
            f64::from(config::HDR_LIGHT_LEVEL_NITS_MIN),
            f64::from(config::HDR_LIGHT_LEVEL_NITS_MAX),
            50.0,
            |value| tr!("{value:.0} nits", value = value).into_owned(),
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
                    tr!("Missing adapter (falls back to High Performance)").into_owned(),
                );
            }
            if labels.is_empty() {
                labels.push(tr!("No graphics adapters detected").into_owned());
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
        GraphicsApiPreference::Metal => tr!("WebGPU").into_owned(),
        other => other.settings_label().to_owned(),
    }
}

fn downloads(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let concurrency = f64::from(state.config.download_max_concurrent());
    if let Some(value) = kit.slider_setting(
        ui,
        "settings::downloads",
        &tr!("Max Concurrent Downloads"),
        Some(&tr!("How many files the launcher downloads at the same time (1–128) for game files, libraries and mods. Higher can be faster on fast connections but may trigger rate limits from download servers. Default: 8.")),
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
        &tr!("Enable Download Speed Limit"),
        Some(&tr!("Limits how fast the launcher downloads. Turn it on to set the speed; if the speed is empty or not valid, no limit is applied. Default: Off.")),
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
            &tr!("Download Speed Limit"),
            Some(&tr!("How fast downloads may go, for example 250Mbps, 250 Mbps, 30MB/s or 1.5 Gbps. Lowercase b (Mbps, Mbit/s) is bits and uppercase B (MB/s, MiB/s) is bytes, so 80Mbps is about 10 MB/s. A bare number is Mbps. Default: empty (no limit).")),
            &mut speed,
            &tr!("250Mbps"),
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
                &tr!("Speed limit is on but empty, so no limit is applied. Enter something like 250Mbps or 30MB/s."),
            );
        } else if let Some(bps) = config::parse_bitrate_to_bps(&value) {
            kit.text(
                ui,
                TextRole::Muted,
                &tr!("Downloads limited to {limit}.", limit = config::describe_bitrate(bps)),
            );
        } else {
            kit.text(
                ui,
                TextRole::Danger,
                &tr!("Not a speed this understands; try 250Mbps or 30MB/s. No limit is applied until it is."),
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
        }
        views.push((runtime, view));
    }
    kit.collapsible(
        ui,
        "settings::java-paths",
        &tr!("Java JVM Paths (Optional)"),
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
        &tr!("Leave blank for None. Configure only versions you actually use."),
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
            tr!("Downloading...").into_owned()
        } else {
            tr!("Download OpenJDK {version}", version = runtime.major()).into_owned()
        };
        let validate_label = if view.in_flight == Some(JavaActionKind::Validate) {
            &tr!("Testing...")
        } else {
            &tr!("Test Java")
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
                &tr!("Set a Java executable path before testing, or download OpenJDK."),
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
        &tr!("Minecraft installations folder"),
        Some(&tr!("New instances are created under this folder as <folder>/<instance.minecraft_root>. Relative paths are resolved from the launcher working directory.")),
        &mut root,
        "",
        false,
    ) {
        state.config.set_minecraft_installations_root(root);
    }
    let purging = state.settings.purge_rx.is_some();
    if purging {
    }
    if kit.button(
        ui,
        "settings::purge-cache",
        &if purging {
            tr!("Purging...")
        } else {
            tr!("Purge metadata cache")
        },
        ButtonKind::Neutral,
        !purging,
    ) {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        state.settings.purge_rx = Some(rx);
        launcher_runtime::spawn_detached(async move {
            let message = match installation::purge_cache_async().await {
                Ok(()) => {
                    tr!("Purged local metadata cache. Version lists will be re-fetched on next refresh.")
                        .into_owned()
                }
                Err(err) => tr!("Failed to purge metadata cache: {err}", err = err).into_owned(),
            };
            let _ = tx.send(message);
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
        &tr!("Default max memory allocation (MiB)"),
        Some(&tr!("Maximum memory (Java -Xmx) for every instance that does not set its own memory in its instance settings. Changing it affects those instances the next time they launch. The slider stops at the RAM this computer has. Default: 4096 MiB.")),
        memory as f64,
        config::INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN as f64,
        max_memory.max(config::INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN + 1) as f64,
        config::INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP as f64,
        |value| tr!("{value:.0} MiB", value = value).into_owned(),
    ) {
        state.config.set_default_instance_max_memory_mib(value as u128);
    }
    let mut args = state.config.default_instance_cli_args().to_owned();
    if kit.text_setting(
        ui,
        "settings::default-args",
        &tr!("Default CLI args"),
        Some(&tr!("Extra JVM arguments for every instance that does not set its own in its instance settings (for example: -XX:+UseZGC). An instance's own arguments replace these rather than adding to them. Applied the next time the instance launches. Default: empty (no extra arguments).")),
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
            &tr!("Linux"),
            &tr!("Linux-specific settings that apply across the launcher."),
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
                &tr!("Blur Protocol"),
                Some(&tr!("Linux compositor blur protocol used when Window Transparency is Blurred. Auto prefers ext_background_effect_v1 and falls back to KDE blur. Better Blur DX, offered only where that KWin effect is installed, is not a protocol: Vertex asks for no blur and the effect blurs the window by its window class.")),
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
                            &tr!("Better Blur DX blurs this window itself, by window class: Vertex asks the compositor for no blur. Its strength, noise, saturation, brightness, contrast, corner radius and refraction are settings of the effect, not of any one window."),
                        );
                        if state.better_blur.blurs_vertex() {
                            kit.text(
                                ui,
                                TextRole::Muted,
                                &tr!("Vertex's window class (org.sturdyfool10.VertexLauncher) is on the effect's list, so the window is blurred."),
                            );
                        } else {
                            kit.text(
                                ui,
                                TextRole::Muted,
                                &tr!("Vertex's window class (org.sturdyfool10.VertexLauncher) is not on the effect's list, so the window stays plain transparent. Vertex adds it when this is chosen; if that did not work, add it here."),
                            );
                            if kit.button(ui, "settings::cover-better-blur", &tr!("Add Vertex to Better Blur DX's window classes"), ButtonKind::Primary, true) {
                                state.better_blur.cover_vertex();
                            }
                        }
                        if kit.button(ui, "settings::open-better-blur", &tr!("Open Desktop Effects (Better Blur DX settings)"), ButtonKind::Neutral, true) {
                            super::super::super::better_blur::open_settings();
                        }
                    }
                    BetterBlurDx::Active => {
                        kit.text(
                            ui,
                            TextRole::Muted,
                            &tr!("Better Blur DX is on, so KWin blurs this window through the protocol above. Choose Better Blur DX as the protocol to let the effect blur it by window class instead."),
                        );
                    }
                    BetterBlurDx::Installed => kit.text(
                        ui,
                        TextRole::Muted,
                        &tr!("Better Blur DX is installed but not switched on. Turn it on in System Settings > Window Management > Desktop Effects (and turn KWin's own Blur off); Vertex then needs nothing else."),
                    ),
                    BetterBlurDx::NotFound => kit.text(
                        ui,
                        TextRole::Muted,
                        &tr!("Linux blur protocols expose only blur regions to clients; Vertex requests a full-window region. Blur strength, radius, saturation, and noise are controlled by the compositor."),
                    ),
                }
                let mut opengl = state.config.linux_set_opengl_driver();
                if kit.toggle_setting(
                ui,
                "settings::linux-opengl",
                &tr!("Set Linux OpenGL Driver"),
                Some(&tr!("Linux-only. Vertex will explicitly manage OpenGL driver environment variables for launched games. This affects all launched versions by default; versions using Vulkan directly should ignore it.")),
                &mut opengl,
            ) {
                state.config.set_linux_set_opengl_driver(opengl);
            }
                let mut zink = state.config.linux_use_zink_driver();
                if kit.toggle_setting_ex(
                ui,
                "settings::linux-zink",
                &tr!("Use Zink Driver (Experimental)"),
                Some(&tr!("Linux-only. Experimental. When the setting above is enabled, forces Mesa Zink so OpenGL runs over Vulkan. Disable it to keep Mesa's default OpenGL driver selection. Versions using Vulkan directly should ignore it.")),
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
            &tr!("Windows"),
            &tr!("Windows-specific window composition and compatibility settings."),
            |ui, kit| {
                if let Some(value) = kit.choice_setting(
                ui,
                "settings::windows-backdrop",
                &tr!("Window Backdrop Type"),
                Some(&tr!("Windows DWM system backdrop material used when Window Transparency is Blurred. Auto tries modern backdrops before legacy blur; None requests DWMSBT_NONE.")),
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
                    &tr!("Blur Tint Strength"),
                    Some(&tr!("How strongly the blur is tinted with the colour below. 0 is no tint. Only the Legacy Blur and Legacy Acrylic backdrops can be tinted.")),
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
                        &tr!("Blur Tint Colour"),
                        Some(&tr!("The tint colour as #rrggbb. Ignored until it is a complete colour.")),
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
                &tr!("Applied only when Window Transparency is Blurred. Mica, Mica Alt and the system Acrylic are drawn by Windows with no strength or tint control; the two Legacy backdrops accept a tint."),
            );
            },
        );
    } else if cfg!(target_os = "macos") {
        section(
            ui,
            kit,
            "macos",
            &tr!("macOS"),
            &tr!("macOS-specific settings."),
            |ui, kit| {
                if let Some(value) = kit.choice_setting(
                    ui,
                    "settings::mac-material",
                    &tr!("Visual Effect Material"),
                    Some(&tr!("NSVisualEffectView material used when macOS native blur is active.")),
                    state.config.macos_visual_effect_material(),
                    &MacosVisualEffectMaterial::ALL,
                    |value| value.label().to_owned(),
                ) {
                    state.config.set_macos_visual_effect_material(value);
                }
                if let Some(value) = kit.choice_setting(
                    ui,
                    "settings::mac-blending",
                    &tr!("Visual Effect Blending Mode"),
                    Some(&tr!("NSVisualEffectView blending mode used when macOS native blur is active.")),
                    state.config.macos_visual_effect_blending_mode(),
                    &MacosVisualEffectBlendingMode::ALL,
                    |value| value.label().to_owned(),
                ) {
                    state.config.set_macos_visual_effect_blending_mode(value);
                }
                if let Some(value) = kit.choice_setting(
                    ui,
                    "settings::mac-state",
                    &tr!("Visual Effect State"),
                    Some(&tr!("NSVisualEffectView state used when macOS native blur is active.")),
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
                &tr!("Visual Effect Emphasized"),
                Some(&tr!("Controls NSVisualEffectView.isEmphasized when macOS native blur is active.")),
                &mut emphasized,
            ) {
                state.config.set_macos_visual_effect_emphasized(emphasized);
            }
                kit.text(
                    ui,
                    TextRole::Muted,
                    &tr!("Applied only when Window Transparency is Blurred."),
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
        &tr!("Info"),
        &tr!("Runtime and hardware information for this installation."),
        |ui, kit| {
            let Some(info) = super::super::super::app_metadata::try_settings_info() else {
                kit.text(ui, TextRole::Muted, &tr!("Collecting system information…"));
                return;
            };
            for (label, value) in [
                (tr!("CPU"), info.cpu.as_str()),
                (tr!("GPU"), info.gpu.as_str()),
                (tr!("Memory"), info.memory.as_str()),
                (tr!("Graphics API"), info.graphics_api.as_str()),
                (tr!("Graphics Driver + Version"), info.graphics_driver.as_str()),
                (tr!("App Version + Commit Hash"), info.app_version.as_str()),
            ] {
                let _row = kit.row(ui, kit::SPACE_LG);
                {
                    let _label = ui.element(
                        &vertex_shell::sturdy::ui::ElementDesc::new()
                            .width(Sizing::Fixed(220.0))
                            .height(Sizing::Fit),
                    );
                    kit.text(ui, TextRole::Strong, &label);
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
            kit.text(ui, TextRole::Heading, &tr!("Use unsecured account storage?"));
            kit::label(
                ui,
                &vertex_i18n::t(INSECURE_STORAGE_WARNING),
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Danger)
                },
            );
            kit::label(
                ui,
                &tr!("Tokens will be written, unencrypted, to {path}. Keep that file private, never share it, and leave it out of backups and screenshots.", path = auth::insecure_token_file_path().display()),
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
                &tr!("Cancel"),
                ButtonKind::Primary,
                true,
            );
            confirmed = kit.button(
                ui,
                "settings-insecure::confirm",
                &tr!("Enable anyway"),
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

#[cfg(test)]
mod tests {
    use crate::app::se5::kit::fuzzy_matches;

    fn fields(list: &[&str]) -> Vec<String> {
        list.iter().map(|field| (*field).to_owned()).collect()
    }

    #[test]
    fn search_finds_substrings_and_near_misses() {
        let fields = fields(&["Graphics & Performance", "Frame Limiter"]);
        assert!(fuzzy_matches("graph", &fields));
        assert!(fuzzy_matches("graphcs", &fields));
        assert!(fuzzy_matches("frame limit", &fields));
        assert!(fuzzy_matches("", &fields));
        assert!(fuzzy_matches("grpahics", &fields));
        assert!(fuzzy_matches("perfromance", &fields));
        assert!(fuzzy_matches("themme", &["Theme".to_owned()]));
        assert!(!fuzzy_matches("them", &["Time Zone".to_owned()]));
        assert!(!fuzzy_matches("zzz", &fields));
    }

    const SECTION_IDS: [&str; 19] = [
        "language", "appearance", "privacy", "notifications", "text", "discord", "sync", "graphics", "minecraft",
        "launching", "logging", "java", "defaults", "linux", "windows", "macos", "displayed-time", "debug", "info",
    ];

    fn every_keyword() -> Vec<String> {
        SECTION_IDS.iter().flat_map(|id| super::section_keywords(id)).collect()
    }

    /// A setting nobody can find by searching is a bug: every row the page draws must be named in the search lists.
    #[test]
    fn every_setting_row_is_searchable() {
        let keywords = every_keyword();
        let source = include_str!("settings.rs");
        let source = &source[..source.find("#[cfg(test)]").unwrap()];
        let mut missing = Vec::new();
        for call in ["choice_setting(", "text_setting(", "toggle_setting(", "slider_setting(", "setting_row("] {
            for (at, _) in source.match_indices(call) {
                if source[..at].ends_with("fn ") || source[..at].ends_with("pub fn ") {
                    continue;
                }
                let rest = &source[at..source.len().min(at + 700)];
                let Some(start) = rest.find("tr!(\"") else { continue };
                let literal = &rest[start + 5..];
                let label = &literal[..literal.find('"').unwrap()];
                // The first message after a call that takes its label from a spec is the tooltip, and a button's
                // label can come first; neither is a setting's name.
                if label.ends_with('.') || label.ends_with('…') {
                    continue;
                }
                if !keywords.iter().any(|keyword| keyword == label) {
                    missing.push(label.to_owned());
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "settings missing from section_keywords: {missing:?}");
    }

    #[test]
    fn every_config_setting_spec_is_searchable() {
        let keywords = every_keyword();
        let mut config = config::Config::default();
        let mut missing = Vec::new();
        config.for_each_toggle_mut(|spec, _| missing.push(spec.label));
        config.for_each_float_mut(|spec, _| missing.push(spec.label));
        config.for_each_int_mut(|spec, _| missing.push(spec.label));
        config.for_each_text_mut(|spec, _| missing.push(spec.label));
        config.for_each_dropdown_mut(|spec, _| missing.push(spec.label));
        // Not drawn on the page (the adapter profile choice stands for it), so there is nothing to find.
        missing.retain(|label| *label != config::ToggleSettingId::LowPowerGpuPreferred.spec().label);
        missing.retain(|label| !keywords.iter().any(|keyword| keyword == label));
        assert!(missing.is_empty(), "setting specs missing from section_keywords: {missing:?}");
    }

    #[test]
    fn language_is_found_by_its_name_in_any_language() {
        let names = vertex_i18n::translations_everywhere("Language");
        assert!(names.iter().any(|name| name == "Language"));
    }
}
