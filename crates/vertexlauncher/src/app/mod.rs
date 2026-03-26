use config::{
    Config, ConfigFormat, JavaRuntimeVersion, LoadConfigResult, create_default_config, load_config,
    save_config,
};
use eframe::{self, egui};
use egui::CentralPanel;
use installation::{
    DownloadPolicy, InstallProgress, InstallProgressCallback, InstallStage, display_user_path,
    ensure_game_files, ensure_openjdk_runtime, running_instance_for_account,
    running_instance_roots,
};
use instances::{
    InstanceRecord, InstanceStore, create_instance, instance_root_path, load_store,
    save_store as save_instance_store,
};
use launcher_runtime as tokio_runtime;
use launcher_ui::ui::svg_aa;
use launcher_ui::{
    console, install_activity, notification, screens, ui,
    ui::instance_context_menu::InstanceContextAction, window_effects,
};
use std::{
    any::Any,
    collections::HashMap,
    fs,
    io::{Read, Write},
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use textui::TextUi;

use self::auth_state::{AuthState, REPAINT_INTERVAL};
use self::config_format_modal::ModalAction;
use self::discord_presence::DiscordPresenceManager;
use self::fonts::FontController;

mod app_icon;
mod app_metadata;
mod auth_state;
mod cli;
mod config_format_modal;
mod create_instance_modal;
mod discord_presence;
mod fonts;
mod import_instance_modal;
mod native_options;
mod platform;
mod single_instance;
mod taskbar_progress;
mod tracing_setup;
mod webview_runtime;
mod webview_sign_in;

pub use single_instance::{SingleInstanceError, acquire_single_instance};

#[derive(Debug)]
pub enum RunError {
    RuntimeBootstrap(launcher_runtime::RuntimeBootstrapError),
    Ui(eframe::Error),
}

pub(crate) fn init_tracing() -> Option<PathBuf> {
    launcher_runtime::set_detached_task_reporter(report_detached_task_failure);
    tracing_setup::init_tracing()
}

fn report_detached_task_failure(task_kind: &str, error: &launcher_runtime::TaskError) {
    notification::emit_replace(
        notification::Severity::Error,
        "background-task",
        format!("A background task failed ({task_kind}): {error}"),
        format!("background-task/{task_kind}"),
    );
}

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
    content_browser_state: screens::ContentBrowserState,
    discover_state: screens::DiscoverState,
    show_create_instance_modal: bool,
    create_instance_state: create_instance_modal::CreateInstanceState,
    show_import_instance_modal: bool,
    import_instance_state: import_instance_modal::ImportInstanceState,
    discover_install_progress_tx: Option<mpsc::Sender<import_instance_modal::ImportProgress>>,
    discover_install_progress_rx:
        Option<Arc<Mutex<mpsc::Receiver<import_instance_modal::ImportProgress>>>>,
    discover_install_results_tx: Option<mpsc::Sender<import_instance_modal::ImportTaskResult>>,
    discover_install_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<import_instance_modal::ImportTaskResult>>>>,
    auth: AuthState,
    text_ui: TextUi,
    config_save_in_flight: bool,
    pending_config_save: Option<Config>,
    config_save_results_tx: Option<mpsc::Sender<Result<(), String>>>,
    config_save_results_rx: Option<mpsc::Receiver<Result<(), String>>>,
    instance_store_save_in_flight: bool,
    pending_instance_store_save: Option<InstanceStore>,
    instance_store_save_results_tx: Option<mpsc::Sender<Result<(), String>>>,
    instance_store_save_results_rx: Option<mpsc::Receiver<Result<(), String>>>,
    discord_presence: DiscordPresenceManager,
    last_frame_end: Option<Instant>,
    last_rendered_screen: Option<screens::AppScreen>,
}

impl VertexApp {
    fn new(cc: &eframe::CreationContext<'_>, config_state: LoadConfigResult) -> Self {
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
        if let Err(error) = window_effects::apply(cc, effective_window_blur_enabled(&config)) {
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

        let mut text_ui = TextUi::new();
        text_ui.begin_frame(&cc.egui_ctx);
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
            show_create_instance_modal: false,
            create_instance_state: create_instance_modal::CreateInstanceState::default(),
            show_import_instance_modal: false,
            import_instance_state: import_instance_modal::ImportInstanceState::default(),
            discover_install_progress_tx: None,
            discover_install_progress_rx: None,
            discover_install_results_tx: None,
            discover_install_results_rx: None,
            auth: AuthState::load(streamer_mode_enabled),
            text_ui,
            config_save_in_flight: false,
            pending_config_save: None,
            config_save_results_tx: None,
            config_save_results_rx: None,
            instance_store_save_in_flight: false,
            pending_instance_store_save: None,
            instance_store_save_results_tx: None,
            instance_store_save_results_rx: None,
            discord_presence: DiscordPresenceManager::default(),
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

    fn update_inner(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.apply_frame_limiter();
        self.text_ui.begin_frame(ctx);
        poll_config_save_results(self);
        poll_instance_store_save_results(self);
        self.auth.poll();
        console::prune_instance_tabs(&running_instance_roots());
        apply_install_activity_os_feedback(ctx, frame);
        if self.auth.should_request_repaint() {
            ctx.request_repaint_after(REPAINT_INTERVAL);
        }

        let previous_config = self.config.clone();
        let previous_instance_store = self.instance_store.clone();
        poll_create_instance_result(self);
        poll_import_instance_progress(self);
        poll_import_instance_result(self);
        poll_discover_install_progress(self);
        poll_discover_install_result(self);
        self.sync_theme_from_config();
        self.theme
            .apply(ctx, effective_window_blur_enabled(&self.config));
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
            },
        );
        if self.active_screen != screens::AppScreen::Instance {
            notification::render_popups(
                ctx,
                &mut self.text_ui,
                self.config.notification_expiry_bars_empty_left(),
            );
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
        if let Some(profile_id) = top_bar_output.refresh_account_id.as_deref() {
            self.auth.refresh_account_token(profile_id);
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

        svg_aa::set_svg_aa_mode(self.config.svg_aa_mode());

        let sidebar_output = ui::sidebar::render(
            ctx,
            self.active_screen,
            &self.instance_shortcuts,
            &mut self.text_ui,
        );

        if let Some(next_screen) = sidebar_output.selected_screen {
            self.active_screen = next_screen;
        }
        if let Some(instance_id) = sidebar_output.selected_profile_id {
            self.selected_instance_id = Some(instance_id);
            self.active_screen = screens::AppScreen::Instance;
        }
        for (instance_id, action) in sidebar_output.instance_context_actions {
            match action {
                InstanceContextAction::OpenInstance => {
                    self.selected_instance_id = Some(instance_id);
                    self.active_screen = screens::AppScreen::Instance;
                }
                InstanceContextAction::OpenFolder => {
                    self.open_instance_folder(&instance_id);
                }
                InstanceContextAction::Delete => {
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
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::ZERO)
                    .outer_margin(egui::Margin::ZERO)
                    .stroke(egui::Stroke::new(
                        1.0,
                        ctx.style().visuals.widgets.noninteractive.bg_stroke.color,
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
            self.selected_instance_id = Some(instance_id);
        }
        if let Some(request) = screen_output.discover_install_requested {
            start_discover_install_task(self, request);
        }
        if let Some(instance_id) = screen_output.delete_requested_instance_id {
            self.selected_instance_id = Some(instance_id.clone());
            self.active_screen = screens::AppScreen::Library;
            screens::request_delete_instance(ctx, &instance_id);
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

        if self.show_import_instance_modal {
            if self.import_instance_state.import_in_flight {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            match import_instance_modal::render(
                ctx,
                &mut self.text_ui,
                &mut self.import_instance_state,
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

        self.config.normalize();
        self.fonts
            .ensure_selected_font_is_available(&mut self.config);
        if self.config != previous_config {
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

        self.discord_presence.update(
            &self.config,
            &self.instance_store,
            Path::new(self.config.minecraft_installations_root()),
        );

        ui::top_bar::handle_window_resize(ctx);
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
                thumbnail_path: instance.thumbnail_path.clone(),
            })
            .collect();
    }

    fn open_instance_folder(&mut self, instance_id: &str) {
        let Some(instance) = self.instance_store.find(instance_id).cloned() else {
            notification::error!(
                "instance_context_menu",
                "Could not find the selected instance to open its folder."
            );
            return;
        };

        let installations_root = PathBuf::from(self.config.minecraft_installations_root());
        let instance_root = instance_root_path(installations_root.as_path(), &instance);
        if let Err(err) = open_path_in_file_manager(&instance_root) {
            notification::error!(
                "instance_context_menu",
                "Failed to open instance folder: {err}"
            );
        }
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

    fn handle_escape(&mut self, ctx: &egui::Context) -> bool {
        if !ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            return false;
        }
        if egui::Popup::is_any_open(ctx) {
            egui::Popup::close_all(ctx);
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

fn open_path_in_file_manager(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open").arg(path).spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer").arg(path).spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn()?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
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

fn effective_window_blur_enabled(config: &Config) -> bool {
    config.window_blur_enabled() && window_effects::platform_supports_blur()
}

fn disable_window_blur_for_startup(
    cc: &eframe::CreationContext<'_>,
    config: &mut Config,
    config_loaded_from_disk: bool,
    message: String,
    save_context: &'static str,
) {
    if !config.window_blur_enabled() {
        return;
    }

    config.set_window_blur_enabled(false);
    cc.egui_ctx
        .send_viewport_cmd(egui::ViewportCommand::Transparent(false));
    notification::warn!("window_blur", "{message}");

    if !config_loaded_from_disk {
        return;
    }

    let config_to_save = config.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let result = save_config(&config_to_save).map_err(|err| err.to_string());
        if let Err(save_error) = result {
            notification::warn!(
                "config",
                "Failed to persist disabled blur setting after {save_context}: {save_error}"
            );
        }
    });
}

fn ensure_config_save_channel(app: &mut VertexApp) {
    if app.config_save_results_tx.is_some() && app.config_save_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    app.config_save_results_tx = Some(tx);
    app.config_save_results_rx = Some(rx);
}

fn start_pending_config_save(app: &mut VertexApp) {
    if app.config_save_in_flight {
        return;
    }
    let Some(config) = app.pending_config_save.take() else {
        return;
    };

    ensure_config_save_channel(app);
    let Some(tx) = app.config_save_results_tx.as_ref().cloned() else {
        app.pending_config_save = Some(config);
        return;
    };

    app.config_save_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = save_config(&config).map_err(|err| err.to_string());
        let _ = tx.send(result);
    });
}

fn queue_config_save(app: &mut VertexApp) {
    app.pending_config_save = Some(app.config.clone());
    start_pending_config_save(app);
}

fn poll_config_save_results(app: &mut VertexApp) {
    let mut should_reset_channel = false;
    let mut saw_result = false;
    loop {
        let Some(result) = app.config_save_results_rx.as_ref().map(|rx| rx.try_recv()) else {
            return;
        };
        match result {
            Ok(result) => {
                saw_result = true;
                app.config_save_in_flight = false;
                if let Err(err) = result {
                    tracing::error!(
                        target: "vertexlauncher/app/config",
                        "Failed to save config: {err}"
                    );
                }
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                should_reset_channel = true;
                app.config_save_in_flight = false;
                break;
            }
        }
    }

    if should_reset_channel {
        app.config_save_results_tx = None;
        app.config_save_results_rx = None;
    }
    if saw_result || !app.config_save_in_flight {
        start_pending_config_save(app);
    }
}

fn ensure_instance_store_save_channel(app: &mut VertexApp) {
    if app.instance_store_save_results_tx.is_some() && app.instance_store_save_results_rx.is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    app.instance_store_save_results_tx = Some(tx);
    app.instance_store_save_results_rx = Some(rx);
}

fn start_pending_instance_store_save(app: &mut VertexApp) {
    if app.instance_store_save_in_flight {
        return;
    }
    let Some(store) = app.pending_instance_store_save.take() else {
        return;
    };

    ensure_instance_store_save_channel(app);
    let Some(tx) = app.instance_store_save_results_tx.as_ref().cloned() else {
        app.pending_instance_store_save = Some(store);
        return;
    };

    app.instance_store_save_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = save_instance_store(&store).map_err(|err| err.to_string());
        let _ = tx.send(result);
    });
}

fn queue_instance_store_save(app: &mut VertexApp) {
    app.pending_instance_store_save = Some(app.instance_store.clone());
    start_pending_instance_store_save(app);
}

fn poll_instance_store_save_results(app: &mut VertexApp) {
    let mut should_reset_channel = false;
    let mut saw_result = false;
    loop {
        let Some(result) = app
            .instance_store_save_results_rx
            .as_ref()
            .map(|rx| rx.try_recv())
        else {
            return;
        };
        match result {
            Ok(result) => {
                saw_result = true;
                app.instance_store_save_in_flight = false;
                if let Err(err) = result {
                    tracing::error!(
                        target: "vertexlauncher/app/instances",
                        "Failed to save instances: {err}"
                    );
                }
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                should_reset_channel = true;
                app.instance_store_save_in_flight = false;
                break;
            }
        }
    }

    if should_reset_channel {
        app.instance_store_save_results_tx = None;
        app.instance_store_save_results_rx = None;
    }
    if saw_result || !app.instance_store_save_in_flight {
        start_pending_instance_store_save(app);
    }
}

fn ensure_create_instance_channel(state: &mut create_instance_modal::CreateInstanceState) {
    if state.create_results_tx.is_some() && state.create_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<create_instance_modal::CreateInstanceTaskResult>();
    state.create_results_tx = Some(tx);
    state.create_results_rx = Some(rx);
}

fn start_create_instance_task(
    app: &mut VertexApp,
    draft: create_instance_modal::CreateInstanceDraft,
) {
    if app.create_instance_state.create_in_flight {
        return;
    }

    ensure_create_instance_channel(&mut app.create_instance_state);
    let Some(tx) = app
        .create_instance_state
        .create_results_tx
        .as_ref()
        .cloned()
    else {
        return;
    };

    app.create_instance_state.error = None;
    app.create_instance_state.create_in_flight = true;
    let mut store = app.instance_store.clone();
    let installations_root = PathBuf::from(app.config.minecraft_installations_root());
    let _ = tokio_runtime::spawn_detached(async move {
        let result = create_instance(
            &mut store,
            &installations_root,
            draft.into_new_instance_spec(),
        )
        .map(|instance| (store, instance))
        .map_err(|err| err.to_string());
        let _ = tx.send(result);
    });
}

fn poll_create_instance_result(app: &mut VertexApp) {
    let Some(result) = app
        .create_instance_state
        .create_results_rx
        .as_ref()
        .map(|rx| rx.try_recv())
    else {
        return;
    };
    let result = match result {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            app.create_instance_state.create_results_tx = None;
            app.create_instance_state.create_results_rx = None;
            app.create_instance_state.create_in_flight = false;
            app.create_instance_state.error =
                Some("Create instance task stopped unexpectedly.".to_owned());
            return;
        }
    };

    app.create_instance_state.create_in_flight = false;
    match result {
        Ok((store, instance)) => {
            let installations_root = PathBuf::from(app.config.minecraft_installations_root());
            app.instance_store = store;
            start_initial_instance_install(&instance, installations_root.as_path(), &app.config);
            app.selected_instance_id = Some(instance.id);
            app.active_screen = screens::AppScreen::Instance;
            app.show_create_instance_modal = false;
            app.create_instance_state.reset();
            app.refresh_instance_shortcuts();
        }
        Err(err) => {
            app.create_instance_state.error = Some(format!("Failed to create instance: {err}"));
        }
    }
}

fn ensure_import_instance_channel(state: &mut import_instance_modal::ImportInstanceState) {
    if state.import_results_tx.is_some() && state.import_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<import_instance_modal::ImportTaskResult>();
    state.import_results_tx = Some(tx);
    state.import_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn ensure_import_instance_progress_channel(state: &mut import_instance_modal::ImportInstanceState) {
    if state.import_progress_tx.is_some() && state.import_progress_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<import_instance_modal::ImportProgress>();
    state.import_progress_tx = Some(tx);
    state.import_progress_rx = Some(Arc::new(Mutex::new(rx)));
}

fn start_import_instance_task(app: &mut VertexApp, request: import_instance_modal::ImportRequest) {
    if app.import_instance_state.import_in_flight {
        return;
    }

    ensure_import_instance_channel(&mut app.import_instance_state);
    ensure_import_instance_progress_channel(&mut app.import_instance_state);
    let Some(tx) = app
        .import_instance_state
        .import_results_tx
        .as_ref()
        .cloned()
    else {
        return;
    };
    let Some(progress_tx) = app
        .import_instance_state
        .import_progress_tx
        .as_ref()
        .cloned()
    else {
        return;
    };

    app.import_instance_state.error = None;
    app.import_instance_state.import_in_flight = true;
    app.import_instance_state.import_latest_progress = None;
    let store = app.instance_store.clone();
    let installations_root = PathBuf::from(app.config.minecraft_installations_root());
    let _ = tokio_runtime::spawn_detached(async move {
        let result = import_package_in_background(store, installations_root, request, progress_tx);
        let _ = tx.send(result);
    });
}

fn poll_import_instance_progress(app: &mut VertexApp) {
    let Some(rx) = app
        .import_instance_state
        .import_progress_rx
        .as_ref()
        .cloned()
    else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        return;
    };
    while let Ok(progress) = receiver.try_recv() {
        app.import_instance_state.import_latest_progress = Some(progress);
    }
}

fn poll_import_instance_result(app: &mut VertexApp) {
    let Some(rx) = app
        .import_instance_state
        .import_results_rx
        .as_ref()
        .cloned()
    else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        return;
    };
    let Ok(result) = receiver.try_recv() else {
        return;
    };

    app.import_instance_state.import_in_flight = false;
    app.import_instance_state.import_latest_progress = None;
    match result {
        Ok((store, instance)) => {
            let installations_root = PathBuf::from(app.config.minecraft_installations_root());
            app.instance_store = store;
            start_initial_instance_install(&instance, installations_root.as_path(), &app.config);
            app.selected_instance_id = Some(instance.id);
            app.active_screen = screens::AppScreen::Instance;
            app.show_import_instance_modal = false;
            app.import_instance_state.reset();
            app.refresh_instance_shortcuts();
        }
        Err(err) => {
            tracing::error!(
                target: "vertexlauncher/app/import",
                error = %err,
                "Import profile task failed."
            );
            app.import_instance_state.error = Some(format!("Failed to import profile: {err}"));
        }
    }
}

fn ensure_discover_install_channel(app: &mut VertexApp) {
    if app.discover_install_results_tx.is_some() && app.discover_install_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<import_instance_modal::ImportTaskResult>();
    app.discover_install_results_tx = Some(tx);
    app.discover_install_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn ensure_discover_install_progress_channel(app: &mut VertexApp) {
    if app.discover_install_progress_tx.is_some() && app.discover_install_progress_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<import_instance_modal::ImportProgress>();
    app.discover_install_progress_tx = Some(tx);
    app.discover_install_progress_rx = Some(Arc::new(Mutex::new(rx)));
}

fn start_discover_install_task(app: &mut VertexApp, request: screens::DiscoverInstallRequest) {
    if app.show_import_instance_modal || app.import_instance_state.import_in_flight {
        return;
    }
    ensure_discover_install_channel(app);
    ensure_discover_install_progress_channel(app);
    let Some(tx) = app.discover_install_results_tx.as_ref().cloned() else {
        return;
    };
    let Some(progress_tx) = app.discover_install_progress_tx.as_ref().cloned() else {
        return;
    };

    app.discover_state
        .begin_install(format!("Downloading {}...", request.version_name));
    let store = app.instance_store.clone();
    let installations_root = PathBuf::from(app.config.minecraft_installations_root());
    let _ = tokio_runtime::spawn_detached(async move {
        let result =
            install_discover_modpack_in_background(store, installations_root, request, progress_tx);
        let _ = tx.send(result);
    });
}

fn poll_discover_install_progress(app: &mut VertexApp) {
    let Some(rx) = app.discover_install_progress_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        return;
    };
    while let Ok(progress) = receiver.try_recv() {
        app.discover_state.apply_install_progress(
            progress.message,
            progress.completed_steps,
            progress.total_steps,
        );
    }
}

fn poll_discover_install_result(app: &mut VertexApp) {
    let Some(rx) = app.discover_install_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        return;
    };
    let Ok(result) = receiver.try_recv() else {
        return;
    };

    match result {
        Ok((store, instance)) => {
            let installations_root = PathBuf::from(app.config.minecraft_installations_root());
            app.instance_store = store;
            start_initial_instance_install(&instance, installations_root.as_path(), &app.config);
            app.selected_instance_id = Some(instance.id);
            app.active_screen = screens::AppScreen::Instance;
            app.discover_state
                .finish_install(Ok("Created instance from modpack.".to_owned()));
            app.refresh_instance_shortcuts();
        }
        Err(err) => {
            tracing::error!(
                target: "vertexlauncher/app/discover",
                error = %err,
                "Discover modpack install failed."
            );
            app.discover_state.finish_install(Err(err));
        }
    }
}

fn install_discover_modpack_in_background(
    store: InstanceStore,
    installations_root: PathBuf,
    request: screens::DiscoverInstallRequest,
    progress_tx: mpsc::Sender<import_instance_modal::ImportProgress>,
) -> Result<(InstanceStore, InstanceRecord), String> {
    let instance_name = request.instance_name.clone();
    let project_summary = request.project_summary.clone();
    let icon_url = request.icon_url.clone();
    match request.source {
        screens::DiscoverInstallSource::Modrinth {
            file_url,
            file_name,
            ..
        } => {
            let temp_path = download_discover_modpack_file(
                file_url.as_str(),
                file_name.as_str(),
                &progress_tx,
            )?;
            let import_request = import_instance_modal::ImportRequest {
                source: import_instance_modal::ImportSource::ManifestFile(temp_path.clone()),
                instance_name: instance_name.clone(),
            };
            let result = import_package_in_background(
                store,
                installations_root.clone(),
                import_request,
                progress_tx,
            );
            let _ = fs::remove_file(temp_path.as_path());
            finalize_discover_instance(
                result,
                installations_root.as_path(),
                instance_name.as_str(),
                project_summary.as_deref(),
                icon_url.as_deref(),
            )
        }
        screens::DiscoverInstallSource::CurseForge {
            project_id,
            file_id,
            file_name,
            download_url,
        } => {
            let download_url = match download_url {
                Some(url) => url,
                None => curseforge::Client::from_env()
                    .ok_or_else(|| "CurseForge API key missing in settings.".to_owned())?
                    .get_mod_file_download_url(project_id, file_id)
                    .map_err(|err| {
                        import_instance_modal::format_curseforge_download_url_error(
                            project_id, file_id, &err,
                        )
                    })?
                    .ok_or_else(|| {
                        format!(
                            "CurseForge file {file_id} for project {project_id} has no download URL"
                        )
                    })?,
            };
            let temp_path = download_discover_modpack_file(
                download_url.as_str(),
                file_name.as_str(),
                &progress_tx,
            )?;
            let import_request = import_instance_modal::ImportRequest {
                source: import_instance_modal::ImportSource::ManifestFile(temp_path.clone()),
                instance_name: instance_name.clone(),
            };
            let result = import_package_in_background(
                store,
                installations_root.clone(),
                import_request,
                progress_tx,
            );
            let final_result = result.and_then(|(store, instance)| {
                let instance_root = instance_root_path(installations_root.as_path(), &instance);
                import_instance_modal::attach_curseforge_modpack_install_state(
                    instance_root.as_path(),
                    project_id,
                    file_id,
                    instance_name.as_str(),
                    request.version_name.as_str(),
                )?;
                Ok((store, instance))
            });
            let _ = fs::remove_file(temp_path.as_path());
            finalize_discover_instance(
                final_result,
                installations_root.as_path(),
                instance_name.as_str(),
                project_summary.as_deref(),
                icon_url.as_deref(),
            )
        }
    }
}

fn finalize_discover_instance(
    result: Result<(InstanceStore, InstanceRecord), String>,
    installations_root: &Path,
    instance_name: &str,
    project_summary: Option<&str>,
    icon_url: Option<&str>,
) -> Result<(InstanceStore, InstanceRecord), String> {
    let (mut store, instance) = result?;
    apply_discover_instance_metadata(
        &mut store,
        installations_root,
        instance.id.as_str(),
        instance_name,
        project_summary,
        icon_url,
    )?;
    let updated = store
        .find(instance.id.as_str())
        .cloned()
        .ok_or_else(|| format!("instance {} disappeared after install", instance.id))?;
    Ok((store, updated))
}

fn apply_discover_instance_metadata(
    store: &mut InstanceStore,
    installations_root: &Path,
    instance_id: &str,
    instance_name: &str,
    project_summary: Option<&str>,
    icon_url: Option<&str>,
) -> Result<(), String> {
    let instance = store
        .find_mut(instance_id)
        .ok_or_else(|| format!("instance {instance_id} disappeared during discover install"))?;
    instance.name = instance_name.trim().to_owned();
    if let Some(summary) = project_summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        instance.description = Some(summary.to_owned());
    }

    let instance_root = instance_root_path(installations_root, instance);
    if let Some(icon_url) = icon_url.map(str::trim).filter(|value| !value.is_empty()) {
        match download_discover_thumbnail(icon_url, instance_root.as_path(), instance_id) {
            Ok(Some(path)) => instance.thumbnail_path = Some(path),
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/app/discover",
                    instance_id,
                    error = %err,
                    "failed to persist discover thumbnail"
                );
            }
        }
    }

    save_instance_store(store).map_err(|err| format!("failed to save instance metadata: {err}"))
}

fn download_discover_modpack_file(
    url: &str,
    file_name: &str,
    progress_tx: &mpsc::Sender<import_instance_modal::ImportProgress>,
) -> Result<PathBuf, String> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| format!("failed to download modpack from {url}: {err}"))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read modpack download from {url}: {err}"))?;
    let _ = progress_tx.send(import_instance_modal::ImportProgress {
        message: "Downloaded modpack package. Importing instance...".to_owned(),
        completed_steps: 1,
        total_steps: 1,
    });
    let temp_path = std::env::temp_dir().join(format!(
        "vertex-discover-{}-{}",
        std::process::id(),
        sanitize_temp_file_name(file_name)
    ));
    let mut file = fs::File::create(temp_path.as_path()).map_err(|err| {
        format!(
            "failed to create temp package {}: {err}",
            temp_path.display()
        )
    })?;
    file.write_all(bytes.as_slice()).map_err(|err| {
        format!(
            "failed to write temp package {}: {err}",
            temp_path.display()
        )
    })?;
    Ok(temp_path)
}

fn sanitize_temp_file_name(file_name: &str) -> String {
    let sanitized = file_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.trim().is_empty() {
        "modpack.mrpack".to_owned()
    } else {
        sanitized
    }
}

fn download_discover_thumbnail(
    url: &str,
    instance_root: &Path,
    instance_id: &str,
) -> Result<Option<String>, String> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| format!("failed to download thumbnail from {url}: {err}"))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read thumbnail from {url}: {err}"))?;
    if bytes.is_empty() {
        return Ok(None);
    }

    let extension = thumbnail_extension_from_url(url);
    let path = instance_root.join(format!(
        ".vertex-discover-thumbnail-{instance_id}.{extension}"
    ));
    fs::write(path.as_path(), bytes)
        .map_err(|err| format!("failed to write thumbnail {}: {err}", path.display()))?;
    Ok(Some(path.to_string_lossy().to_string()))
}

fn thumbnail_extension_from_url(url: &str) -> &'static str {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "jpg"
    } else if path.ends_with(".webp") {
        "webp"
    } else if path.ends_with(".svg") {
        "svg"
    } else {
        "png"
    }
}

fn import_package_in_background(
    mut store: InstanceStore,
    installations_root: PathBuf,
    request: import_instance_modal::ImportRequest,
    progress_tx: mpsc::Sender<import_instance_modal::ImportProgress>,
) -> Result<(InstanceStore, InstanceRecord), String> {
    let instance = import_instance_modal::import_package_with_progress(
        &mut store,
        installations_root.as_path(),
        request,
        |progress| {
            let _ = progress_tx.send(progress);
        },
    )?;
    Ok((store, instance))
}

impl eframe::App for VertexApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.update_inner(ctx, frame))) {
            log_unexpected_panic("ui update", payload.as_ref());
            resume_unwind(payload);
        }
    }
}

fn panic_payload_text(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        return (*text).to_owned();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "non-string panic payload".to_owned()
}

fn log_unexpected_panic(context: &'static str, payload: &(dyn Any + Send)) {
    tracing::error!(
        target: "vertexlauncher/app/stability",
        context,
        message = %panic_payload_text(payload),
        "Launcher hit an unrecoverable panic."
    );
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

    let _ = tokio_runtime::spawn_detached(async move {
        let last_emit = Arc::new(Mutex::new(
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        ));
        let notification_source_for_progress = notification_source.clone();
        let activity_instance_for_progress = activity_instance.clone();
        let result: Result<_, String> = (|| {
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
                            .map(|path| display_user_path(path.as_path()))
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
        })();

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

pub fn run() -> Result<(), RunError> {
    match catch_unwind(AssertUnwindSafe(run_inner)) {
        Ok(result) => result,
        Err(payload) => {
            log_unexpected_panic("launcher runtime", payload.as_ref());
            resume_unwind(payload)
        }
    }
}

fn run_inner() -> Result<(), RunError> {
    let log_path = init_tracing();
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
    launcher_runtime::init().map_err(RunError::RuntimeBootstrap)?;
    let config_state = load_config();
    let startup_config = match &config_state {
        LoadConfigResult::Loaded(config) => config.clone(),
        LoadConfigResult::Missing { .. } => Config::default(),
    };

    tracing::info!(
        target: "vertexlauncher/app/startup",
        "Building native window and renderer options."
    );
    let options = native_options::build(&startup_config);
    tracing::info!(
        target: "vertexlauncher/app/startup",
        "Starting eframe runtime."
    );

    eframe::run_native(
        "Vertex Launcher",
        options,
        Box::new(move |cc| {
            tracing::info!(
                target: "vertexlauncher/app/startup",
                "Renderer initialized; constructing application state."
            );
            match catch_unwind(AssertUnwindSafe(|| {
                VertexApp::new(cc, config_state.clone())
            })) {
                Ok(app) => Ok(Box::new(app) as Box<dyn eframe::App>),
                Err(payload) => {
                    log_unexpected_panic("ui startup", payload.as_ref());
                    resume_unwind(payload)
                }
            }
        }),
    )
    .map_err(RunError::Ui)
}

pub fn maybe_run_webview_helper() -> Result<bool, String> {
    webview_sign_in::maybe_run_helper_from_args()
}

pub fn maybe_run_cli_command() -> Result<bool, String> {
    cli::maybe_run_from_args()
}
