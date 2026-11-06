use sturdy::diagnostics::{self, LogLevel};
use tracing::level_filters::LevelFilter;

/// Sends engine log lines into `tracing` at the severity the engine gave them, and silences the engine's own console output to
/// avoid printing every line twice.
///
/// The engine files everything that is not a problem (progress, device selection, timings) at its lowest severity, trace, and
/// reserves warn and error for problems, so those arrive here as `trace!`, `warn!` and `error!` and the launcher's log filter
/// decides what shows: by default only the problems, and `RUST_LOG=sturdy=trace` brings the rest back. The engine's own level is set
/// to whatever the log filter lets through, so what the filter would drop is never formatted and sent in the first place.
pub(crate) fn forward_engine_logs() {
    diagnostics::set_console_logging(false);
    diagnostics::set_log_level(match LevelFilter::current() {
        LevelFilter::TRACE => LogLevel::Trace,
        LevelFilter::DEBUG => LogLevel::Debug,
        LevelFilter::INFO => LogLevel::Info,
        LevelFilter::WARN => LogLevel::Warn,
        _ => LogLevel::Error,
    });
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
