use std::collections::{HashMap, HashSet};
#[cfg(target_os = "linux")]
use std::env;
use std::fs;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use config::Config;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use installation::{display_user_path, running_instance_roots};
use instances::{InstanceStore, instance_root_path};
use launcher_ui::screens::{
    AppScreen, HomePresenceSection, InstancePresenceSection, MenuPresenceContext,
};
use vertex_constants::branding::DISCORD_APPLICATION_ID;
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const PRESENCE_RESYNC_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
enum DesiredPresence {
    InGame {
        instance_id: String,
        instance_name: String,
        started_at_unix_secs: i64,
    },
    Menu {
        context: MenuPresenceContext,
        selected_instance_name: Option<String>,
    },
}

pub struct DiscordPresenceManager {
    client: Option<DiscordIpcClient>,
    session_start_by_instance_id: HashMap<String, i64>,
    active_presence: Option<DesiredPresence>,
    last_desired_presence: Option<DesiredPresence>,
    connected: bool,
    last_connect_attempt_at: Option<Instant>,
    last_connect_error: Option<String>,
    last_presence_sync_at: Option<Instant>,
}

impl Default for DiscordPresenceManager {
    fn default() -> Self {
        Self {
            client: None,
            session_start_by_instance_id: HashMap::new(),
            active_presence: None,
            last_desired_presence: None,
            connected: false,
            last_connect_attempt_at: None,
            last_connect_error: None,
            last_presence_sync_at: None,
        }
    }
}

impl DiscordPresenceManager {
    pub fn update(
        &mut self,
        config: &Config,
        instances: &InstanceStore,
        installations_root: &Path,
        menu_context: MenuPresenceContext,
        selected_instance_id: Option<&str>,
    ) {
        let running_roots = running_instance_roots();
        let desired = self.desired_presence(
            config,
            instances,
            installations_root,
            &running_roots,
            menu_context,
            selected_instance_id,
        );
        let should_resync = self
            .last_presence_sync_at
            .is_none_or(|last| last.elapsed() >= PRESENCE_RESYNC_INTERVAL);

        if desired == self.active_presence && !should_resync {
            return;
        }

        if desired.is_some()
            && !self.connected
            && !self.can_attempt_connect()
            && desired == self.last_desired_presence
        {
            return;
        }

        match desired.clone() {
            Some(next) => {
                self.last_desired_presence = Some(next.clone());
                log_presence_update_attempt(&next, should_resync);
                if let Err(err) = self.set_presence(&next) {
                    log_presence_update_failure(&next, &err);
                } else {
                    self.active_presence = Some(next);
                }
            }
            None => {
                if self.active_presence.is_some() {
                    tracing::debug!(
                        target: "vertexlauncher/discord_presence",
                        "Clearing Discord Rich Presence because no eligible running instance remains."
                    );
                }
                self.clear_presence();
                self.active_presence = None;
                self.last_desired_presence = None;
            }
        }
    }

    fn desired_presence(
        &mut self,
        config: &Config,
        instances: &InstanceStore,
        installations_root: &Path,
        running_roots: &[String],
        menu_context: MenuPresenceContext,
        selected_instance_id: Option<&str>,
    ) -> Option<DesiredPresence> {
        let running_set: HashSet<&str> = running_roots.iter().map(String::as_str).collect();
        let mut active_instance_ids = HashSet::new();

        let mut first_eligible = None;
        let mut launcher_presence_blocked_by_mod = false;
        for instance in &instances.instances {
            let instance_root = instance_root_path(installations_root, instance);
            let instance_key = fs::canonicalize(instance_root.as_path())
                .map(|path| display_user_path(path.as_path()))
                .unwrap_or_else(|_| display_user_path(instance_root.as_path()));
            if !running_set.contains(instance_key.as_str()) {
                continue;
            }
            active_instance_ids.insert(instance.id.clone());
            if instance.discord_rich_presence_mod_installed {
                launcher_presence_blocked_by_mod = true;
                continue;
            }
            if first_eligible.is_none() {
                let started_at_unix_secs = *self
                    .session_start_by_instance_id
                    .entry(instance.id.clone())
                    .or_insert_with(current_unix_timestamp_secs);
                first_eligible = Some(DesiredPresence::InGame {
                    instance_id: instance.id.clone(),
                    instance_name: instance.name.clone(),
                    started_at_unix_secs,
                });
            }
        }

        self.session_start_by_instance_id
            .retain(|instance_id, _| active_instance_ids.contains(instance_id));

        if !config.discord_rich_presence_enabled() {
            tracing::info!(
                target: "vertexlauncher/discord_presence",
                running_instances = active_instance_ids.len(),
                "Discord Rich Presence is disabled in config."
            );
            return None;
        }

        if let Some(presence) = first_eligible {
            return Some(presence);
        }

        if launcher_presence_blocked_by_mod {
            tracing::info!(
                target: "vertexlauncher/discord_presence",
                running_instances = active_instance_ids.len(),
                "Skipping launcher-owned Discord Rich Presence because a running instance provides its own Rich Presence mod."
            );
            return None;
        }

        if !active_instance_ids.is_empty() {
            tracing::info!(
                target: "vertexlauncher/discord_presence",
                running_instances = active_instance_ids.len(),
                "No eligible launcher-owned in-game Discord Rich Presence source was found."
            );
        }

        Some(self.menu_presence(menu_context, selected_instance_id, instances))
    }

    fn set_presence(&mut self, desired: &DesiredPresence) -> Result<(), String> {
        let activity = build_activity(desired);

        let initial_attempt = self
            .ensure_client_connected()?
            .set_activity(activity.clone())
            .map_err(|err| format!("failed to set Discord activity: {err}"));

        match initial_attempt {
            Ok(()) => {
                self.last_presence_sync_at = Some(Instant::now());
                log_presence_update_success(desired, false);
                Ok(())
            }
            Err(initial_err) => {
                log_presence_reconnect(desired);
                self.reset_client();
                self.ensure_client_connected()?
                    .set_activity(activity)
                    .map_err(|err| {
                        format!(
                            "{initial_err}; retry after reconnect also failed to set Discord activity: {err}"
                        )
                    })?;
                self.last_presence_sync_at = Some(Instant::now());
                log_presence_update_success(desired, true);
                Ok(())
            }
        }
    }

    fn menu_presence(
        &self,
        menu_context: MenuPresenceContext,
        selected_instance_id: Option<&str>,
        instances: &InstanceStore,
    ) -> DesiredPresence {
        let selected_instance_name = match menu_context {
            MenuPresenceContext::Instance(_) | MenuPresenceContext::Screen(AppScreen::Instance) => {
                selected_instance_id.and_then(|instance_id| {
                    instances
                        .find(instance_id)
                        .map(|instance| instance.name.clone())
                })
            }
            _ => None,
        };
        DesiredPresence::Menu {
            context: menu_context,
            selected_instance_name,
        }
    }

    fn clear_presence(&mut self) {
        let clear_failed = if let Some(client) = self.client.as_mut() {
            if let Err(err) = client.clear_activity() {
                tracing::warn!(
                    target: "vertexlauncher/discord_presence",
                    "Failed to clear Discord Rich Presence: {err}"
                );
                true
            } else {
                tracing::info!(
                    target: "vertexlauncher/discord_presence",
                    "Cleared Discord Rich Presence."
                );
                false
            }
        } else {
            false
        };

        if clear_failed {
            self.reset_client();
        }
        self.last_presence_sync_at = None;
    }

    fn ensure_client_connected(&mut self) -> Result<&mut DiscordIpcClient, String> {
        if self.client.is_none() {
            prepare_discord_ipc_environment();
            self.client = Some(DiscordIpcClient::new(DISCORD_APPLICATION_ID));
            tracing::info!(
                target: "vertexlauncher/discord_presence",
                application_id = DISCORD_APPLICATION_ID,
                "Created Discord IPC client."
            );
        }

        if self.connected {
            return self
                .client
                .as_mut()
                .ok_or_else(|| "Discord client missing".to_owned());
        }

        let should_retry = self.can_attempt_connect();

        if should_retry {
            self.last_connect_attempt_at = Some(Instant::now());
            tracing::info!(
                target: "vertexlauncher/discord_presence",
                "Attempting Discord IPC connection."
            );

            let connect_result = {
                let client = self
                    .client
                    .as_mut()
                    .ok_or_else(|| "Discord client missing".to_owned())?;
                client.connect()
            };

            connect_result.map_err(|err| {
                let message = format!("failed to connect to Discord IPC: {err}");
                self.last_connect_error = Some(message.clone());
                tracing::warn!(
                    target: "vertexlauncher/discord_presence",
                    error = %message,
                    "Discord IPC connection attempt failed."
                );
                message
            })?;
            self.connected = true;
            self.last_connect_error = None;
            tracing::info!(
                target: "vertexlauncher/discord_presence",
                "Connected to Discord IPC."
            );
        }

        if !self.connected {
            tracing::info!(
                target: "vertexlauncher/discord_presence",
                retry_interval_secs = CONNECT_RETRY_INTERVAL.as_secs(),
                previous_error = self.last_connect_error.as_deref().unwrap_or("unknown"),
                "Skipping Discord IPC reconnect attempt because the retry interval has not elapsed."
            );
            return Err(match self.last_connect_error.as_deref() {
                Some(previous) => format!(
                    "Discord IPC reconnect is rate-limited; waiting before retrying after previous failure: {previous}"
                ),
                None => "Discord IPC reconnect is rate-limited; waiting before retrying".to_owned(),
            });
        }

        self.client
            .as_mut()
            .ok_or_else(|| "Discord client missing".to_owned())
    }

    fn reset_client(&mut self) {
        if let Some(client) = self.client.as_mut() {
            let _ = client.close();
        }
        tracing::debug!(
            target: "vertexlauncher/discord_presence",
            "Resetting Discord IPC client state."
        );
        self.client = None;
        self.connected = false;
        self.last_connect_attempt_at = None;
        self.last_presence_sync_at = None;
    }

    fn can_attempt_connect(&self) -> bool {
        self.last_connect_attempt_at
            .is_none_or(|last| last.elapsed() >= CONNECT_RETRY_INTERVAL)
    }
}

#[cfg(target_os = "linux")]
fn prepare_discord_ipc_environment() {
    let runtime_dir = env::var("XDG_RUNTIME_DIR").ok();
    let candidate_dirs = discord_ipc_candidate_dirs();
    let candidate_dir_display = candidate_dirs
        .iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>();

    let Some(socket_dir) = find_discord_ipc_socket_dir(&candidate_dirs) else {
        tracing::info!(
            target: "vertexlauncher/discord_presence",
            xdg_runtime_dir = runtime_dir.as_deref().unwrap_or(""),
            candidate_dirs = ?candidate_dir_display,
            "No visible Discord IPC socket was found in any candidate directory."
        );
        return;
    };

    let socket_dir_display = socket_dir.display().to_string();
    // SAFETY: This is called on the UI thread before each Discord IPC client creation.
    // We only update process env vars used by the `discord-rich-presence` crate's socket lookup.
    unsafe {
        env::set_var("TMPDIR", &socket_dir);
    }
    tracing::info!(
        target: "vertexlauncher/discord_presence",
        socket_dir = %socket_dir_display,
        "Prepared Discord IPC environment override."
    );
}

#[cfg(not(target_os = "linux"))]
fn prepare_discord_ipc_environment() {}

#[cfg(target_os = "linux")]
fn discord_ipc_candidate_dirs() -> Vec<PathBuf> {
    let mut candidate_dirs = Vec::new();

    if let Ok(runtime_dir) = env::var("XDG_RUNTIME_DIR") {
        let runtime_dir = PathBuf::from(runtime_dir);
        candidate_dirs.push(runtime_dir.clone());
        candidate_dirs.push(runtime_dir.join("app/com.discordapp.Discord"));
        candidate_dirs.push(runtime_dir.join("app/com.discordapp.DiscordCanary"));
        candidate_dirs.push(runtime_dir.join("app/com.discordapp.DiscordPTB"));
        candidate_dirs.push(runtime_dir.join("app/dev.vencord.Vesktop"));
    }

    if let Ok(home) = env::var("HOME") {
        let home = PathBuf::from(home);
        candidate_dirs.push(home.join(".flatpak/com.discordapp.Discord/xdg-run"));
        candidate_dirs.push(home.join(".flatpak/com.discordapp.DiscordCanary/xdg-run"));
        candidate_dirs.push(home.join(".flatpak/com.discordapp.DiscordPTB/xdg-run"));
        candidate_dirs.push(home.join(".flatpak/dev.vencord.Vesktop/xdg-run"));
    }

    candidate_dirs
}

#[cfg(target_os = "linux")]
fn find_discord_ipc_socket_dir(candidate_dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in candidate_dirs {
        for index in 0..10 {
            if dir.join(format!("discord-ipc-{index}")).exists() {
                return Some(dir.clone());
            }
        }
    }

    None
}

fn build_activity(desired: &DesiredPresence) -> activity::Activity<'static> {
    match desired {
        DesiredPresence::InGame {
            instance_name,
            started_at_unix_secs,
            ..
        } => activity::Activity::new()
            .activity_type(activity::ActivityType::Playing)
            .details(format!("Playing {instance_name}"))
            .state("Launched via Vertex")
            .timestamps(activity::Timestamps::new().start(*started_at_unix_secs)),
        DesiredPresence::Menu {
            context,
            selected_instance_name,
        } => {
            let (details, state) = menu_status(*context, selected_instance_name.as_deref());
            activity::Activity::new()
                .activity_type(activity::ActivityType::Playing)
                .details(details)
                .state(state)
        }
    }
}

fn menu_status(
    context: MenuPresenceContext,
    selected_instance_name: Option<&str>,
) -> (String, &'static str) {
    match context {
        MenuPresenceContext::Home(HomePresenceSection::Activity) => {
            ("Browsing featured packs".to_owned(), "On the home screen")
        }
        MenuPresenceContext::Home(HomePresenceSection::Screenshots) => (
            "Reviewing screenshots".to_owned(),
            "In the screenshot manager",
        ),
        MenuPresenceContext::Instance(InstancePresenceSection::Content)
        | MenuPresenceContext::Screen(AppScreen::Instance) => (
            selected_instance_name
                .map(|name| format!("Managing {name}"))
                .unwrap_or_else(|| "Managing an instance".to_owned()),
            "In instance settings",
        ),
        MenuPresenceContext::Instance(InstancePresenceSection::Screenshots) => (
            selected_instance_name
                .map(|name| format!("Reviewing {name} screenshots"))
                .unwrap_or_else(|| "Reviewing instance screenshots".to_owned()),
            "In the screenshot gallery",
        ),
        MenuPresenceContext::Instance(InstancePresenceSection::Logs) => (
            selected_instance_name
                .map(|name| format!("Reading {name} logs"))
                .unwrap_or_else(|| "Reading instance logs".to_owned()),
            "In the logs viewer",
        ),
        MenuPresenceContext::Screen(AppScreen::Home) => {
            ("Browsing featured packs".to_owned(), "On the home screen")
        }
        MenuPresenceContext::Screen(AppScreen::Library) => {
            ("Managing instances".to_owned(), "In the library")
        }
        MenuPresenceContext::Screen(AppScreen::Discover) => {
            ("Browsing modpacks".to_owned(), "In Discover")
        }
        MenuPresenceContext::Screen(AppScreen::DiscoverDetail) => {
            ("Reviewing a modpack".to_owned(), "In Discover")
        }
        MenuPresenceContext::Screen(AppScreen::ContentBrowser) => {
            ("Adding content".to_owned(), "Managing mods and resources")
        }
        MenuPresenceContext::Screen(AppScreen::Skins) => {
            ("Customizing a skin".to_owned(), "In Skin Manager")
        }
        MenuPresenceContext::Screen(AppScreen::Settings) => {
            ("Adjusting settings".to_owned(), "Configuring Vertex")
        }
        MenuPresenceContext::Screen(AppScreen::Legal) => {
            ("Reviewing licenses".to_owned(), "In legal information")
        }
        MenuPresenceContext::Screen(AppScreen::Console) => {
            ("Checking logs".to_owned(), "In the console")
        }
    }
}

fn log_presence_update_attempt(desired: &DesiredPresence, should_resync: bool) {
    match desired {
        DesiredPresence::InGame {
            instance_id,
            instance_name,
            started_at_unix_secs,
        } => {
            let details = format!("Playing {instance_name}");
            tracing::info!(
                target: "vertexlauncher/discord_presence",
                instance_id = %instance_id,
                instance_name = %instance_name,
                started_at_unix_secs,
                should_resync,
                details = %details,
                state = "Launched via Vertex",
                "Updating in-game Discord Rich Presence."
            )
        }
        DesiredPresence::Menu {
            context,
            selected_instance_name,
        } => {
            let (details, state) = menu_status(*context, selected_instance_name.as_deref());
            tracing::info!(
                target: "vertexlauncher/discord_presence",
                context = ?context,
                selected_instance_name = selected_instance_name.as_deref().unwrap_or(""),
                should_resync,
                details = %details,
                state,
                "Updating menu Discord Rich Presence."
            )
        }
    }
}

fn log_presence_update_failure(desired: &DesiredPresence, err: &str) {
    match desired {
        DesiredPresence::InGame {
            instance_id,
            instance_name,
            ..
        } => tracing::warn!(
            target: "vertexlauncher/discord_presence",
            instance_id = %instance_id,
            instance_name = %instance_name,
            "Discord Rich Presence update failed: {err}"
        ),
        DesiredPresence::Menu {
            context,
            selected_instance_name,
        } => tracing::warn!(
            target: "vertexlauncher/discord_presence",
            context = ?context,
            selected_instance_name = selected_instance_name.as_deref().unwrap_or(""),
            "Discord Rich Presence update failed: {err}"
        ),
    }
}

fn log_presence_update_success(desired: &DesiredPresence, after_reconnect: bool) {
    match desired {
        DesiredPresence::InGame {
            instance_id,
            instance_name,
            ..
        } => tracing::info!(
            target: "vertexlauncher/discord_presence",
            instance_id = %instance_id,
            instance_name = %instance_name,
            after_reconnect,
            "Discord Rich Presence updated."
        ),
        DesiredPresence::Menu {
            context,
            selected_instance_name,
        } => tracing::info!(
            target: "vertexlauncher/discord_presence",
            context = ?context,
            selected_instance_name = selected_instance_name.as_deref().unwrap_or(""),
            after_reconnect,
            "Discord Rich Presence updated."
        ),
    }
}

fn log_presence_reconnect(desired: &DesiredPresence) {
    match desired {
        DesiredPresence::InGame {
            instance_id,
            instance_name,
            ..
        } => tracing::info!(
            target: "vertexlauncher/discord_presence",
            instance_id = %instance_id,
            instance_name = %instance_name,
            "Discord IPC session became stale; reconnecting and retrying in-game activity update."
        ),
        DesiredPresence::Menu {
            context,
            selected_instance_name,
        } => tracing::info!(
            target: "vertexlauncher/discord_presence",
            context = ?context,
            selected_instance_name = selected_instance_name.as_deref().unwrap_or(""),
            "Discord IPC session became stale; reconnecting and retrying menu activity update."
        ),
    }
}

fn current_unix_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
