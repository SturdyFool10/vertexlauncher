#[derive(Debug, Clone, Copy)]
pub(crate) enum ServerPingStatus {
    Unknown,
    Offline,
    Online { latency_ms: u64 },
}
