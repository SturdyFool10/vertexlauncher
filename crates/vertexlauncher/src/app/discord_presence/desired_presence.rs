use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DesiredPresence {
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
