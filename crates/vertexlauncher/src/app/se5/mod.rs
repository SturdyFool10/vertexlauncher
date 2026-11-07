//! The SturdyEngine5 front end: Vertex's UI as an immediate-mode app on SE5.
//!
//! One draw closure rebuilds the whole window every frame from plain state ([`AppState`]); widget animation state lives
//! in [`kit::Kit`]. The engine only renders when something asks (input, a background task finishing, an animation), so
//! an idle launcher costs ~0%. Every setting applies while the launcher runs: config changes go through
//! [`vertex_shell::SettingsBridge`] to the engine appliers and are saved in the background.
//!
//! All non-UI work (accounts, launching, installs, the console, notifications, saving) reuses the same code as the
//! egui front end.

mod accounts;
mod chrome;
mod create_instance;
mod flows;
mod file_dialog;
mod fonts;
mod gamepad;
mod icons;
mod images;
mod kit;
mod mc_text;
mod link_profile;
mod screens;
mod toasts;

/// How long after a graphics-device switch the icon textures are made once more.
const SVG_SETTLE_DELAY: Duration = Duration::from_millis(600);

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use config::{Config, ConfigFormat, LoadConfigResult, SettingsDomain, TextRenderingPath};
use installation::{normalize_path_key, take_finished_instance_processes};
use instances::{InstanceStore, delete_instance, instance_root_path, load_store};
use launcher_ui::app_commands::AppCommand;
use crate::app::se5::file_dialog::FileDialogs;
use launcher_ui::notification;
use launcher_ui::screens::home_api::HomeSession;
use launcher_ui::screens::instance_api as api;
use launcher_ui::screens::instance_api::InstanceSession;
use launcher_ui::screens::{
    AppScreen, ContentBrowserState, DiscoverState, LibraryRuntimeState, MenuPresenceContext,
};
use vertex_shell::palette::Palette;
use vertex_shell::sturdy::glam::Vec2;
use vertex_shell::sturdy::ui::Ui;
use vertex_shell::sturdy::ui_kit::PopupState;
use vertex_shell::sturdy::widgets::TextRasterMode;
use vertex_shell::sturdy::window::{DropEvent, HitTestRegions, WindowRect};
use vertex_shell::{Applied, SettingsBridge, ShellConfig};
use vertex_theme::Theme;
use vertex_theme::ThemeCatalog;

use super::InitialInstanceInstallResult;
use super::appearance::effective_ui_opacity_percent;
use super::auth_state::AuthState;
use super::discord_presence::DiscordPresenceManager;
use super::obs_detection::{ObsChange, ObsWatcher};

use self::fonts::{FontOptions, FontSignature};
use self::kit::Kit;

/// Starts the SE5 front end and runs it until the window closes.
pub(super) fn run(config_state: LoadConfigResult) -> ExitCode {
    // Before anything slow, so a launcher that never gets going still logs how far it got.
    vertex_shell::watchdog::start();
    vertex_shell::watchdog::step("launcher started, loading settings");
    let (mut config, first_run_format) = match config_state {
        LoadConfigResult::Loaded(config) => (config, None),
        LoadConfigResult::Missing { default_format } => {
            let mut config = Config::default();
            // A Steam Deck's game mode needs the window out of the game's way, so a first run there has it on.
            config.set_steam_deck_handoff(config::running_on_steam_deck());
            (config, Some(default_format))
        }
    };
    config.normalize();
    // Read by account loading, so it is set before `AuthState::load`.
    auth::set_insecure_file_storage(config.insecure_file_account_storage());
    crate::app::tracing_setup::apply_console_log_levels(
        config.console_log_level(),
        config.game_console_log_level(),
    );
    apply_curseforge_key(&config);

    vertex_shell::watchdog::step("settings ready, building the app state");
    let bridge = SettingsBridge::new();
    let exit_snapshot = Arc::new(Mutex::new(None));
    let mut app = App::new(
        config.clone(),
        first_run_format,
        bridge.clone(),
        exit_snapshot.clone(),
    );
    vertex_shell::watchdog::step("app state built (themes, instances, accounts started)");
    vertex_shell::install_game_probe(|| !installation::running_instance_roots().is_empty());
    // CPU-bound work (the skin preview's rasteriser, image decoding) runs on the UI engine's own task scheduler, so it
    // uses exactly the machine's cores next to the engine's jobs instead of a second set of threads.
    launcher_runtime::install_compute_backend(launcher_runtime::ComputeBackend {
        spawn: |task| {
            // Dropping the handle detaches the task; its result goes through its own channel.
            drop(vertex_shell::sturdy::task::spawn(task));
        },
        workers: || vertex_shell::sturdy::diagnostics::worker_count() as usize,
    });
    // Waits ride on the UI engine's timer reactor instead of the runtime's own timer thread. If the engine cannot
    // supply one for a wait, that wait falls back to the thread.
    launcher_runtime::install_async_backend(launcher_runtime::AsyncBackend {
        sleep: |duration| {
            Box::pin(async move { vertex_shell::sturdy::net::sleep(duration).await.is_ok() })
        },
    });
    #[cfg(target_os = "macos")]
    super::app_icon::apply_macos_dock_icon();

    let mut shell = ShellConfig::new("Vertex Launcher", "org.sturdyfool10.VertexLauncher");
    shell.size = (1280, 820);
    tracing::info!(target: "vertexlauncher/se5", "Starting the SturdyEngine5 front end.");
    let code = vertex_shell::run(shell, config, bridge, move |ui| app.draw(ui));
    save_on_exit(&exit_snapshot);
    code
}

/// The CurseForge key from the settings overrides the one in the environment.
fn apply_curseforge_key(config: &Config) {
    curseforge::set_api_key_override(
        (!config.curseforge_api_key().trim().is_empty())
            .then(|| config.curseforge_api_key().to_owned()),
    );
}

fn save_on_exit(snapshot: &Mutex<Option<(Config, InstanceStore)>>) {
    let Some((config, store)) = snapshot
        .lock()
        .ok()
        .and_then(|mut snapshot| snapshot.take())
    else {
        return;
    };
    if let Err(err) = config::save_config(&config) {
        tracing::error!(target: "vertexlauncher/se5/save", "Failed to save settings on exit: {err}");
    }
    if let Err(err) = instances::save_store(&store) {
        tracing::error!(target: "vertexlauncher/se5/save", "Failed to save instances on exit: {err}");
    }
}

/// Pending dialogs. Only one is open at a time; the newest request wins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// First launch: choose the config file format.
    ConfigFormat(ConfigFormat),
    CreateInstance,
    Import,
    SignIn,
    ConfirmDelete(String),
    /// The screenshot viewer, showing one file.
    Screenshot(std::path::PathBuf),
    InstanceSettings(String),
    ExportPack(String),
    ExportServer(String),
    MoveInstance(String),
    /// A screenshot from an instance's gallery, with delete.
    InstanceScreenshot(String, std::path::PathBuf),
    /// A screenshot from Home's cross-instance gallery, by key.
    HomeScreenshot(String),
    /// Confirmation before unsecured account storage is switched on.
    ConfirmInsecureStorage,
    /// The game settings (`options.txt`) editor for an instance; its state is `AppState::game_settings`.
    GameSettings(String),
    GamepadCalibration,
    ServerSync,
    WorldSync,
    /// Rename, delete, move or copy a Home world or server card; its state is `AppState::manage`.
    ManageEntry,
}

/// Background saves of the config and instance store: at most one write in flight, the newest state wins.
struct Saver<T: Clone + Send + 'static> {
    pending: Option<T>,
    in_flight: bool,
    results: launcher_runtime::WorkerChannel<Result<(), String>>,
    what: &'static str,
    save: fn(&T) -> Result<(), String>,
}

impl<T: Clone + Send + 'static> Saver<T> {
    fn new(what: &'static str, save: fn(&T) -> Result<(), String>) -> Self {
        Self {
            pending: None,
            in_flight: false,
            results: Default::default(),
            what,
            save,
        }
    }

    fn queue(&mut self, value: &T) {
        self.pending = Some(value.clone());
        self.start();
    }

    fn start(&mut self) {
        if self.in_flight {
            return;
        }
        let Some(value) = self.pending.take() else {
            return;
        };
        let tx = self.results.sender();
        let save = self.save;
        self.in_flight = true;
        launcher_runtime::spawn_blocking_detached(move || {
            let _ = tx.send(save(&value));
        });
    }

    fn poll(&mut self) {
        let drained = self.results.drain();
        if drained.disconnected {
            self.in_flight = false;
        }
        for result in drained.items {
            self.in_flight = false;
            if let Err(err) = result {
                tracing::error!(target: "vertexlauncher/se5/save", "Failed to save {}: {err}", self.what);
                notification::error!("save", "Failed to save {}: {}", self.what, err);
            }
        }
        self.start();
    }

}

/// Everything the screens read and change. Plain data; the draw closure owns it.
pub struct AppState {
    pub config: Config,
    pub theme_catalog: ThemeCatalog,
    pub theme: Theme,
    pub screen: AppScreen,
    pub selected_instance_id: Option<String>,
    pub instances: InstanceStore,
    pub auth: AuthState,
    pub runtime: LibraryRuntimeState,
    pub create: create_instance::CreateState,
    pub dialog: Option<Dialog>,
    /// One session per instance opened this run; background work (exports, moves) keeps running off-screen.
    pub sessions: HashMap<String, InstanceSession>,
    pub home: HomeSession,
    pub home_tab: screens::home::HomeTab,
    pub home_menu: PopupState,
    pub home_menu_target: Option<String>,
    pub add_content_menu: PopupState,
    pub confirming_screenshot_delete: Option<String>,
    /// A screenshot delete was started from the dialog; checked once when it finishes.
    pub screenshot_delete_requested: bool,
    pub discover: DiscoverState,
    pub content: ContentBrowserState,
    pub flows: flows::Flows,
    pub file_dialogs: FileDialogs,
    pub font_options: FontOptions,
    pub console: screens::console::ConsoleView,
    pub settings: screens::settings::SettingsView,
    /// Whether the Better Blur DX KWin effect is on this machine (Linux), refreshed in the background.
    pub better_blur: super::better_blur::BetterBlurWatch,
    pub game_settings: Option<screens::game_settings::GameSettingsState>,
    pub gamepad: gamepad::Gamepad,
    pub viewer: screens::viewer::ViewerState,
    pub calibration: super::gamepad_calibration_modal::GamepadCalibrationState,
    pub skins: launcher_ui::screens::skins_api::SkinsSession,
    pub skins_view: screens::skins::SkinsView,
    pub server_sync: Option<screens::sync_dialogs::ServerSyncState>,
    pub world_sync: Option<screens::sync_dialogs::WorldSyncState>,
    pub manage: Option<screens::manage_dialogs::ManageState>,
    pub account_menu: PopupState,
    pub accounts_view: accounts::AccountsView,
    pub link_results: launcher_runtime::WorkerChannel<link_profile::Detected>,
    pub profile_menu: PopupState,
    pub instance_menu: PopupState,
    pub instance_menu_target: Option<String>,
    /// A sign-in flow was running while the sign-in dialog was open.
    pub sign_in_running: bool,
    pub started: Instant,
    font_scan: Option<mpsc::Receiver<FontOptions>>,
    /// Themes-folder fingerprint and when it was last checked, for live theme reload.
    theme_watch: (u64, std::time::Instant),
    /// Folder fingerprints computed off the UI thread, and whether one is being computed.
    theme_scan: launcher_runtime::WorkerChannel<(u64, Option<ThemeCatalog>)>,
    theme_scan_in_flight: bool,
    applied_font: Option<FontSignature>,
    /// The config last handed to the engine appliers.
    submitted: Config,
    /// The config before that; a failed apply restores the failed domain's fields from it.
    before_submit: Config,
    saved_instances: InstanceStore,
    config_saver: Saver<Config>,
    store_saver: Saver<InstanceStore>,
    initial_install: launcher_runtime::WorkerChannel<InitialInstanceInstallResult>,
    /// Screenshot files read for the clipboard: (is PNG, bytes or error).
    image_copies: launcher_runtime::WorkerChannel<Result<Vec<u8>, String>>,
    discord: DiscordPresenceManager,
    obs: ObsWatcher,
    /// The latest config and instances, written synchronously once the window closes so a background save that the
    /// exit cut short cannot lose them.
    exit_snapshot: Arc<Mutex<Option<(Config, InstanceStore)>>>,
}

impl AppState {
    /// Streamer mode is on in the settings, or OBS is running (that part is never written to the config).
    pub fn streamer_mode(&self) -> bool {
        self.config.streamer_mode_enabled() || self.obs.is_active()
    }

    pub fn installations_root(&self) -> std::path::PathBuf {
        self.config
            .minecraft_installations_root_path()
            .to_path_buf()
    }

    pub fn open(&mut self, screen: AppScreen) {
        self.screen = screen;
    }

    pub fn open_instance(&mut self, id: &str) {
        self.selected_instance_id = Some(id.to_owned());
        self.screen = AppScreen::Instance;
    }

    /// Creates the session for `instance` the first time it is opened.
    pub fn ensure_session(&mut self, instance: &instances::InstanceRecord) {
        if !self.sessions.contains_key(&instance.id) {
            self.sessions.insert(
                instance.id.clone(),
                InstanceSession::new(instance, &self.config),
            );
        }
    }

    pub fn open_export(&mut self, id: &str, kind: screens::instance_dialogs::ExportKind) {
        use screens::instance_dialogs::ExportKind;
        let Some(session) = self.sessions.get_mut(id) else {
            return;
        };
        match kind {
            ExportKind::Pack | ExportKind::Patch => {
                session.state.export_vtmpack_patch_mode = kind == ExportKind::Patch;
                session.state.show_export_vtmpack_modal = true;
                self.dialog = Some(Dialog::ExportPack(id.to_owned()));
            }
            ExportKind::Server => {
                session.state.show_export_server_modal = true;
                self.dialog = Some(Dialog::ExportServer(id.to_owned()));
            }
        }
    }

    pub fn open_move(&mut self, id: &str) {
        if let Some(instance) = self.instances.find(id).cloned()
            && let Some(session) = self.sessions.get_mut(id)
        {
            api::settings::prepare_move(
                &mut session.state,
                &self.instances,
                &self.config,
                &instance.id,
            );
            self.dialog = Some(Dialog::MoveInstance(id.to_owned()));
        }
    }

    /// Polls every open instance's background work, and drops sessions of deleted instances.
    fn tick_sessions(&mut self) {
        self.sessions
            .retain(|id, _| self.instances.find(id).is_some());
        let AppState {
            sessions,
            config,
            instances,
            ..
        } = self;
        for session in sessions.values_mut() {
            session.tick(config, instances);
        }
    }

    pub fn initial_install_sender(&mut self) -> mpsc::Sender<InitialInstanceInstallResult> {
        self.initial_install.sender()
    }

    fn poll(&mut self) {
        self.auth.poll();
        launcher_ui::screens::poll_library_runtime_actions(
            &mut self.runtime,
            &mut self.config,
            &mut self.instances,
        );
        launcher_ui::screens::poll_library_delete_results(&mut self.runtime, &mut self.instances);
        create_instance::poll(self);
        self.poll_flows();
        link_profile::poll(self);
        self.tick_sessions();
        self.poll_initial_installs();
        self.poll_finished_processes();
        self.config_saver.poll();
        self.store_saver.poll();
        if let Some(rx) = &self.font_scan
            && let Ok(options) = rx.try_recv()
        {
            self.font_options = options;
            self.font_scan = None;
        }
        self.poll_obs();
        self.better_blur.poll();
        self.better_blur
            .ensure_covered(self.config.linux_blur_protocol() == config::LinuxBlurProtocol::BetterBlurDx);
        // The effect is not here (never was, or was removed), so a saved choice of it goes back to Auto: it is not
        // offered, so it would otherwise sit selected but invisible.
        if self.better_blur.settled()
            && !self.better_blur.available()
            && self.config.linux_blur_protocol() == config::LinuxBlurProtocol::BetterBlurDx
        {
            self.config.set_linux_blur_protocol(config::LinuxBlurProtocol::Auto);
        }
        self.poll_themes();
        let streamer_mode = self.streamer_mode();
        notification::set_streamer_mode(streamer_mode);
        // Everything the UI draws is filtered through this, so names stay hidden in streamer mode.
        launcher_ui::privacy::update_hidden_accounts(streamer_mode, self.auth.account_identities());
        self.poll_screenshots();
    }

    /// Picks up theme files added, edited or removed while the launcher runs.
    fn poll_themes(&mut self) {
        const INTERVAL: Duration = Duration::from_secs(1);
        let elapsed = self.theme_watch.1.elapsed();
        if elapsed < INTERVAL {
            return;
        }
        self.theme_watch.1 = std::time::Instant::now();
        // The folder is looked at on a worker; the UI only ever reads the answer.
        let results = self.theme_scan.drain();
        if let Some((signature, catalog)) = results.items.into_iter().last() {
            self.theme_scan_in_flight = false;
            if let Some(catalog) = catalog {
                self.theme_watch.0 = signature;
                self.theme_catalog = catalog;
                self.sync_theme();
            }
        }
        if !self.theme_scan_in_flight {
            self.theme_scan_in_flight = true;
            let tx = self.theme_scan.sender();
            let known = self.theme_watch.0;
            launcher_runtime::spawn_blocking_detached(move || {
                let signature = ThemeCatalog::directory_signature();
                let catalog = (signature != known).then(ThemeCatalog::load);
                let _ = tx.send((signature, catalog));
            });
        }
    }

    fn poll_screenshots(&mut self) {
        let playing_as = if installation::running_instance_roots().is_empty() {
            None
        } else {
            self.auth
                .active_launch_context()
                .map(|context| (context.player_uuid, context.player_name))
        };
        launcher_ui::screenshot_watch::poll(
            &self.instances,
            &self.config,
            playing_as
                .as_ref()
                .map(|(id, name)| launcher_ui::account_text::AccountMention { id, name }),
        );
    }

    /// Reads the image on the launcher runtime; `handle_commands` puts it on the clipboard once it arrives.
    fn start_copy_image(&mut self, path: std::path::PathBuf) {
        let is_png = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png"));
        let tx = self.image_copies.sender();
        launcher_runtime::spawn_blocking_detached(move || {
            let result = if is_png {
                std::fs::read(&path).map_err(|err| err.to_string())
            } else {
                Err("Only PNG screenshots can be copied to the clipboard.".to_owned())
            };
            let _ = tx.send(result);
        });
    }

    /// Handles what notification buttons and background code asked the UI to do.
    fn handle_commands(&mut self, ui: &mut Ui<'_>) {
        // F11 toggles fullscreen, in the style chosen in Settings. (Read by the shell, which sees every tick.)
        if vertex_shell::take_fullscreen_key() {
            chrome::toggle_fullscreen(self.config.fullscreen_style());
        }
        if let Some(tab) = launcher_ui::console::take_focus_request()
            && self.config.follow_console_on_launch()
        {
            launcher_ui::console::set_active_tab(&tab);
            self.screen = AppScreen::Console;
        }
        for result in self.image_copies.drain().items {
            match result {
                Ok(bytes) => copy_image(ui, &bytes),
                Err(err) => notification::error!("screenshots", "Failed to copy screenshot: {}", err),
            }
        }
        let drops = ui.engine().take_drop_events();
        for event in drops {
            if let DropEvent::File { path, .. } = event {
                if self.screen == AppScreen::Skins && self.dialog.is_none() {
                    self.skins.load_skin_path(path);
                } else {
                    self.handle_dropped_file(path);
                }
            }
        }
        for command in launcher_ui::app_commands::drain() {
            match command {
                AppCommand::ViewScreenshot(path) => self.dialog = Some(Dialog::Screenshot(path)),
                AppCommand::CopyImage(path) => self.start_copy_image(path),
                AppCommand::CopyText(text) => {
                    let _ = ui
                        .engine()
                        .primary_window_control()
                        .map(|mut window| window.set_clipboard_text(&text));
                }
            }
        }
    }

    /// Follows OBS starting and stopping, with a notification each way unless streamer mode is switched on anyway.
    fn poll_obs(&mut self) {
        let detection_enabled = self.config.obs_detection_enabled();
        self.obs.set_enabled(detection_enabled);
        let change = self.obs.poll_change();
        if self.config.streamer_mode_enabled() {
            return;
        }
        match change {
            Some(ObsChange::Started) => {
                notification::info!(
                    "Streamer Mode",
                    "Streamer mode was turned on because OBS was detected."
                )
            }
            Some(ObsChange::Stopped) if detection_enabled => {
                notification::info!(
                    "Streamer Mode",
                    "Streamer mode was turned off because OBS is no longer running."
                )
            }
            _ => {}
        }
    }

    fn poll_initial_installs(&mut self) {
        for update in self.initial_install.drain().items {
            let InitialInstanceInstallResult::Failed {
                instance_id,
                instance_name,
                error,
            } = update;
            let root = self.installations_root();
            match delete_instance(&mut self.instances, instance_id.as_str(), root.as_path()) {
                Ok(_) => notification::error!(
                    format!("installation/{instance_name}"),
                    "{}: initial install failed and the incomplete instance was removed: {}",
                    instance_name,
                    error
                ),
                Err(delete_err) => notification::error!(
                    format!("installation/{instance_name}"),
                    "{}: initial install failed: {}. Cleanup also failed: {}",
                    instance_name,
                    error,
                    delete_err
                ),
            }
        }
    }

    fn poll_finished_processes(&mut self) {
        let finished = take_finished_instance_processes();
        if !finished.is_empty() {
            // The game rewrites servers, hotbars and history on exit; fan the results out.
            launcher_ui::sync_runner::spawn(
                self.instances.clone(),
                launcher_ui::sync_runner::SyncRunConfig::from_config(&self.config),
            );
        }
        let root = self.installations_root();
        for process in finished {
            if process.exit_code != Some(1) {
                continue;
            }
            let name = self.instances.instances.iter().find_map(|instance| {
                (normalize_path_key(instance_root_path(root.as_path(), instance).as_path())
                    == process.instance_root)
                    .then(|| instance.name.clone())
            });
            match name {
                Some(name) => {
                    notification::error!(
                        format!("instance/crash/{}", process.pid),
                        "{} crashed with exit code 1.",
                        name
                    )
                }
                None => notification::error!(
                    format!("instance/crash/{}", process.pid),
                    "An instance crashed with exit code 1: {}",
                    process.instance_root
                ),
            }
        }
    }

    /// Saves and applies config changes, saves instance-store changes, and keeps selection valid.
    fn commit(&mut self, bridge: &SettingsBridge) {
        self.config.normalize();
        if self.config != self.submitted {
            if self.config.theme_id() != self.submitted.theme_id() {
                self.sync_theme();
            }
            if self.config.curseforge_api_key() != self.submitted.curseforge_api_key() {
                apply_curseforge_key(&self.config);
            }
            bridge.submit(self.config.clone());
            self.config_saver.queue(&self.config);
            self.before_submit = std::mem::replace(&mut self.submitted, self.config.clone());
        }
        for (domain, result) in bridge.take_results() {
            self.report_apply(domain, result, bridge);
        }

        self.instances.normalize();
        if self.instances != self.saved_instances {
            if self
                .selected_instance_id
                .as_deref()
                .is_some_and(|id| self.instances.find(id).is_none())
            {
                self.selected_instance_id = self.instances.instances.first().map(|i| i.id.clone());
                if self.selected_instance_id.is_none() && self.screen == AppScreen::Instance {
                    self.screen = AppScreen::Library;
                }
            }
            self.store_saver.queue(&self.instances);
            self.saved_instances = self.instances.clone();
        }
        if let Ok(mut snapshot) = self.exit_snapshot.lock() {
            let stale = snapshot
                .as_ref()
                .is_none_or(|(config, store)| *config != self.config || *store != self.instances);
            if stale {
                *snapshot = Some((self.config.clone(), self.instances.clone()));
            }
        }

        let presence = MenuPresenceContext::Screen(self.screen);
        let root = self.installations_root();
        self.discord.update(
            &self.config,
            &self.instances,
            root.as_path(),
            presence,
            self.selected_instance_id.as_deref(),
        );
    }

    fn report_apply(&mut self, domain: SettingsDomain, result: Applied, bridge: &SettingsBridge) {
        let what = match domain {
            SettingsDomain::GraphicsDevice => "graphics device",
            SettingsDomain::WindowEffect => "window effect",
            SettingsDomain::Presentation => "display settings",
            _ => "setting",
        };
        match result {
            Applied::Ok => {}
            // A fallback (blur unavailable, say) still leaves a working window; the reason goes to the log only.
            Applied::Degraded(reason) => {
                tracing::info!(target: "vertexlauncher/se5/settings", "The {what} was applied with a fallback: {reason}");
            }
            Applied::Failed(reason) => {
                notification::error!(
                    "settings",
                    "Could not apply the {}; the previous choice is back: {}",
                    what,
                    reason
                );
                // The engine kept running on the previous values; put them back in the config so it matches.
                let previous = self.before_submit.clone();
                restore_domain(&mut self.config, &previous, domain);
                self.submitted = self.config.clone();
                bridge.submit(self.config.clone());
                self.config_saver.queue(&self.config);
            }
        }
    }

    /// Closes whatever popup or menu is open (the back button's first job). True if there was one.
    fn close_popups(&mut self, kit: &mut Kit) -> bool {
        let mut closed = kit.close_search_popups();
        for menu in [
            &mut self.account_menu,
            &mut self.instance_menu,
            &mut self.home_menu,
            &mut self.add_content_menu,
            &mut self.profile_menu,
        ] {
            if menu.is_open() {
                menu.close();
                closed = true;
            }
        }
        closed
    }

    fn sync_theme(&mut self) {
        if !self.theme_catalog.contains(self.config.theme_id()) {
            self.config
                .set_theme_id(self.theme_catalog.default_theme_id().to_owned());
        }
        self.theme = self.theme_catalog.resolve(self.config.theme_id()).clone();
    }
}

/// Names whatever holds the UI thread. The draw closure runs on it, so anything slow in here is a frozen window;
/// a frame over [`StallWatch::LIMIT`] logs how long each phase took, which points straight at the offender.
struct StallWatch {
    started: Instant,
    last: Instant,
    phases: Vec<(&'static str, Duration)>,
}

impl StallWatch {
    const LIMIT: Duration = Duration::from_millis(50);

    fn new(now: Instant) -> Self {
        Self { started: now, last: now, phases: Vec::with_capacity(8) }
    }

    fn phase(&mut self, name: &'static str) {
        let now = Instant::now();
        self.phases.push((name, now.duration_since(self.last)));
        self.last = now;
        vertex_shell::watchdog::step(name);
    }

    fn finish(&self) {
        let total = self.last.duration_since(self.started);
        if total >= Self::LIMIT {
            let breakdown: Vec<String> =
                self.phases.iter().map(|(name, took)| format!("{name}={took:.1?}")).collect();
            tracing::warn!(
                target: "vertexlauncher/se5/stall",
                "The UI thread spent {total:.1?} in one frame ({}). Move that work to a worker.",
                breakdown.join(", ")
            );
        }
    }
}

struct App {
    state: AppState,
    kit: Kit,
    bridge: Arc<SettingsBridge>,
    last_frame: Instant,
    window_feedback: WindowFeedback,
    /// The "start in fullscreen" setting has been acted on.
    started_fullscreen: bool,
}

impl App {
    fn new(
        config: Config,
        first_run_format: Option<ConfigFormat>,
        bridge: Arc<SettingsBridge>,
        exit_snapshot: Arc<Mutex<Option<(Config, InstanceStore)>>>,
    ) -> Self {
        let obs_enabled = config.obs_detection_enabled();
        let theme_catalog = ThemeCatalog::load();
        let mut config = config;
        if !theme_catalog.contains(config.theme_id()) {
            config.set_theme_id(theme_catalog.default_theme_id().to_owned());
        }
        let theme = theme_catalog.resolve(config.theme_id()).clone();
        let instances = match load_store() {
            Ok(store) => store,
            Err(err) => {
                notification::error!("instance_store", "Failed to load instance store: {err}");
                InstanceStore::default()
            }
        };
        if super::app_metadata::try_settings_info().is_none() {
            let _ = launcher_runtime::spawn_blocking_detached(
                super::app_metadata::preload_settings_info,
            );
        }
        launcher_ui::account_colors::warm();
        let palette = Palette::new(&theme, effective_ui_opacity_percent(&config));
        let kit = Kit::new(palette, config.ui_font_size());
        let selected_instance_id = instances
            .instances
            .first()
            .map(|instance| instance.id.clone());
        let state = AppState {
            submitted: config.clone(),
            before_submit: config.clone(),
            saved_instances: instances.clone(),
            config,
            theme_catalog,
            theme,
            screen: AppScreen::Home,
            selected_instance_id,
            instances,
            auth: AuthState::load(),
            runtime: LibraryRuntimeState::default(),
            create: create_instance::CreateState::default(),
            dialog: first_run_format.map(Dialog::ConfigFormat),
            sessions: HashMap::new(),
            home: HomeSession::default(),
            home_tab: screens::home::HomeTab::default(),
            home_menu: PopupState::default(),
            home_menu_target: None,
            add_content_menu: PopupState::default(),
            confirming_screenshot_delete: None,
            screenshot_delete_requested: false,
            discover: DiscoverState::default(),
            content: ContentBrowserState::default(),
            flows: flows::Flows::default(),
            file_dialogs: FileDialogs::default(),
            font_options: FontOptions::included_only(),
            console: screens::console::ConsoleView::default(),
            settings: screens::settings::SettingsView::default(),
            better_blur: Default::default(),
            game_settings: None,
            gamepad: Default::default(),
            viewer: Default::default(),
            calibration: Default::default(),
            skins: Default::default(),
            skins_view: Default::default(),
            server_sync: None,
            world_sync: None,
            manage: None,
            account_menu: PopupState::default(),
            accounts_view: Default::default(),
            link_results: Default::default(),
            profile_menu: PopupState::default(),
            instance_menu: PopupState::default(),
            instance_menu_target: None,
            sign_in_running: false,
            started: Instant::now(),
            font_scan: Some(FontOptions::scan()),
            theme_watch: (
                ThemeCatalog::directory_signature(),
                std::time::Instant::now(),
            ),
            theme_scan: Default::default(),
            theme_scan_in_flight: false,
            applied_font: None,
            config_saver: Saver::new("settings", |config| {
                config::save_config(config).map_err(|err| err.to_string())
            }),
            store_saver: Saver::new("instances", |store| {
                instances::save_store(store).map_err(|err| err.to_string())
            }),
            initial_install: Default::default(),
            image_copies: Default::default(),
            discord: DiscordPresenceManager::default(),
            obs: ObsWatcher::spawn(obs_enabled),
            exit_snapshot,
        };
        Self {
            state,
            kit,
            bridge,
            last_frame: Instant::now(),
            window_feedback: WindowFeedback::default(),
            started_fullscreen: false,
        }
    }

    fn draw(&mut self, ui: &mut Ui<'_>) {
        let now = Instant::now();
        self.kit.delta_seconds = now.duration_since(self.last_frame).as_secs_f32().min(0.25);
        self.last_frame = now;

        let mut stalls = StallWatch::new(now);
        self.state.poll();
        stalls.phase("poll");
        self.state.handle_commands(ui);
        stalls.phase("commands");
        gamepad::frame(ui, &mut self.kit, &mut self.state);
        stalls.phase("gamepad");
        let escape_seen = vertex_shell::take_escape_key();
        let mut closed_a_menu = false;
        if self.kit.back_pressed() && self.state.close_popups(&mut self.kit) {
            self.kit.consume_back();
            closed_a_menu = true;
        }
        // Escape leaves fullscreen, but not in the same press that closed a menu, and not while a dialog is open (it
        // takes Escape itself). The shell's view of the key covers a press in a tick the UI did not see.
        if !closed_a_menu
            && (self.kit.back_pressed() || escape_seen)
            && vertex_shell::fullscreen() != vertex_shell::Fullscreen::Off
            && self.state.dialog.is_none()
        {
            vertex_shell::request_fullscreen(vertex_shell::Fullscreen::Off);
            self.kit.consume_back();
        }
        if !self.started_fullscreen {
            self.started_fullscreen = true;
            if self.state.config.start_fullscreen() {
                chrome::enter_fullscreen(self.state.config.fullscreen_style());
            }
        }
        vertex_shell::set_handoff(self.state.config.steam_deck_handoff());
        stalls.phase("shell");
        self.kit.images.pump(&mut ui.engine());
        stalls.phase("images");
        self.kit.palette = Palette::new(
            &self.state.theme,
            effective_ui_opacity_percent(&self.state.config),
        );
        self.kit.font_size = self.state.config.ui_font_size();
        self.kit.typography = self.state.config.typography().clone();
        self.apply_fonts(ui);
        stalls.phase("fonts");

        let maximized = ui
            .engine()
            .primary_window_control()
            .is_some_and(|mut window| window.is_maximized());
        stalls.phase("setup");
        chrome::window(ui, &mut self.kit, &mut self.state, maximized);
        stalls.phase("screen");
        toasts::show(ui, &mut self.kit, &self.state.config);
        self.dialogs(ui);
        stalls.phase("dialogs");
        self.window_feedback.update(ui);
        screens::import::manual_download_dialog(ui, &mut self.kit, &mut self.state);
        update_hit_regions(ui);
        self.kit.resolve_cursor(ui);

        self.state.commit(&self.bridge);
        stalls.phase("commit");
        stalls.finish();
    }

    fn dialogs(&mut self, ui: &mut Ui<'_>) {
        let Some(dialog) = self.state.dialog.clone() else {
            return;
        };
        let keep_open = match dialog {
            Dialog::ConfigFormat(format) => {
                screens::first_run::config_format(ui, &mut self.kit, &mut self.state, format)
            }
            Dialog::CreateInstance => create_instance::dialog(ui, &mut self.kit, &mut self.state),
            Dialog::Import => screens::import::dialog(ui, &mut self.kit, &mut self.state),
            Dialog::SignIn => accounts::sign_in_dialog(ui, &mut self.kit, &mut self.state),
            Dialog::InstanceSettings(ref id) => {
                screens::instance_dialogs::settings(ui, &mut self.kit, &mut self.state, id)
            }
            Dialog::ExportPack(ref id) => {
                screens::instance_dialogs::export_pack(ui, &mut self.kit, &mut self.state, id)
            }
            Dialog::ExportServer(ref id) => {
                screens::instance_dialogs::export_server(ui, &mut self.kit, &mut self.state, id)
            }
            Dialog::MoveInstance(ref id) => {
                screens::instance_dialogs::move_instance(ui, &mut self.kit, &mut self.state, id)
            }
            Dialog::InstanceScreenshot(ref id, ref path) => {
                screens::instance_dialogs::screenshot(ui, &mut self.kit, &mut self.state, id, path)
            }
            Dialog::HomeScreenshot(ref key) => {
                screens::home::viewer(ui, &mut self.kit, &mut self.state, key)
            }
            Dialog::Screenshot(ref path) => {
                screens::screenshots::viewer(ui, &mut self.kit, &mut self.state, path)
            }
            Dialog::ConfirmDelete(ref id) => {
                screens::library::confirm_delete(ui, &mut self.kit, &mut self.state, id)
            }
            Dialog::GameSettings(ref id) => {
                screens::game_settings::dialog(ui, &mut self.kit, &mut self.state, id)
            }
            Dialog::GamepadCalibration => {
                screens::gamepad_calibration::dialog(ui, &mut self.kit, &mut self.state)
            }
            Dialog::ServerSync => {
                screens::sync_dialogs::server_dialog(ui, &mut self.kit, &mut self.state)
            }
            Dialog::WorldSync => {
                screens::sync_dialogs::world_dialog(ui, &mut self.kit, &mut self.state)
            }
            Dialog::ManageEntry => {
                screens::manage_dialogs::dialog(ui, &mut self.kit, &mut self.state)
            }
            Dialog::ConfirmInsecureStorage => {
                screens::settings::confirm_insecure_storage(ui, &mut self.kit, &mut self.state)
            }
        };
        if !keep_open && self.state.dialog.as_ref() == Some(&dialog) {
            self.state.dialog = None;
        }
    }

    /// Registers the font stack under font id 0 whenever the font choices change: a live font change. Also keeps the
    /// glyph raster mode in step with the Text rendering setting.
    fn apply_fonts(&mut self, ui: &mut Ui<'_>) {
        let mode = match self.state.config.text_rendering_path() {
            TextRenderingPath::Auto => TextRasterMode::Auto,
            // SE5 has no plain alpha-mask atlas; SDF is its single-channel equivalent.
            TextRenderingPath::AlphaMask | TextRenderingPath::Sdf => TextRasterMode::Sdf,
            TextRenderingPath::Msdf => TextRasterMode::Msdf,
        };
        // Icons are scaled down by the GPU's bilinear filter, which has no mipmaps here: it averages 2x2 texels, so
        // only a 2x raster is filtered cleanly. 3x and 4x skip texels and look worse than no supersampling, so the
        // higher modes stop at 2x (the rasteriser already anti-aliases analytically).
        let supersample = self.state.config.svg_aa_mode().supersample_scale().min(2);
        // A graphics-device switch (Vertex applies the saved GPU choice on the first frames) rebuilds the engine's
        // textures, and icons made right around it can draw as solid white blocks. A later switch, once the new device
        // has settled, always came out right, so the icons are made again: at the switch, and once more a moment after.
        let generation = ui.engine().device_generation();
        if generation != self.kit.svg_device_generation {
            self.kit.svg_device_generation = generation;
            self.kit.svg_rebuild_at = Some(Instant::now() + SVG_SETTLE_DELAY);
            ui.clear_svg_cache();
            icons::forget_textures();
        } else if self.kit.svg_rebuild_at.is_some_and(|at| Instant::now() >= at) {
            self.kit.svg_rebuild_at = None;
            ui.clear_svg_cache();
            icons::forget_textures();
        }
        if supersample != self.kit.svg_supersample {
            // Icons are rasterised at a multiple of their drawn size; the cached ones are the wrong size now.
            ui.clear_svg_cache();
            icons::forget_textures();
            self.kit.svg_supersample = supersample;
        }
        if ui.text_raster_mode() != mode {
            ui.set_text_raster_mode(mode);
        }
        let signature = FontSignature::of(&self.state.config);
        if self.state.applied_font.as_ref() == Some(&signature) {
            return;
        }
        let stack = fonts::stack_for(&self.state.config);
        if let Err(err) = ui.engine().register_ui_font_stack(0, &stack) {
            notification::error!("fonts", "Could not use the chosen font: {}", err);
        }
        for role in config::TextRole::ALL {
            let stack = fonts::stack_for_role(&self.state.config, role);
            if let Err(err) = ui
                .engine()
                .register_ui_font_stack(fonts::role_font_id(role), &stack)
            {
                notification::error!(
                    "fonts",
                    "Could not use the font for {}: {}",
                    role.label(),
                    err
                );
                break;
            }
        }
        mc_text::fonts_changed();
        self.kit.styled_faces = fonts::register_styled_faces(&mut ui.engine(), &self.state.config);
        self.state.applied_font = Some(signature);
    }
}

/// Copies the fields of `domain` from `from` into `config` (the rollback after a failed apply).
fn restore_domain(config: &mut Config, from: &Config, domain: SettingsDomain) {
    match domain {
        SettingsDomain::GraphicsDevice => {
            config.set_graphics_api_preference(from.graphics_api_preference());
            config.set_graphics_adapter_preference_type(from.graphics_adapter_preference_type());
            config.set_graphics_adapter_profile(from.graphics_adapter_profile());
            config.set_graphics_adapter_explicit_hash(from.graphics_adapter_explicit_hash());
        }
        SettingsDomain::WindowEffect => {
            config.set_window_transparency(from.window_transparency());
            config.set_windows_backdrop_type(from.windows_backdrop_type());
            config.set_linux_blur_protocol(from.linux_blur_protocol());
            config.set_ui_opacity_percent(from.ui_opacity_percent());
        }
        SettingsDomain::Presentation => {
            config.set_hdr_when_available(from.hdr_when_available());
            config.set_hdr_color_space(from.hdr_color_space());
            config.set_hdr_ui_white_scale(from.hdr_ui_white_scale());
            config.set_hdr_max_cll_nits(from.hdr_max_cll_nits());
            config.set_hdr_max_fall_nits(from.hdr_max_fall_nits());
            config.set_frame_limit_fps(from.frame_limit_fps());
            let limiter = from.frame_limiter_enabled();
            config.for_each_toggle_mut(|spec, value| {
                if spec.id == config::ToggleSettingId::FrameLimiterEnabled {
                    *value = limiter;
                }
            });
        }
        _ => {}
    }
}

/// The title bar drags the window; its controls stay clickable. Rebuilt every frame from the last layout.
fn update_hit_regions(ui: &mut Ui<'_>) {
    let rect = |bounds: (Vec2, Vec2)| WindowRect {
        x: bounds.0.x as i32,
        y: bounds.0.y as i32,
        width: bounds.1.x as i32,
        height: bounds.1.y as i32,
    };
    let Some(bar) = ui.element_bounds(chrome::TITLE_BAR_ID) else {
        return;
    };
    let regions = HitTestRegions {
        // Fullscreen, the title bar neither drags the window nor does its edge resize it.
        enabled: vertex_shell::fullscreen() == vertex_shell::Fullscreen::Off,
        resize_border: 6,
        drag: vec![rect(bar)],
        no_drag: chrome::TITLE_BAR_CONTROLS
            .iter()
            .filter_map(|id| ui.element_bounds(id))
            .map(rect)
            .collect(),
    };
    let _ = ui
        .engine()
        .primary_window_control()
        .map(|mut window| window.set_hit_test_regions(&regions));
}

/// Puts PNG bytes (what Minecraft writes screenshots as) on the clipboard.
fn copy_image(ui: &mut Ui<'_>, bytes: &[u8]) {
    let copied = ui
        .engine()
        .primary_window_control()
        .map(|mut window| window.set_clipboard_data("image/png", bytes))
        .unwrap_or_else(|| Err(vertex_shell::sturdy::Error::new("no window")));
    match copied {
        Ok(()) => notification::info!("screenshots", "Copied screenshot to clipboard."),
        Err(err) => notification::error!("screenshots", "Failed to copy screenshot: {}", err),
    }
}

/// What the window shows about the launcher: its icon, and the install progress in the title and the taskbar.
#[derive(Default)]
struct WindowFeedback {
    icon_set: bool,
    title: String,
    taskbar: Option<(vertex_shell::sturdy::window::TaskbarProgress, u32)>,
}

impl WindowFeedback {
    fn update(&mut self, ui: &mut Ui<'_>) {
        use vertex_shell::sturdy::window::TaskbarProgress;
        let activity = launcher_ui::install_activity::snapshot();
        let (title, taskbar) = match &activity {
            Some(activity) => {
                let fraction = if activity.total_files > 0 {
                    (activity.downloaded_files as f32 / activity.total_files as f32).clamp(0.0, 1.0)
                } else {
                    match activity.total_bytes {
                        Some(total) if total > 0 => {
                            (activity.downloaded_bytes as f32 / total as f32).clamp(0.0, 1.0)
                        }
                        _ => 0.0,
                    }
                };
                let eta = activity
                    .eta_seconds
                    .map(|eta| format!(" ETA {eta}s"))
                    .unwrap_or_default();
                (
                    format!(
                        "Vertex Launcher · Installing {}% · {:.1} MiB/s{eta}",
                        (fraction * 100.0).round() as u32,
                        activity.bytes_per_second / (1024.0 * 1024.0)
                    ),
                    Some((TaskbarProgress::Normal, (fraction * 100.0).round() as u32)),
                )
            }
            None => ("Vertex Launcher".to_owned(), None),
        };
        let mut engine = ui.engine();
        let Some(mut window) = engine.primary_window_control() else {
            return;
        };
        if !self.icon_set {
            self.icon_set = true;
            if let Some((width, height, rgba)) = super::app_icon::rgba() {
                let _ = window.set_icon(width, height, &rgba);
            }
        }
        if self.title != title {
            let _ = window.set_title(&title);
            self.title = title;
        }
        if self.taskbar != taskbar {
            let _ = match taskbar {
                Some((state, percent)) => {
                    window.set_taskbar_progress(state, percent as f32 / 100.0)
                }
                None => window.set_taskbar_progress(TaskbarProgress::None, 0.0),
            };
            self.taskbar = taskbar;
        }
    }
}
