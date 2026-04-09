use super::*;

pub struct DiscordPresenceManager {
    pub(super) client: Option<DiscordIpcClient>,
    pub(super) session_start_by_instance_id: HashMap<String, i64>,
    pub(super) active_presence: Option<DesiredPresence>,
    pub(super) last_desired_presence: Option<DesiredPresence>,
    pub(super) connected: bool,
    pub(super) last_connect_attempt_at: Option<Instant>,
    pub(super) last_connect_error: Option<String>,
    pub(super) last_presence_sync_at: Option<Instant>,
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
