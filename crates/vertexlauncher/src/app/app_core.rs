use super::*;

pub(super) const DISCOVER_STATE_PURGE_DELAY: Duration = Duration::from_secs(4);

#[derive(Default)]
pub(super) struct ScreenPurgeTimer {
    pub(super) inactive_since: Option<Instant>,
    pub(super) purged: bool,
}

#[derive(Debug)]
pub enum RunError {
    RuntimeBootstrap(launcher_runtime::RuntimeBootstrapError),
    Ui(eframe::Error),
}

pub fn init_tracing() -> Option<PathBuf> {
    launcher_runtime::set_detached_task_reporter(report_detached_task_failure);
    tracing_setup::init_tracing()
}

pub(super) fn report_detached_task_failure(task_kind: &str, error: &launcher_runtime::TaskError) {
    notification::emit_replace(
        notification::Severity::Error,
        "background-task",
        format!("A background task failed ({task_kind}): {error}"),
        format!("background-task/{task_kind}"),
    );
}

pub(super) fn is_discover_screen(screen: screens::AppScreen) -> bool {
    matches!(
        screen,
        screens::AppScreen::Discover | screens::AppScreen::DiscoverDetail
    )
}

pub(super) fn update_screen_purge_timer(
    ctx: &egui::Context,
    timer: &mut ScreenPurgeTimer,
    is_active: bool,
    purge_delay: Duration,
    purge: impl FnOnce(),
) {
    if is_active {
        timer.inactive_since = None;
        timer.purged = false;
        return;
    }

    let inactive_since = timer.inactive_since.get_or_insert_with(Instant::now);
    if timer.purged {
        return;
    }

    let elapsed = inactive_since.elapsed();
    if elapsed >= purge_delay {
        purge();
        timer.purged = true;
    } else {
        ctx.request_repaint_after(purge_delay - elapsed);
    }
}

pub(super) struct VertexApp {
    pub(super) fonts: FontController,
    pub(super) config: Config,
    pub(super) theme_catalog: ui::theme::ThemeCatalog,
    pub(super) theme: ui::theme::Theme,
    pub(super) show_config_format_modal: bool,
    pub(super) selected_config_format: ConfigFormat,
    pub(super) default_config_format: ConfigFormat,
    pub(super) config_creation_error: Option<String>,
    pub(super) active_screen: screens::AppScreen,
    pub(super) instance_shortcuts: Vec<ui::sidebar::ProfileShortcut>,
    pub(super) selected_instance_id: Option<String>,
    pub(super) instance_store: InstanceStore,
    pub(super) content_browser_state: screens::ContentBrowserState,
    pub(super) discover_state: screens::DiscoverState,
    pub(super) home_purge_timer: ScreenPurgeTimer,
    pub(super) library_purge_timer: ScreenPurgeTimer,
    pub(super) instance_purge_timer: ScreenPurgeTimer,
    pub(super) discover_purge_timer: ScreenPurgeTimer,
    pub(super) content_browser_purge_timer: ScreenPurgeTimer,
    pub(super) skins_purge_timer: ScreenPurgeTimer,
    pub(super) show_create_instance_modal: bool,
    pub(super) create_instance_state: create_instance_modal::CreateInstanceState,
    pub(super) show_import_instance_modal: bool,
    pub(super) import_instance_state: import_instance_modal::ImportInstanceState,
    pub(super) show_gamepad_calibration_modal: bool,
    pub(super) gamepad_calibration_state: gamepad_calibration_modal::GamepadCalibrationState,
    pub(super) in_flight_import_request: Option<import_instance_modal::ImportRequest>,
    pub(super) curseforge_manual_download_preflight_request:
        Option<import_instance_modal::ImportRequest>,
    pub(super) curseforge_manual_download_preflight_in_flight: bool,
    pub(super) curseforge_manual_download_preflight_rx: Option<
        mpsc::Receiver<
            Result<Option<Vec<import_instance_modal::CurseForgeManualDownloadRequirement>>, String>,
        >,
    >,
    pub(super) discover_curseforge_manual_download_preflight_request:
        Option<screens::DiscoverInstallRequest>,
    pub(super) discover_curseforge_manual_download_preflight_in_flight: bool,
    pub(super) discover_curseforge_manual_download_preflight_tx: Option<
        mpsc::Sender<
            Result<Option<import_instance_modal::CurseForgeManualDownloadRequirement>, String>,
        >,
    >,
    pub(super) discover_curseforge_manual_download_preflight_rx: Option<
        mpsc::Receiver<
            Result<Option<import_instance_modal::CurseForgeManualDownloadRequirement>, String>,
        >,
    >,
    pub(super) pending_curseforge_manual_download: Option<PendingCurseForgeManualDownloadState>,
    pub(super) discover_install_progress_tx:
        Option<mpsc::Sender<import_instance_modal::ImportProgress>>,
    pub(super) discover_install_progress_rx:
        Option<Arc<Mutex<mpsc::Receiver<import_instance_modal::ImportProgress>>>>,
    pub(super) discover_install_results_tx:
        Option<mpsc::Sender<import_instance_modal::ImportTaskResult>>,
    pub(super) discover_install_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<import_instance_modal::ImportTaskResult>>>>,
    pub(super) auth: AuthState,
    pub(super) startup_graphics: platform::StartupGraphicsConfig,
    pub(super) text_ui: TextUi,
    pub(super) config_save_in_flight: bool,
    pub(super) pending_config_save: Option<Config>,
    pub(super) config_save_results_tx: Option<mpsc::Sender<Result<(), String>>>,
    pub(super) config_save_results_rx: Option<mpsc::Receiver<Result<(), String>>>,
    pub(super) instance_store_save_in_flight: bool,
    pub(super) pending_instance_store_save: Option<InstanceStore>,
    pub(super) instance_store_save_results_tx: Option<mpsc::Sender<Result<(), String>>>,
    pub(super) instance_store_save_results_rx: Option<mpsc::Receiver<Result<(), String>>>,
    pub(super) initial_install_results_tx: Option<mpsc::Sender<InitialInstanceInstallResult>>,
    pub(super) initial_install_results_rx: Option<mpsc::Receiver<InitialInstanceInstallResult>>,
    pub(super) discord_presence: DiscordPresenceManager,
    pub(super) gamepad: Option<gamepad::GamepadNavigator>,
    pub(super) last_frame_end: Option<Instant>,
    pub(super) last_rendered_screen: Option<screens::AppScreen>,
}

#[derive(Debug)]
pub(super) enum InitialInstanceInstallResult {
    Failed {
        instance_id: String,
        instance_name: String,
        error: String,
    },
}

impl VertexApp {
    pub(super) fn new(cc: &eframe::CreationContext<'_>, config_state: LoadConfigResult) -> Self {
        tracing::info!(
            target: "vertexlauncher/app/startup",
            "VertexApp::new entered."
        );
        egui_extras::install_image_loaders(&cc.egui_ctx);
        #[cfg(target_os = "macos")]
        app_icon::apply_macos_dock_icon();

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
        #[cfg(target_os = "macos")]
        if config.window_blur_enabled() {
            disable_window_blur_for_startup(
                cc,
                &mut config,
                config_loaded_from_disk,
                "Window blur is temporarily disabled on macOS to keep launcher startup on the stable path.".to_owned(),
                "macOS safety fallback",
            );
        }
        if let Err(error) = window_effects::apply(
            cc,
            effective_window_blur_enabled(&config),
            config.windows_backdrop_type(),
        ) {
            disable_window_blur_for_startup(
                cc,
                &mut config,
                config_loaded_from_disk,
                format!(
                    "Window blur is unsupported here and has been disabled. Restart may be required to fully apply the change. {error}"
                ),
                "unsupported platform check",
            );
        }

        let theme_catalog = ui::theme::ThemeCatalog::load();
        if !theme_catalog.contains(config.theme_id()) {
            config.set_theme_id(theme_catalog.default_theme_id().to_owned());
        }
        let theme = theme_catalog.resolve(config.theme_id()).clone();
        let startup_graphics = platform::startup_graphics_config(
            effective_window_blur_enabled(&config),
            config.graphics_api_preference(),
        );

        let mut text_ui =
            TextUi::new_with_graphics_config(build_text_graphics_config(&config, startup_graphics));
        let _ =
            textui_adapter::begin_frame(&mut text_ui, &cc.egui_ctx, cc.wgpu_render_state.as_ref());
        FontController::register_included_fonts(&mut text_ui);

        let instance_store = match load_store() {
            Ok(store) => store,
            Err(err) => {
                notification::error!("instance_store", "Failed to load instance store: {err}");
                InstanceStore::default()
            }
        };
        let streamer_mode_enabled = config.streamer_mode_enabled();
        if app_metadata::try_settings_info().is_none() {
            let _ = tokio_runtime::spawn_blocking_detached(app_metadata::preload_settings_info);
        }

        let mut app = Self {
            fonts: FontController::new(config.ui_font_family()),
            config,
            theme_catalog,
            theme,
            show_config_format_modal,
            selected_config_format,
            default_config_format,
            config_creation_error: None,
            active_screen: screens::AppScreen::Home,
            instance_shortcuts: Vec::new(),
            selected_instance_id: None,
            instance_store,
            content_browser_state: screens::ContentBrowserState::default(),
            discover_state: screens::DiscoverState::default(),
            home_purge_timer: ScreenPurgeTimer::default(),
            library_purge_timer: ScreenPurgeTimer::default(),
            instance_purge_timer: ScreenPurgeTimer::default(),
            discover_purge_timer: ScreenPurgeTimer::default(),
            content_browser_purge_timer: ScreenPurgeTimer::default(),
            skins_purge_timer: ScreenPurgeTimer::default(),
            show_create_instance_modal: false,
            create_instance_state: create_instance_modal::CreateInstanceState::default(),
            show_import_instance_modal: false,
            import_instance_state: import_instance_modal::ImportInstanceState::default(),
            show_gamepad_calibration_modal: false,
            gamepad_calibration_state: gamepad_calibration_modal::GamepadCalibrationState::default(
            ),
            in_flight_import_request: None,
            curseforge_manual_download_preflight_request: None,
            curseforge_manual_download_preflight_in_flight: false,
            curseforge_manual_download_preflight_rx: None,
            discover_curseforge_manual_download_preflight_request: None,
            discover_curseforge_manual_download_preflight_in_flight: false,
            discover_curseforge_manual_download_preflight_tx: None,
            discover_curseforge_manual_download_preflight_rx: None,
            pending_curseforge_manual_download: None,
            discover_install_progress_tx: None,
            discover_install_progress_rx: None,
            discover_install_results_tx: None,
            discover_install_results_rx: None,
            auth: AuthState::load(streamer_mode_enabled),
            startup_graphics,
            text_ui,
            config_save_in_flight: false,
            pending_config_save: None,
            config_save_results_tx: None,
            config_save_results_rx: None,
            instance_store_save_in_flight: false,
            pending_instance_store_save: None,
            instance_store_save_results_tx: None,
            instance_store_save_results_rx: None,
            initial_install_results_tx: None,
            initial_install_results_rx: None,
            discord_presence: DiscordPresenceManager::default(),
            gamepad: gamepad::GamepadNavigator::new(),
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

    #[allow(deprecated)]
    pub(super) fn update_inner(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let calibrations = self.config.gamepad_calibrations().clone();
        let gamepad_update = if let Some(gamepad) = &mut self.gamepad {
            gamepad.update(ctx, &calibrations, self.active_screen)
        } else {
            gamepad::GamepadUpdate::default()
        };
        screens::set_skins_gamepad_orbit_input(
            ctx,
            if self.active_screen == screens::AppScreen::Skins {
                gamepad_update.skins_preview_orbit
            } else {
                0.0
            },
        );
        screens::set_home_screenshot_viewer_gamepad_input(
            ctx,
            if self.active_screen == screens::AppScreen::Home {
                gamepad_update.screenshot_viewer_pan
            } else {
                egui::Vec2::ZERO
            },
            if self.active_screen == screens::AppScreen::Home {
                gamepad_update.screenshot_viewer_zoom
            } else {
                0.0
            },
        );
        screens::set_instance_screenshot_viewer_gamepad_input(
            ctx,
            if self.active_screen == screens::AppScreen::Instance {
                gamepad_update.screenshot_viewer_pan
            } else {
                egui::Vec2::ZERO
            },
            if self.active_screen == screens::AppScreen::Instance {
                gamepad_update.screenshot_viewer_zoom
            } else {
                0.0
            },
        );
        if let Some(device) = gamepad_update.calibration_requested
            && !self.show_gamepad_calibration_modal
        {
            self.show_gamepad_calibration_modal = true;
            self.gamepad_calibration_state.start(device);
        }
        self.apply_frame_limiter();
        self.text_ui.set_graphics_config(build_text_graphics_config(
            &self.config,
            self.startup_graphics,
        ));
        let _ = textui_adapter::begin_frame(&mut self.text_ui, ctx, frame.wgpu_render_state());
        launcher_ui::ui::components::image_textures::begin_frame(ctx);
        poll_config_save_results(self);
        poll_instance_store_save_results(self);
        self.auth.poll();
        poll_finished_instance_process_notifications(self);
        console::prune_instance_tabs(&running_instance_roots());
        apply_install_activity_os_feedback(ctx, frame);
        if self.auth.should_request_repaint() {
            ctx.request_repaint_after(REPAINT_INTERVAL);
        }

        let previous_config = self.config.clone();
        let previous_instance_store = self.instance_store.clone();
        poll_create_instance_result(self);
        poll_curseforge_manual_download_preflight(self);
        poll_discover_curseforge_manual_download_preflight(self);
        poll_import_instance_progress(self);
        poll_import_instance_result(self);
        poll_pending_curseforge_manual_download(self);
        poll_discover_install_progress(self);
        poll_discover_install_result(self);
        poll_initial_instance_install_results(self);
        self.sync_theme_from_config();
        self.theme
            .apply(ctx, effective_ui_opacity_percent(&self.config));
        self.auth
            .set_streamer_mode(self.config.streamer_mode_enabled());
        notification::set_streamer_mode(self.config.streamer_mode_enabled());
        self.fonts
            .ensure_selected_font_is_available(&mut self.config);
        self.fonts
            .apply_from_config(ctx, &self.config, &mut self.text_ui);
        let _ = self.handle_escape(ctx);

        let account_entries = self.auth.account_entries();
        let profile_accounts = account_entries
            .iter()
            .map(|entry| ui::top_bar::ProfileAccountOption {
                profile_id: entry.profile_id.clone(),
                display_name: entry.display_name.clone(),
                is_active: entry.is_active,
                is_failed: entry.is_failed,
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
                avatar_png: self.auth.avatar_png(),
                sign_in_in_progress: self.auth.sign_in_in_progress(),
                auth_busy: self.auth.auth_busy(),
                token_refresh_in_progress: self.auth.token_refresh_in_progress(),
                streamer_mode,
                status_message: self.auth.status_message(),
                accounts: &profile_accounts,
                user_instance_active,
                device_code_prompt: self.auth.device_code_prompt(),
            },
        );
        let suppressed_progress_source = if self.active_screen == screens::AppScreen::Instance {
            self.selected_instance_id
                .as_deref()
                .and_then(|id| self.instance_store.find(id))
                .map(|instance| format!("installation/{}", instance.name))
        } else {
            None
        };
        notification::render_popups(
            ctx,
            &mut self.text_ui,
            self.config.notification_expiry_bars_empty_left(),
            suppressed_progress_source.as_deref(),
        );

        if top_bar_output.start_webview_sign_in {
            self.auth.start_webview_sign_in();
        }
        if top_bar_output.start_device_code_sign_in {
            self.auth.start_device_code_sign_in();
        }
        if top_bar_output.cancel_device_code_sign_in {
            self.auth.cancel_device_code_sign_in();
        }
        if top_bar_output.open_device_code_browser {
            self.auth.start_system_browser_sign_in(&self.theme);
        }
        let previous_active_screen = self.active_screen;
        if self.active_screen == screens::AppScreen::Settings
            && self.last_rendered_screen != Some(screens::AppScreen::Settings)
        {
            if let Some(focused_id) = ctx.memory(|memory| memory.focused()) {
                ctx.memory_mut(|memory| memory.surrender_focus(focused_id));
            }
            screens::request_settings_theme_focus(ctx);
        }
        let previous_selected_instance_id = self.selected_instance_id.clone();
        let mut account_switched = false;
        if let Some(profile_id) = top_bar_output.select_account_id.as_deref() {
            self.auth.select_account(profile_id);
            account_switched = true;
        }
        if let Some(profile_id) = top_bar_output.remove_account_id.as_deref() {
            self.auth.remove_account(profile_id);
        }
        if let Some(profile_id) = top_bar_output.refresh_account_id.as_deref() {
            self.auth.refresh_account_token(profile_id);
        }
        if top_bar_output.open_active_user_terminal {
            tracing::info!(
                target: "vertexlauncher/navigation",
                from_screen = ?self.active_screen,
                to_screen = ?screens::AppScreen::Console,
                "Top bar opened the active user terminal."
            );
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

        svg_aa::set_svg_aa_mode(self.config.svg_aa_mode());

        let sidebar_output = ui::sidebar::render(
            ctx,
            self.active_screen,
            &self.instance_shortcuts,
            &mut self.text_ui,
        );

        if let Some(next_screen) = sidebar_output.selected_screen {
            tracing::info!(
                target: "vertexlauncher/navigation",
                from_screen = ?self.active_screen,
                to_screen = ?next_screen,
                "Sidebar selected a screen."
            );
            self.active_screen = next_screen;
        }
        if let Some(instance_id) = sidebar_output.selected_profile_id {
            tracing::info!(
                target: "vertexlauncher/navigation",
                instance_id = %instance_id,
                "Sidebar selected an instance profile."
            );
            self.selected_instance_id = Some(instance_id);
            self.active_screen = screens::AppScreen::Instance;
        }
        for (instance_id, action) in sidebar_output.instance_context_actions {
            match action {
                InstanceContextAction::OpenInstance => {
                    tracing::info!(
                        target: "vertexlauncher/navigation",
                        instance_id = %instance_id,
                        "Instance context menu opened an instance."
                    );
                    self.selected_instance_id = Some(instance_id);
                    self.active_screen = screens::AppScreen::Instance;
                }
                InstanceContextAction::OpenFolder => {
                    self.open_instance_folder(&instance_id);
                }
                InstanceContextAction::CopyLaunchCommand => {
                    let active_launch_auth = self.auth.active_launch_context().map(|context| {
                        screens::LaunchAuthContext {
                            account_key: context.account_key,
                            player_name: context.player_name,
                            player_uuid: context.player_uuid,
                            access_token: context.access_token,
                            xuid: context.xuid,
                            user_type: context.user_type,
                        }
                    });
                    let active_username = self.auth.display_name();
                    if let Some(user) = screens::selected_quick_launch_user(
                        active_username,
                        active_launch_auth.as_ref(),
                    ) {
                        let command = screens::build_quick_launch_command(
                            screens::QuickLaunchCommandMode::Pack,
                            instance_id.as_str(),
                            user.as_str(),
                            None,
                            None,
                        );
                        ctx.copy_text(command);
                        notification::info!(
                            "sidebar/quick_launch",
                            "Copied instance command line to clipboard."
                        );
                    } else {
                        notification::warn!(
                            "sidebar/quick_launch",
                            "Sign in before copying an instance command line."
                        );
                    }
                }
                InstanceContextAction::CopySteamLaunchOptions => {
                    let active_launch_auth = self.auth.active_launch_context().map(|context| {
                        screens::LaunchAuthContext {
                            account_key: context.account_key,
                            player_name: context.player_name,
                            player_uuid: context.player_uuid,
                            access_token: context.access_token,
                            xuid: context.xuid,
                            user_type: context.user_type,
                        }
                    });
                    let active_username = self.auth.display_name();
                    if let Some(user) = screens::selected_quick_launch_user(
                        active_username,
                        active_launch_auth.as_ref(),
                    ) {
                        let options = screens::build_quick_launch_steam_options(
                            screens::QuickLaunchCommandMode::Pack,
                            instance_id.as_str(),
                            user.as_str(),
                            None,
                            None,
                        );
                        ctx.copy_text(options);
                        notification::info!(
                            "sidebar/quick_launch",
                            "Copied Steam launch options to clipboard."
                        );
                    } else {
                        notification::warn!(
                            "sidebar/quick_launch",
                            "Sign in before copying Steam launch options."
                        );
                    }
                }
                InstanceContextAction::Delete => {
                    tracing::info!(
                        target: "vertexlauncher/navigation",
                        instance_id = %instance_id,
                        "Instance context menu requested delete; redirecting to Library."
                    );
                    self.selected_instance_id = Some(instance_id.clone());
                    self.active_screen = screens::AppScreen::Library;
                    screens::request_delete_instance(ctx, &instance_id);
                }
            }
        }
        if sidebar_output.create_instance_clicked {
            self.show_create_instance_modal = true;
            self.create_instance_state.error = None;
        }
        if sidebar_output.import_instance_clicked {
            self.show_import_instance_modal = true;
            self.import_instance_state.error = None;
        }

        let mut screen_output = screens::ScreenOutput::default();
        let wgpu_target_format = frame.wgpu_render_state().map(|state| state.target_format);
        let skin_preview_msaa_samples = 4;
        if app_metadata::try_settings_info().is_none() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let settings_info = app_metadata::try_settings_info().unwrap_or_default();
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
                    .fill(ctx.global_style().visuals.panel_fill)
                    .inner_margin(egui::Margin::ZERO)
                    .outer_margin(egui::Margin::ZERO)
                    .stroke(egui::Stroke::new(
                        1.0,
                        ctx.global_style()
                            .visuals
                            .widgets
                            .noninteractive
                            .bg_stroke
                            .color,
                    )),
            )
            .show(ctx, |ui| {
                let content_rect = ui
                    .max_rect()
                    .shrink2(egui::vec2(ui::style::SPACE_MD, ui::style::SPACE_SM));
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(content_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                    |ui| {
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
                            self.fonts.available_emoji_fonts(),
                            self.theme_catalog.themes(),
                            &settings_info,
                            &mut self.content_browser_state,
                            &mut self.discover_state,
                            &mut self.text_ui,
                        );
                    },
                );
            });
        self.last_rendered_screen = Some(self.active_screen);

        if screen_output.instances_changed {
            self.refresh_instance_shortcuts();
        }
        if let Some(instance_id) = screen_output.selected_instance_id {
            tracing::info!(
                target: "vertexlauncher/navigation",
                selected_instance_id = %instance_id,
                "Screen output selected an instance."
            );
            self.selected_instance_id = Some(instance_id);
        }
        if let Some(request) = screen_output.discover_install_requested {
            start_discover_install_task(self, request);
        }
        if let Some(instance_id) = screen_output.delete_requested_instance_id {
            tracing::info!(
                target: "vertexlauncher/navigation",
                instance_id = %instance_id,
                "Screen output requested instance deletion flow; redirecting to Library."
            );
            self.selected_instance_id = Some(instance_id.clone());
            self.active_screen = screens::AppScreen::Library;
            screens::request_delete_instance(ctx, &instance_id);
        }
        if let Some(requested_screen) = screen_output.requested_screen {
            tracing::info!(
                target: "vertexlauncher/navigation",
                from_screen = ?self.active_screen,
                requested_screen = ?requested_screen,
                "Screen output requested navigation."
            );
            self.active_screen = requested_screen;
        }

        self.update_discover_lifecycle(ctx);

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
            if self.create_instance_state.create_in_flight {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
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
                    start_create_instance_task(self, draft);
                }
            }
        }

        if self.show_import_instance_modal && self.pending_curseforge_manual_download.is_none() {
            if self.import_instance_state.import_in_flight {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            match import_instance_modal::render(
                ctx,
                &mut self.text_ui,
                &mut self.import_instance_state,
                !self.config.curseforge_api_key().trim().is_empty(),
            ) {
                import_instance_modal::ModalAction::None => {}
                import_instance_modal::ModalAction::Cancel => {
                    self.show_import_instance_modal = false;
                    self.import_instance_state.reset();
                }
                import_instance_modal::ModalAction::Import(request) => {
                    start_import_instance_task(self, request);
                }
            }
        }
        if self.show_gamepad_calibration_modal {
            let live_sample = self.gamepad.as_ref().and_then(|gamepad| {
                self.gamepad_calibration_state
                    .device_key()
                    .and_then(|device_key| gamepad.current_left_stick(device_key))
            });
            match gamepad_calibration_modal::render(
                ctx,
                &mut self.text_ui,
                &mut self.gamepad_calibration_state,
                live_sample,
            ) {
                gamepad_calibration_modal::ModalAction::None => {}
                gamepad_calibration_modal::ModalAction::Cancel => {
                    self.show_gamepad_calibration_modal = false;
                    self.gamepad_calibration_state.reset();
                }
                gamepad_calibration_modal::ModalAction::Save {
                    device_key,
                    calibration,
                } => {
                    self.config
                        .set_gamepad_calibration(device_key.clone(), calibration);
                    if let Some(gamepad) = self.gamepad.as_mut() {
                        gamepad.reset_navigation_state(device_key.as_str());
                    }
                    self.show_gamepad_calibration_modal = false;
                    self.gamepad_calibration_state.reset();
                    notification::info!("gamepad", "Saved gamepad calibration.");
                }
            }
        }
        if let Some(pending) = self
            .pending_curseforge_manual_download
            .as_mut()
            .filter(|pending| !pending.pending_files.is_empty())
        {
            ctx.request_repaint_after(Duration::from_millis(200));
            match render_curseforge_manual_download_modal(ctx, &mut self.text_ui, pending) {
                ManualCurseForgeDownloadAction::None => {}
                ManualCurseForgeDownloadAction::Cancel => {
                    cancel_pending_curseforge_manual_download(self);
                }
                ManualCurseForgeDownloadAction::OpenDownloadsFolder => {
                    if let Err(err) =
                        launcher_ui::desktop::open_in_file_manager(pending.downloads_dir.as_path())
                    {
                        pending.error = Some(format!("Failed to open downloads folder: {err}"));
                    }
                }
            }
        }

        self.config.normalize();
        self.fonts
            .ensure_selected_font_is_available(&mut self.config);
        if self.config != previous_config {
            if transparent_viewport_enabled(&self.config)
                != transparent_viewport_enabled(&previous_config)
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Transparent(
                    transparent_viewport_enabled(&self.config),
                ));
            }
            queue_config_save(self);
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
            queue_instance_store_save(self);
        }

        let menu_presence_context = screen_output
            .menu_presence_context
            .unwrap_or(screens::MenuPresenceContext::Screen(self.active_screen));

        if self.active_screen != previous_active_screen
            || self.selected_instance_id != previous_selected_instance_id
        {
            tracing::info!(
                target: "vertexlauncher/navigation",
                from_screen = ?previous_active_screen,
                to_screen = ?self.active_screen,
                previous_selected_instance_id = previous_selected_instance_id.as_deref().unwrap_or(""),
                selected_instance_id = self.selected_instance_id.as_deref().unwrap_or(""),
                menu_presence_context = ?menu_presence_context,
                "Launcher navigation state changed."
            );
        }

        if previous_active_screen == screens::AppScreen::Home
            && self.active_screen != screens::AppScreen::Home
        {
            screens::purge_home_screenshot_state(ctx);
        }
        if previous_active_screen == screens::AppScreen::Instance
            && (self.active_screen != screens::AppScreen::Instance
                || self.selected_instance_id != previous_selected_instance_id)
        {
            screens::purge_instance_screenshot_state(ctx, previous_selected_instance_id.as_deref());
        }

        self.discord_presence.update(
            &self.config,
            &self.instance_store,
            self.config.minecraft_installations_root_path(),
            menu_presence_context,
            self.selected_instance_id.as_deref(),
        );

        ui::top_bar::handle_window_resize(ctx);
    }

    pub(super) fn update_discover_lifecycle(&mut self, ctx: &egui::Context) {
        update_screen_purge_timer(
            ctx,
            &mut self.home_purge_timer,
            self.active_screen == screens::AppScreen::Home,
            DISCOVER_STATE_PURGE_DELAY,
            || {
                screens::purge_inactive_home_state(ctx);
                tracing::info!(
                    target: "vertexlauncher/home",
                    "Purged inactive home state after timeout."
                );
            },
        );
        update_screen_purge_timer(
            ctx,
            &mut self.library_purge_timer,
            self.active_screen == screens::AppScreen::Library,
            DISCOVER_STATE_PURGE_DELAY,
            || {
                screens::purge_inactive_library_state(ctx);
                tracing::info!(
                    target: "vertexlauncher/library",
                    "Purged inactive library state after timeout."
                );
            },
        );
        update_screen_purge_timer(
            ctx,
            &mut self.instance_purge_timer,
            self.active_screen == screens::AppScreen::Instance,
            DISCOVER_STATE_PURGE_DELAY,
            || {
                screens::purge_inactive_instance_state(ctx, self.selected_instance_id.as_deref());
                tracing::info!(
                    target: "vertexlauncher/instance",
                    selected_instance_id = self.selected_instance_id.as_deref().unwrap_or(""),
                    "Purged inactive instance state after timeout."
                );
            },
        );
        update_screen_purge_timer(
            ctx,
            &mut self.discover_purge_timer,
            is_discover_screen(self.active_screen),
            DISCOVER_STATE_PURGE_DELAY,
            || {
                self.discover_state.purge_inactive_state();
                tracing::info!(
                    target: "vertexlauncher/discover",
                    "Purged inactive discover state after timeout."
                );
            },
        );
        update_screen_purge_timer(
            ctx,
            &mut self.content_browser_purge_timer,
            self.active_screen == screens::AppScreen::ContentBrowser,
            DISCOVER_STATE_PURGE_DELAY,
            || {
                self.content_browser_state.purge_inactive_state();
                tracing::info!(
                    target: "vertexlauncher/content_browser",
                    "Purged inactive content browser state after timeout."
                );
            },
        );
        update_screen_purge_timer(
            ctx,
            &mut self.skins_purge_timer,
            self.active_screen == screens::AppScreen::Skins,
            DISCOVER_STATE_PURGE_DELAY,
            || {
                screens::purge_inactive_skins_state(ctx);
                tracing::info!(
                    target: "vertexlauncher/skins",
                    "Purged inactive skins state after timeout."
                );
            },
        );
    }

    pub(super) fn create_config_with_choice(&mut self, choice: ConfigFormat) {
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

    pub(super) fn sync_theme_from_config(&mut self) {
        if !self.theme_catalog.contains(self.config.theme_id()) {
            self.config
                .set_theme_id(self.theme_catalog.default_theme_id().to_owned());
        }

        let resolved = self.theme_catalog.resolve(self.config.theme_id());
        if self.theme.id != resolved.id {
            self.theme = resolved.clone();
        }
    }

    pub(super) fn refresh_instance_shortcuts(&mut self) {
        self.instance_shortcuts = self
            .instance_store
            .instances
            .iter()
            .map(|instance| ui::sidebar::ProfileShortcut {
                id: instance.id.clone(),
                name: instance.name.clone(),
                thumbnail_path: instance.thumbnail_path.clone(),
            })
            .collect();
    }

    pub(super) fn open_instance_folder(&mut self, instance_id: &str) {
        let Some(instance) = self.instance_store.find(instance_id).cloned() else {
            notification::error!(
                "instance_context_menu",
                "Could not find the selected instance to open its folder."
            );
            return;
        };

        let installations_root = self
            .config
            .minecraft_installations_root_path()
            .to_path_buf();
        let instance_root = instance_root_path(installations_root.as_path(), &instance);
        if let Err(err) = launcher_ui::desktop::open_in_file_manager(&instance_root) {
            notification::error!(
                "instance_context_menu",
                "Failed to open instance folder: {err}"
            );
        }
    }

    pub(super) fn apply_frame_limiter(&mut self) {
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

    pub(super) fn handle_escape(&mut self, ctx: &egui::Context) -> bool {
        if !ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            return false;
        }
        if egui::Popup::is_any_open(ctx) {
            egui::Popup::close_all(ctx);
            return true;
        }
        if self.pending_curseforge_manual_download.is_some() {
            if self.import_instance_state.import_in_flight
                || self.curseforge_manual_download_preflight_in_flight
                || self.discover_curseforge_manual_download_preflight_in_flight
            {
                return true;
            }
            cancel_pending_curseforge_manual_download(self);
            return true;
        }
        if self.show_import_instance_modal {
            if self.import_instance_state.import_in_flight {
                return true;
            }
            self.show_import_instance_modal = false;
            self.import_instance_state.reset();
            return true;
        }
        if self.show_create_instance_modal {
            if self.create_instance_state.create_in_flight {
                return true;
            }
            self.show_create_instance_modal = false;
            self.create_instance_state.reset();
            return true;
        }
        if self.show_config_format_modal {
            self.create_config_with_choice(self.default_config_format);
            return true;
        }
        if self.active_screen == screens::AppScreen::DiscoverDetail {
            self.active_screen = screens::AppScreen::Discover;
            return true;
        }
        screens::handle_escape(
            ctx,
            self.active_screen,
            self.selected_instance_id.as_deref(),
        )
    }
}
