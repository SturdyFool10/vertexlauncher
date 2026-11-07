use sturdy::diagnostics::{self, LogLevel};
use std::sync::atomic::{AtomicU8, Ordering};

static ENGINE_LEVEL: AtomicU8 = AtomicU8::new(2);

fn set_engine_log_level(rank: u8) {
    diagnostics::set_log_level(match rank {
        4 => LogLevel::Trace,
        3 => LogLevel::Debug,
        2 => LogLevel::Info,
        1 => LogLevel::Warn,
        _ => LogLevel::Error,
    });
}

/// The most verbose level any of the launcher's log filters wants from the engine. Takes effect at once, also before the
/// engine is running.
pub fn set_engine_log_filter(level: tracing::Level) {
    let rank = match level {
        tracing::Level::ERROR => 0,
        tracing::Level::WARN => 1,
        tracing::Level::INFO => 2,
        tracing::Level::DEBUG => 3,
        tracing::Level::TRACE => 4,
    };
    ENGINE_LEVEL.store(rank, Ordering::Relaxed);
    set_engine_log_level(rank);
}

/// Sends engine log lines into `tracing` at the severity the engine gave them, and silences the engine's own console output to
/// avoid printing every line twice.
///
/// The engine files everything that is not a problem (progress, device selection, timings) at its lowest severity, trace, and
/// reserves warn and error for problems, so those arrive here as `trace!`, `warn!` and `error!` and the launcher's log filters
/// decide what shows: by default only the problems, and the console's log level (or `RUST_LOG=sturdy=trace` for the log file)
/// brings the rest back. The engine's own level is set by [`set_engine_log_level`] to the most verbose any filter wants, so what
/// every filter would drop is never formatted and sent in the first place.
pub(crate) fn forward_engine_logs() {
    diagnostics::set_console_logging(false);
    set_engine_log_level(ENGINE_LEVEL.load(Ordering::Relaxed));
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
