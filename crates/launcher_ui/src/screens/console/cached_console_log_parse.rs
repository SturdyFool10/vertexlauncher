use super::*;

#[derive(Clone)]
pub(super) struct CachedConsoleLogParse {
    pub(super) fingerprint: u64,
    pub(super) level: Option<LogLevel>,
    pub(super) in_error_trace_after: bool,
    pub(super) last_used_frame: u64,
}
