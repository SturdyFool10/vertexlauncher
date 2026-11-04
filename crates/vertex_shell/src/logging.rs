use sturdy::diagnostics::{self, LogLevel};

/// Sends engine log lines into `tracing` so the launcher has one log, and silences the engine's own console output to
/// avoid printing every line twice. Every line is passed on as it comes; the launcher's console tab is what collapses
/// repeats.
pub(crate) fn forward_engine_logs() {
    diagnostics::set_console_logging(false);
    diagnostics::add_log_sink(|level, message| match level {
        LogLevel::Trace => tracing::trace!(target: "sturdy", "{message}"),
        LogLevel::Debug => tracing::debug!(target: "sturdy", "{message}"),
        LogLevel::Info => tracing::info!(target: "sturdy", "{message}"),
        LogLevel::Warn => tracing::warn!(target: "sturdy", "{message}"),
        _ => {
            crate::watchdog::engine_error(message);
            tracing::error!(target: "sturdy", "{message}")
        }
    });
}
