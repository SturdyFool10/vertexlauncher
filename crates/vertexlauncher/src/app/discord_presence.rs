use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use config::Config;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use installation::{display_user_path, running_instance_roots};
use instances::{InstanceStore, instance_root_path};

const DISCORD_APPLICATION_ID: &str = "1486469547073601627";
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
struct DesiredPresence {
    instance_id: String,
    instance_name: String,
    started_at_unix_secs: i64,
}

pub struct DiscordPresenceManager {
    client: Option<DiscordIpcClient>,
    session_start_by_instance_id: HashMap<String, i64>,
    active_presence: Option<DesiredPresence>,
    last_connect_attempt_at: Option<Instant>,
}

impl Default for DiscordPresenceManager {
    fn default() -> Self {
        Self {
            client: None,
            session_start_by_instance_id: HashMap::new(),
            active_presence: None,
            last_connect_attempt_at: None,
        }
    }
}

impl DiscordPresenceManager {
    pub fn update(
        &mut self,
        config: &Config,
        instances: &InstanceStore,
        installations_root: &Path,
    ) {
        let running_roots = running_instance_roots();
        let desired = self.desired_presence(config, instances, installations_root, &running_roots);

        if desired == self.active_presence {
            return;
        }

        match desired.clone() {
            Some(next) => {
                if self.set_presence(&next).is_ok() {
                    self.active_presence = Some(next);
                }
            }
            None => {
                self.clear_presence();
                self.active_presence = None;
            }
        }
    }

    fn desired_presence(
        &mut self,
        config: &Config,
        instances: &InstanceStore,
        installations_root: &Path,
        running_roots: &[String],
    ) -> Option<DesiredPresence> {
        let running_set: HashSet<&str> = running_roots.iter().map(String::as_str).collect();
        let mut active_instance_ids = HashSet::new();

        let mut first_eligible = None;
        for instance in &instances.instances {
            let instance_root = instance_root_path(installations_root, instance);
            let instance_key = fs::canonicalize(instance_root.as_path())
                .map(|path| display_user_path(path.as_path()))
                .unwrap_or_else(|_| display_user_path(instance_root.as_path()));
            if !running_set.contains(instance_key.as_str()) {
                continue;
            }
            active_instance_ids.insert(instance.id.clone());
            if first_eligible.is_none() && !instance.discord_rich_presence_mod_installed {
                let started_at_unix_secs = *self
                    .session_start_by_instance_id
                    .entry(instance.id.clone())
                    .or_insert_with(current_unix_timestamp_secs);
                first_eligible = Some(DesiredPresence {
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

        first_eligible
    }

    fn set_presence(&mut self, desired: &DesiredPresence) -> Result<(), String> {
        let client = self.ensure_client_connected()?;
        let activity = activity::Activity::new()
            .state(format!("Playing {}", desired.instance_name))
            .details("Launched through Vertex")
            .timestamps(activity::Timestamps::new().start(desired.started_at_unix_secs));
        client
            .set_activity(activity)
            .map_err(|err| format!("failed to set Discord activity: {err}"))
    }

    fn clear_presence(&mut self) {
        if let Some(client) = self.client.as_mut() {
            let _ = client.clear_activity();
        }
    }

    fn ensure_client_connected(&mut self) -> Result<&mut DiscordIpcClient, String> {
        if self.client.is_none() {
            self.client = Some(DiscordIpcClient::new(DISCORD_APPLICATION_ID));
        }

        let should_retry = self
            .last_connect_attempt_at
            .is_none_or(|last| last.elapsed() >= CONNECT_RETRY_INTERVAL);

        if should_retry {
            self.last_connect_attempt_at = Some(Instant::now());

            let connect_result = {
                let client = self
                    .client
                    .as_mut()
                    .ok_or_else(|| "Discord client missing".to_owned())?;
                client.connect()
            };

            connect_result.map_err(|err| format!("failed to connect to Discord IPC: {err}"))?;
        }

        self.client
            .as_mut()
            .ok_or_else(|| "Discord client missing".to_owned())
    }
}

fn current_unix_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
