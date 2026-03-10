use config::{
    Config, ConfigFormat, JavaRuntimeVersion, LoadConfigResult, create_default_config, load_config,
    save_config,
};
use eframe::{self, egui};
use egui::CentralPanel;
use installation::{
    DownloadPolicy, InstallProgress, InstallProgressCallback, InstallStage, ensure_game_files,
    ensure_openjdk_runtime, running_instance_for_account, running_instance_roots,
};
use instances::{
    InstanceRecord, InstanceStore, create_instance, instance_root_path, load_store,
    save_store as save_instance_store,
};
use launcher_runtime as tokio_runtime;
use launcher_ui::{console, install_activity, notification, screens, ui, window_effects};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use textui::TextUi;

use self::auth_state::{AuthState, REPAINT_INTERVAL};
use self::config_format_modal::ModalAction;
use self::fonts::FontController;

mod app_icon;
mod auth_state;
mod config_format_modal;
mod create_instance_modal;
mod fonts;
mod native_options;
mod taskbar_progress;
mod tracing_setup;
mod webview_sign_in;

struct VertexApp {
    fonts: FontController,
    config: Config,
    theme_catalog: ui::theme::ThemeCatalog,
    theme: ui::theme::Theme,
    show_config_format_modal: bool,
    selected_config_format: ConfigFormat,
    default_config_format: ConfigFormat,
    config_creation_error: Option<String>,
    active_screen: screens::AppScreen,
    instance_shortcuts: Vec<ui::sidebar::ProfileShortcut>,
    selected_instance_id: Option<String>,
    instance_store: InstanceStore,
    show_create_instance_modal: bool,
    create_instance_state: create_instance_modal::CreateInstanceState,
    auth: AuthState,
    text_ui: TextUi,
    last_frame_end: Option<Instant>,
    last_rendered_screen: Option<screens::AppScreen>,
}

impl VertexApp {
    fn new(cc: &eframe::CreationContext<'_>, config_state: LoadConfigResult) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let (
            mut config,
            config_loaded_from_disk,
            show_config_format_modal,
            selected_config_format,
            default_config_format,
        ) = match config_state {
            LoadConfigResult::Loaded(config) => {
                (config, true, false, ConfigFormat::Json, ConfigFormat::Json)
            }
            LoadConfigResult::Missing { default_format } => (
                Config::default(),
                false,
                true,
                default_format,
                default_format,
            ),
        };

        config.normalize();
        if let Err(error) = window_effects::apply(cc, config.window_blur_enabled()) {
            config.set_window_blur_enabled(false);
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::Transparent(false));
            notification::warn!(
                "window_blur",
                "Window blur is unsupported here and has been disabled. Restart may be required to fully apply the change. {error}"
            );
            if config_loaded_from_disk && let Err(save_error) = save_config(&config) {
                notification::warn!(
                    "config",
                    "Failed to persist disabled blur setting after unsupported platform check: {save_error}"
                );
            }
        }

        let theme_catalog = ui::theme::ThemeCatalog::load();
        if !theme_catalog.contains(config.theme_id()) {
            config.set_theme_id(theme_catalog.default_theme_id().to_owned());
        }
        let theme = theme_catalog.resolve(config.theme_id()).clone();

        let mut text_ui = TextUi::new();
        FontController::register_included_fonts(&mut text_ui);

        let instance_store = match load_store() {
            Ok(store) => store,
            Err(err) => {
                notification::error!("instance_store", "Failed to load instance store: {err}");
                InstanceStore::default()
            }
        };

        let mut app = Self {
            fonts: FontController::new(config.ui_font_family()),
            config,
            theme_catalog,
            theme,
            show_config_format_modal,
            selected_config_format,
            default_config_format,
            config_creation_error: None,
            active_screen: screens::AppScreen::Library,
            instance_shortcuts: Vec::new(),
            selected_instance_id: None,
            instance_store,
            show_create_instance_modal: false,
            create_instance_state: create_instance_modal::CreateInstanceState::default(),
            auth: AuthState::load(),
            text_ui,
            last_frame_end: None,
            last_rendered_screen: None,
        };

        app.refresh_instance_shortcuts();
        if let Some(first) = app.instance_shortcuts.first() {
            app.selected_instance_id = Some(first.id.clone());
        }

        app.fonts.ensure_selected_font_is_available(&mut app.config);
        app.fonts
            .apply_from_config(&cc.egui_ctx, &app.config, &mut app.text_ui);
        app
    }

    fn create_config_with_choice(&mut self, choice: ConfigFormat) {
        match create_default_config(choice) {
            Ok(config) => {
                self.config = config;
                self.config.normalize();
                self.fonts
                    .ensure_selected_font_is_available(&mut self.config);
                self.show_config_format_modal = false;
                self.config_creation_error = None;
            }
            Err(err) => {
                self.config_creation_error = Some(format!("Failed to create config: {err}"));
            }
        }
    }

    fn sync_theme_from_config(&mut self) {
        if !self.theme_catalog.contains(self.config.theme_id()) {
            self.config
                .set_theme_id(self.theme_catalog.default_theme_id().to_owned());
        }

        let resolved = self.theme_catalog.resolve(self.config.theme_id());
        if self.theme.id != resolved.id {
            self.theme = resolved.clone();
        }
    }

    fn refresh_instance_shortcuts(&mut self) {
        self.instance_shortcuts = self
            .instance_store
            .instances
            .iter()
            .map(|instance| ui::sidebar::ProfileShortcut {
                id: instance.id.clone(),
                name: instance.name.clone(),
            })
            .collect();
        tracing::info!(
            target: "vertexlauncher/app/sidebar",
            count = self.instance_shortcuts.len(),
            "Refreshed sidebar instance shortcuts."
        );
    }

    fn apply_frame_limiter(&mut self) {
        if !self.config.frame_limiter_enabled() {
            self.last_frame_end = None;
            return;
        }

        let fps = self.config.frame_limit_fps().clamp(30, 240) as u32;
        let frame_time = Duration::from_secs_f64(1.0 / fps as f64);
        let now = Instant::now();
        if let Some(last) = self.last_frame_end {
            let elapsed = now.saturating_duration_since(last);
            if elapsed < frame_time {
                let remaining = frame_time - elapsed;
                sleep_precise(remaining);
            }
        }
        self.last_frame_end = Some(Instant::now());
    }
}

fn sleep_precise(duration: Duration) {
    let coarse = Duration::from_millis(1);
    let tail = Duration::from_micros(250);
    if duration > coarse + tail {
        std::thread::sleep(duration - tail);
    }
    let deadline = Instant::now() + tail.min(duration);
    while Instant::now() < deadline {
        std::hint::spin_loop();
        std::thread::yield_now();
    }
}

impl eframe::App for VertexApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.apply_frame_limiter();
        self.text_ui.begin_frame(ctx);
        self.auth.poll();
        console::prune_instance_tabs(&running_instance_roots());
        apply_install_activity_os_feedback(ctx, frame);
        if self.auth.should_request_repaint() {
            ctx.request_repaint_after(REPAINT_INTERVAL);
        }

        let previous_config = self.config.clone();
        let previous_instance_store = self.instance_store.clone();
        self.sync_theme_from_config();
        self.theme.apply(ctx, self.config.window_blur_enabled());
        self.auth
            .set_streamer_mode(self.config.streamer_mode_enabled());
        self.fonts
            .ensure_selected_font_is_available(&mut self.config);
        self.fonts
            .apply_from_config(ctx, &self.config, &mut self.text_ui);

        let account_entries = self.auth.account_entries();
        let profile_accounts = account_entries
            .iter()
            .map(|entry| ui::top_bar::ProfileAccountOption {
                profile_id: entry.profile_id.clone(),
                display_name: entry.display_name.clone(),
                is_active: entry.is_active,
            })
            .collect::<Vec<_>>();
        let account_avatars_by_key = account_entries
            .into_iter()
            .filter_map(|entry| {
                entry
                    .avatar_png
                    .map(|avatar| (entry.profile_id.to_ascii_lowercase(), avatar))
            })
            .collect::<HashMap<_, _>>();
        let streamer_mode = self.config.streamer_mode_enabled();
        let account_avatars_by_key = if streamer_mode {
            HashMap::new()
        } else {
            account_avatars_by_key
        };
        let active_launch_auth =
            self.auth
                .active_launch_context()
                .map(|context| screens::LaunchAuthContext {
                    account_key: context.account_key,
                    player_name: context.player_name,
                    player_uuid: context.player_uuid,
                    access_token: context.access_token,
                    xuid: context.xuid,
                    user_type: context.user_type,
                });
        let user_instance_active = active_launch_auth.as_ref().is_some_and(|context| {
            running_instance_for_account(context.account_key.as_str()).is_some()
        });

        let top_bar_section_label = if self.active_screen == screens::AppScreen::Instance {
            self.selected_instance_id
                .as_deref()
                .and_then(|id| self.instance_store.find(id))
                .map(|instance| instance.name.clone())
                .unwrap_or_else(|| self.active_screen.label().to_owned())
        } else {
            self.active_screen.label().to_owned()
        };

        let top_bar_output = ui::top_bar::render(
            ctx,
            top_bar_section_label.as_str(),
            &mut self.text_ui,
            ui::top_bar::ProfileUiModel {
                display_name: self.auth.display_name(),
                avatar_png: if streamer_mode {
                    None
                } else {
                    self.auth.avatar_png()
                },
                sign_in_in_progress: self.auth.sign_in_in_progress(),
                auth_busy: self.auth.auth_busy(),
                token_refresh_in_progress: self.auth.token_refresh_in_progress(),
                streamer_mode,
                status_message: self.auth.status_message(),
                accounts: &profile_accounts,
                user_instance_active,
            },
        );
        if self.active_screen != screens::AppScreen::Instance {
            notification::render_popups(ctx, &mut self.text_ui);
        }

        if top_bar_output.start_sign_in {
            self.auth.start_sign_in();
        }
        let mut account_switched = false;
        if let Some(profile_id) = top_bar_output.select_account_id.as_deref() {
            self.auth.select_account(profile_id);
            account_switched = true;
        }
        if let Some(profile_id) = top_bar_output.remove_account_id.as_deref() {
            self.auth.remove_account(profile_id);
        }
        if top_bar_output.open_active_user_terminal {
            self.active_screen = screens::AppScreen::Console;
            let active_launch_auth =
                self.auth
                    .active_launch_context()
                    .map(|context| screens::LaunchAuthContext {
                        account_key: context.account_key,
                        player_name: context.player_name,
                        player_uuid: context.player_uuid,
                        access_token: context.access_token,
                        xuid: context.xuid,
                        user_type: context.user_type,
                    });
            let _ = console::activate_tab_for_user(
                active_launch_auth
                    .as_ref()
                    .map(|context| context.account_key.as_str()),
                self.auth.display_name(),
            );
        }

        let sidebar_output = ui::sidebar::render(ctx, self.active_screen, &self.instance_shortcuts);

        if let Some(next_screen) = sidebar_output.selected_screen {
            self.active_screen = next_screen;
        }
        if let Some(instance_id) = sidebar_output.selected_profile_id {
            self.selected_instance_id = Some(instance_id);
            self.active_screen = screens::AppScreen::Instance;
        }
        if sidebar_output.create_instance_clicked {
            self.show_create_instance_modal = true;
            self.create_instance_state.error = None;
        }

        let mut screen_output = screens::ScreenOutput::default();
        let wgpu_target_format = frame.wgpu_render_state().map(|state| state.target_format);
        let skin_preview_msaa_samples = 4;
        let skin_manager_opened = self.active_screen == screens::AppScreen::Skins
            && self.last_rendered_screen != Some(screens::AppScreen::Skins);
        let skin_manager_account_switched =
            self.active_screen == screens::AppScreen::Skins && account_switched;
        let active_launch_auth =
            self.auth
                .active_launch_context()
                .map(|context| screens::LaunchAuthContext {
                    account_key: context.account_key,
                    player_name: context.player_name,
                    player_uuid: context.player_uuid,
                    access_token: context.access_token,
                    xuid: context.xuid,
                    user_type: context.user_type,
                });
        CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::ZERO)
                    .outer_margin(egui::Margin::ZERO)
                    .stroke(egui::Stroke::new(
                        1.0,
                        ctx.style().visuals.widgets.noninteractive.bg_stroke.color,
                    )),
            )
            .show(ctx, |ui| {
                screen_output = screens::render(
                    ui,
                    self.active_screen,
                    skin_manager_opened,
                    skin_manager_account_switched,
                    self.selected_instance_id.as_deref(),
                    self.auth.display_name(),
                    active_launch_auth.as_ref(),
                    self.auth.active_account_owns_minecraft(),
                    streamer_mode,
                    &mut self.config,
                    &mut self.instance_store,
                    &account_avatars_by_key,
                    wgpu_target_format,
                    skin_preview_msaa_samples,
                    self.fonts.available_ui_fonts(),
                    self.theme_catalog.themes(),
                    &mut self.text_ui,
                );
            });
        self.last_rendered_screen = Some(self.active_screen);

        if screen_output.instances_changed {
            self.refresh_instance_shortcuts();
        }
        if let Some(instance_id) = screen_output.selected_instance_id {
            self.selected_instance_id = Some(instance_id);
        }
        if let Some(requested_screen) = screen_output.requested_screen {
            self.active_screen = requested_screen;
        }

        if self.show_config_format_modal {
            match config_format_modal::render(
                ctx,
                &mut self.text_ui,
                &mut self.selected_config_format,
                self.config_creation_error.as_deref(),
            ) {
                ModalAction::None => {}
                ModalAction::Cancel => self.create_config_with_choice(self.default_config_format),
                ModalAction::Create(choice) => self.create_config_with_choice(choice),
            }
        }

        if self.show_create_instance_modal {
            match create_instance_modal::render(
                ctx,
                &mut self.text_ui,
                &mut self.create_instance_state,
                self.config.include_snapshots_and_betas(),
            ) {
                create_instance_modal::ModalAction::None => {}
                create_instance_modal::ModalAction::Cancel => {
                    self.show_create_instance_modal = false;
                    self.create_instance_state.reset();
                }
                create_instance_modal::ModalAction::Create(draft) => {
                    let installations_root =
                        PathBuf::from(self.config.minecraft_installations_root());
                    match create_instance(
                        &mut self.instance_store,
                        &installations_root,
                        draft.into_new_instance_spec(),
                    ) {
                        Ok(instance) => {
                            start_initial_instance_install(
                                &instance,
                                installations_root.as_path(),
                                &self.config,
                            );
                            self.selected_instance_id = Some(instance.id);
                            self.active_screen = screens::AppScreen::Instance;
                            self.show_create_instance_modal = false;
                            self.create_instance_state.reset();
                            self.refresh_instance_shortcuts();
                        }
                        Err(err) => {
                            self.create_instance_state.error =
                                Some(format!("Failed to create instance: {err}"));
                        }
                    }
                }
            }
        }

        self.config.normalize();
        self.fonts
            .ensure_selected_font_is_available(&mut self.config);
        if self.config != previous_config {
            if let Err(err) = save_config(&self.config) {
                tracing::error!(target: "vertexlauncher/app/config", "Failed to save config: {err}");
            }
            self.fonts
                .apply_from_config(ctx, &self.config, &mut self.text_ui);
        }

        self.instance_store.normalize();
        if self.instance_store != previous_instance_store {
            if self
                .selected_instance_id
                .as_deref()
                .is_some_and(|id| self.instance_store.find(id).is_none())
            {
                self.selected_instance_id =
                    self.instance_store.instances.first().map(|i| i.id.clone());
                if self.selected_instance_id.is_none()
                    && self.active_screen == screens::AppScreen::Instance
                {
                    self.active_screen = screens::AppScreen::Library;
                }
            }
            self.refresh_instance_shortcuts();
            if let Err(err) = save_instance_store(&self.instance_store) {
                tracing::error!(target: "vertexlauncher/app/instances", "Failed to save instances: {err}");
            }
        }

        ui::top_bar::handle_window_resize(ctx);
    }
}

fn apply_install_activity_os_feedback(ctx: &egui::Context, frame: &eframe::Frame) {
    if let Some(activity) = install_activity::snapshot() {
        let fraction = if activity.total_files > 0 {
            (activity.downloaded_files as f32 / activity.total_files as f32).clamp(0.0, 1.0)
        } else if let Some(total) = activity.total_bytes {
            if total > 0 {
                (activity.downloaded_bytes as f32 / total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            }
        } else {
            0.0
        };
        let percent = (fraction * 100.0).round() as u32;
        let speed_mib = activity.bytes_per_second / (1024.0 * 1024.0);
        let eta_suffix = activity
            .eta_seconds
            .map(|eta| format!(" ETA {}s", eta))
            .unwrap_or_default();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "Vertex Launcher · Installing {}% · {:.1} MiB/s{}",
            percent, speed_mib, eta_suffix
        )));
        ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::Progress);
        taskbar_progress::set_install_progress(frame, Some(fraction));
        return;
    }

    ctx.send_viewport_cmd(egui::ViewportCommand::Title("Vertex Launcher".to_owned()));
    taskbar_progress::set_install_progress(frame, None);
}

fn start_initial_instance_install(
    instance: &InstanceRecord,
    installations_root: &Path,
    config: &Config,
) {
    let instance_name = instance.name.clone();
    let activity_instance = instance_name.clone();
    let game_version = instance.game_version.trim().to_owned();
    let modloader = instance.modloader.trim().to_owned();
    let modloader_version = {
        let trimmed = instance.modloader_version.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    };
    if game_version.is_empty() || modloader.is_empty() {
        return;
    }

    let instance_root = instance_root_path(installations_root, instance);
    let download_policy = DownloadPolicy {
        max_concurrent_downloads: config.download_max_concurrent().max(1),
        max_download_bps: config.parsed_download_speed_limit_bps(),
    };
    let java_8 = config
        .java_runtime_path(JavaRuntimeVersion::Java8)
        .map(str::to_owned);
    let java_16 = config
        .java_runtime_path(JavaRuntimeVersion::Java16)
        .map(str::to_owned);
    let java_17 = config
        .java_runtime_path(JavaRuntimeVersion::Java17)
        .map(str::to_owned);
    let java_21 = config
        .java_runtime_path(JavaRuntimeVersion::Java21)
        .map(str::to_owned);

    let notification_source = format!("installation/{instance_name}");
    install_activity::set_progress(
        activity_instance.as_str(),
        &InstallProgress {
            stage: InstallStage::PreparingFolders,
            message: format!(
                "Starting installation for Minecraft {} ({})...",
                game_version, modloader
            ),
            downloaded_files: 0,
            total_files: 0,
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: None,
        },
    );
    notification::progress!(
        notification::Severity::Info,
        notification_source.clone(),
        0.0f32,
        "Starting initial install: Minecraft {} / {}.",
        game_version,
        modloader
    );

    let _ = tokio_runtime::spawn(async move {
        let last_emit = Arc::new(Mutex::new(
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        ));
        let notification_source_for_progress = notification_source.clone();
        let activity_instance_for_progress = activity_instance.clone();
        let result = tokio_runtime::spawn_blocking(move || {
            let progress_callback: InstallProgressCallback = {
                let last_emit = Arc::clone(&last_emit);
                Arc::new(move |progress: InstallProgress| {
                    install_activity::set_progress(
                        activity_instance_for_progress.as_str(),
                        &progress,
                    );
                    let should_emit = if let Ok(mut last) = last_emit.lock() {
                        if last.elapsed() >= std::time::Duration::from_millis(250) {
                            *last = std::time::Instant::now();
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !should_emit {
                        return;
                    }
                    let fraction = if progress.total_files > 0 {
                        (progress.downloaded_files as f32 / progress.total_files as f32)
                            .clamp(0.0, 1.0)
                    } else if let Some(total) = progress.total_bytes {
                        if total > 0 {
                            (progress.downloaded_bytes as f32 / total as f32).clamp(0.0, 1.0)
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    };
                    notification::progress!(
                        notification::Severity::Info,
                        notification_source_for_progress.clone(),
                        fraction,
                        "{} · {:.1} MiB/s{}",
                        progress.message,
                        progress.bytes_per_second / (1024.0 * 1024.0),
                        progress
                            .eta_seconds
                            .map(|eta| format!(" · ETA {}s", eta))
                            .unwrap_or_default()
                    );
                })
            };
            let runtime = recommended_java_runtime_for_game(game_version.as_str());
            let configured_java = runtime.and_then(|runtime| match runtime {
                JavaRuntimeVersion::Java8 => java_8.as_deref(),
                JavaRuntimeVersion::Java16 => java_16.as_deref(),
                JavaRuntimeVersion::Java17 => java_17.as_deref(),
                JavaRuntimeVersion::Java21 => java_21.as_deref(),
            });
            let java_path = configured_java
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .filter(|value| Path::new(value).exists())
                .map(str::to_owned)
                .or_else(|| {
                    runtime.and_then(|runtime| {
                        ensure_openjdk_runtime(runtime.major())
                            .ok()
                            .map(|path| path.display().to_string())
                    })
                })
                .unwrap_or_else(|| "java".to_owned());

            ensure_game_files(
                instance_root.as_path(),
                game_version.as_str(),
                modloader.as_str(),
                modloader_version.as_deref(),
                Some(java_path.as_str()),
                &download_policy,
                Some(progress_callback),
            )
            .map_err(|err| err.to_string())
        })
        .await
        .map_err(|err| format!("initial install task join error: {err}"))
        .and_then(|inner| inner);

        match result {
            Ok(setup) => {
                install_activity::clear_instance(activity_instance.as_str());
                notification::progress!(
                    notification::Severity::Info,
                    notification_source,
                    1.0f32,
                    "Initial install complete ({} files, loader {}).",
                    setup.downloaded_files,
                    setup.resolved_modloader_version.as_deref().unwrap_or("n/a")
                );
            }
            Err(err) => {
                install_activity::clear_instance(activity_instance.as_str());
                notification::error!(
                    notification_source,
                    "{}: initial install failed: {}",
                    instance_name,
                    err
                );
            }
        }
    });
}

fn recommended_java_runtime_for_game(game_version: &str) -> Option<JavaRuntimeVersion> {
    let mut parts = game_version
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next().unwrap_or(0);

    if major != 1 {
        return Some(JavaRuntimeVersion::Java21);
    }
    if minor <= 16 {
        return Some(JavaRuntimeVersion::Java8);
    }
    if minor == 17 {
        return Some(JavaRuntimeVersion::Java16);
    }
    if minor > 20 || (minor == 20 && patch >= 5) {
        return Some(JavaRuntimeVersion::Java21);
    }
    Some(JavaRuntimeVersion::Java17)
}

pub fn run() -> eframe::Result<()> {
    let log_path = tracing_setup::init_tracing();
    if let Some(log_path) = log_path.as_deref() {
        tracing::info!(
            target: "vertexlauncher/app/startup",
            "Launcher started. Log file: {}",
            log_path.display()
        );
    } else {
        tracing::info!(
            target: "vertexlauncher/app/startup",
            "Launcher started. File logging unavailable; using stderr/console only."
        );
    }
    launcher_runtime::init();
    #[cfg(target_os = "macos")]
    app_icon::apply_macos_dock_icon();
    let config_state = load_config();
    let startup_config = match &config_state {
        LoadConfigResult::Loaded(config) => config.clone(),
        LoadConfigResult::Missing { .. } => Config::default(),
    };

    let options = native_options::build(&startup_config);

    eframe::run_native(
        "Vertex Launcher",
        options,
        Box::new(move |cc| Ok(Box::new(VertexApp::new(cc, config_state)))),
    )
}

pub fn maybe_run_webview_helper() -> Result<bool, String> {
    webview_sign_in::maybe_run_helper_from_args()
}
