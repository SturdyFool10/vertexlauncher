use vertex_i18n::tr;
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

#[path = "discord_presence/desired_presence.rs"]
mod desired_presence;
#[path = "discord_presence/discord_presence_manager.rs"]
mod discord_presence_manager;

use self::desired_presence::DesiredPresence;
pub use self::discord_presence_manager::DiscordPresenceManager;

/// What the manager asks the task to do.
enum Command {
    Set(DesiredPresence),
    Clear,
}

/// What the task reports back.
enum Event {
    Set {
        desired: DesiredPresence,
        result: Result<(), String>,
    },
    Cleared,
}

/// The background task that owns the Discord connection, and the ends of its two channels.
pub(super) struct PresenceWorker {
    commands: std::sync::mpsc::Sender<Command>,
    events: std::sync::mpsc::Receiver<Event>,
}

impl PresenceWorker {
    fn start() -> Self {
        let (commands, command_rx) = std::sync::mpsc::channel();
        let (event_tx, events) = std::sync::mpsc::channel();
        // A blocking task, on the launcher runtime's blocking pool: it waits on a socket, which must not hold up the
        // engine's own workers. It ends when the manager (the sender) is dropped.
        launcher_runtime::spawn_blocking_detached(move || worker_loop(command_rx, event_tx));
        Self { commands, events }
    }
}

fn worker_loop(commands: std::sync::mpsc::Receiver<Command>, events: std::sync::mpsc::Sender<Event>) {
    let mut client = PresenceClient::default();
    while let Ok(mut command) = commands.recv() {
        // Only the newest request matters; anything that queued up while this one was blocked is skipped.
        while let Ok(newer) = commands.try_recv() {
            command = newer;
        }
        let event = match command {
            Command::Set(desired) => {
                let result = client.set_presence(&desired);
                Event::Set { desired, result }
            }
            Command::Clear => {
                client.clear_presence();
                Event::Cleared
            }
        };
        if events.send(event).is_err() {
            break;
        }
    }
    client.reset();
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
        self.take_events();
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
        // One request at a time, and a failing one (Discord not running) waits out the retry interval.
        if self.in_flight {
            return;
        }
        if desired.is_some()
            && desired == self.last_desired_presence
            && self.last_request_failed
            && self.last_request_at.is_some_and(|at| at.elapsed() < CONNECT_RETRY_INTERVAL)
        {
            return;
        }

        match desired {
            Some(next) => {
                self.last_desired_presence = Some(next.clone());
                self.send(Command::Set(next));
            }
            None => {
                if self.active_presence.is_some() {
                    tracing::info!(
                        target: "vertexlauncher/discord_presence",
                        "Discord Rich Presence stopped."
                    );
                    self.send(Command::Clear);
                }
                self.active_presence = None;
                self.last_desired_presence = None;
                self.last_presence_sync_at = None;
            }
        }
    }

    fn send(&mut self, command: Command) {
        let worker = self.worker.get_or_insert_with(PresenceWorker::start);
        if worker.commands.send(command).is_ok() {
            self.in_flight = true;
            self.last_request_at = Some(Instant::now());
        }
    }

    /// Takes in what the task has finished since the last frame.
    fn take_events(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        let events: Vec<Event> = worker.events.try_iter().collect();
        for event in events {
            self.in_flight = false;
            match event {
                Event::Set { desired, result: Ok(()) } => {
                    if self.active_presence.is_none() {
                        tracing::info!(
                            target: "vertexlauncher/discord_presence",
                            "Discord Rich Presence started."
                        );
                    }
                    self.active_presence = Some(desired);
                    self.last_presence_sync_at = Some(Instant::now());
                    self.last_request_failed = false;
                }
                Event::Set { result: Err(error), .. } => {
                    tracing::debug!(
                        target: "vertexlauncher/discord_presence",
                        "Could not update Discord Rich Presence: {error}"
                    );
                    self.last_request_failed = true;
                }
                Event::Cleared => self.last_request_failed = false,
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
            // Nothing is running, so nothing can match: skip the per-instance canonicalize
            // (a filesystem call) that this per-frame path would otherwise make for every instance.
            if running_set.is_empty() {
                break;
            }
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
            return None;
        }

        if let Some(presence) = first_eligible {
            return Some(presence);
        }

        if launcher_presence_blocked_by_mod {
            return None;
        }

        Some(self.menu_presence(menu_context, selected_instance_id, instances))
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

}

/// The Discord connection, used only by the background task (every call here can block on the socket).
#[derive(Default)]
struct PresenceClient {
    client: Option<DiscordIpcClient>,
    connected: bool,
    last_connect_attempt_at: Option<Instant>,
    last_connect_error: Option<String>,
}

impl PresenceClient {
    fn set_presence(&mut self, desired: &DesiredPresence) -> Result<(), String> {
        let activity = build_activity(desired);

        let initial_attempt = self
            .ensure_connected()?
            .set_activity(activity.clone())
            .map_err(|err| format!("failed to set Discord activity: {err}"));

        match initial_attempt {
            Ok(()) => Ok(()),
            Err(initial_err) => {
                self.reset();
                self.ensure_connected()?
                    .set_activity(activity)
                    .map_err(|err| {
                        format!(
                            "{initial_err}; retry after reconnect also failed to set Discord activity: {err}"
                        )
                    })?;
                Ok(())
            }
        }
    }

    fn clear_presence(&mut self) {
        let clear_failed = self
            .client
            .as_mut()
            .is_some_and(|client| client.clear_activity().is_err());
        if clear_failed {
            self.reset();
        }
    }

    fn ensure_connected(&mut self) -> Result<&mut DiscordIpcClient, String> {
        if self.client.is_none() {
            prepare_discord_ipc_environment();
            self.client = Some(DiscordIpcClient::new(DISCORD_APPLICATION_ID));
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
                message
            })?;
            self.connected = true;
            self.last_connect_error = None;
        }

        if !self.connected {
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

    fn reset(&mut self) {
        if let Some(client) = self.client.as_mut() {
            let _ = client.close();
        }
        self.client = None;
        self.connected = false;
        self.last_connect_attempt_at = None;
    }

    fn can_attempt_connect(&self) -> bool {
        self.last_connect_attempt_at
            .is_none_or(|last| last.elapsed() >= CONNECT_RETRY_INTERVAL)
    }
}

#[cfg(target_os = "linux")]
fn prepare_discord_ipc_environment() {
    let candidate_dirs = discord_ipc_candidate_dirs();

    let Some(socket_dir) = find_discord_ipc_socket_dir(&candidate_dirs) else {
        return;
    };

    // SAFETY: This is called by the presence task before each Discord IPC client creation. We only update the process
    // env var the `discord-rich-presence` crate's socket lookup reads, and nothing else in the launcher writes it.
    unsafe {
        env::set_var("TMPDIR", &socket_dir);
    }
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
            .details(tr!("Playing {instance_name}", instance_name = instance_name).into_owned())
            .state(tr!("Launched via Vertex").into_owned())
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
) -> (String, String) {
    match context {
        MenuPresenceContext::Home(HomePresenceSection::Activity) => {
            (tr!("Browsing featured packs").into_owned(), tr!("On the home screen").into_owned())
        }
        MenuPresenceContext::Home(HomePresenceSection::Screenshots) => (
            tr!("Reviewing screenshots").into_owned(),
            tr!("In the screenshot manager").into_owned(),
        ),
        MenuPresenceContext::Instance(InstancePresenceSection::Content)
        | MenuPresenceContext::Screen(AppScreen::Instance) => (
            selected_instance_name
                .map(|name| tr!("Managing {name}", name = name).into_owned())
                .unwrap_or_else(|| tr!("Managing an instance").into_owned()),
            tr!("In instance settings").into_owned(),
        ),
        MenuPresenceContext::Instance(InstancePresenceSection::Screenshots) => (
            selected_instance_name
                .map(|name| tr!("Reviewing {name} screenshots", name = name).into_owned())
                .unwrap_or_else(|| tr!("Reviewing instance screenshots").into_owned()),
            tr!("In the screenshot gallery").into_owned(),
        ),
        MenuPresenceContext::Instance(InstancePresenceSection::Logs) => (
            selected_instance_name
                .map(|name| tr!("Reading {name} logs", name = name).into_owned())
                .unwrap_or_else(|| tr!("Reading instance logs").into_owned()),
            tr!("In the logs viewer").into_owned(),
        ),
        MenuPresenceContext::Screen(AppScreen::Home) => {
            (tr!("Browsing featured packs").into_owned(), tr!("On the home screen").into_owned())
        }
        MenuPresenceContext::Screen(AppScreen::Library) => {
            (tr!("Managing instances").into_owned(), tr!("In the library").into_owned())
        }
        MenuPresenceContext::Screen(AppScreen::Discover) => {
            (tr!("Browsing modpacks").into_owned(), tr!("In Discover").into_owned())
        }
        MenuPresenceContext::Screen(AppScreen::DiscoverDetail) => {
            (tr!("Reviewing a modpack").into_owned(), tr!("In Discover").into_owned())
        }
        MenuPresenceContext::Screen(AppScreen::ContentBrowser) => {
            (tr!("Adding content").into_owned(), tr!("Managing mods and resources").into_owned())
        }
        MenuPresenceContext::Screen(AppScreen::Skins) => {
            (tr!("Customizing a skin").into_owned(), tr!("In Skin Manager").into_owned())
        }
        MenuPresenceContext::Screen(AppScreen::Settings) => {
            (tr!("Adjusting settings").into_owned(), tr!("Configuring Vertex").into_owned())
        }
        MenuPresenceContext::Screen(AppScreen::Legal) => {
            (tr!("Reviewing licenses").into_owned(), tr!("In legal information").into_owned())
        }
        MenuPresenceContext::Screen(AppScreen::Console) => {
            (tr!("Checking logs").into_owned(), tr!("In the console").into_owned())
        }
    }
}

fn current_unix_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// A Discord that accepts the connection and then never answers the handshake, as one that is starting up or stuck
    /// does. Connecting to it blocks for as long as it stays that way, so the UI thread must never be the one doing it.
    #[test]
    fn a_discord_that_never_answers_cannot_freeze_the_ui_thread() {
        let dir = std::env::temp_dir().join(format!("vertex-discord-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let socket = dir.join("discord-ipc-0");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("fake Discord socket");
        // Hold every connection open without ever replying.
        let held = std::thread::spawn(move || {
            let mut connections = Vec::new();
            while let Ok((stream, _)) = listener.accept() {
                connections.push(stream);
            }
        });
        // SAFETY: nothing else in the tests reads or writes this variable.
        unsafe { env::set_var("XDG_RUNTIME_DIR", &dir) };

        // Rich Presence is on by default.
        let config = Config::default();
        assert!(config.discord_rich_presence_enabled());
        let instances = InstanceStore::default();
        let mut manager = DiscordPresenceManager::default();
        let started = Instant::now();
        for _ in 0..50 {
            manager.update(
                &config,
                &instances,
                Path::new("/nonexistent"),
                MenuPresenceContext::Screen(AppScreen::Home),
                None,
            );
        }
        let spent = started.elapsed();
        assert!(spent < Duration::from_millis(500), "update blocked the calling thread for {spent:?}");
        assert!(manager.in_flight, "the request should be with the background task, still unanswered");
        drop(manager);
        let _ = std::fs::remove_file(&socket);
        drop(held);
    }
}
