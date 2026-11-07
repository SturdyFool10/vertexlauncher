mod app_log_layer;
mod message_visitor;

use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::Once,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use tracing_subscriber::prelude::*;

use crate::app::tracing_setup::app_log_layer::{AppLogLayer, ConsoleLayer, console_captures};

pub(super) type SharedLogWriter = Arc<Mutex<Box<dyn io::Write + Send>>>;

static PANIC_LOG_WRITER: std::sync::OnceLock<SharedLogWriter> = std::sync::OnceLock::new();
static PANIC_HOOK_INSTALLED: Once = Once::new();
static ACTIVE_LOG_PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

/// Makes the console remember what `launcher` shows for Vertex's own log, and a little more: lines down to debug are kept
/// even when they are hidden, so choosing a more verbose level afterwards brings back what was logged before. The engine
/// is asked for the same, so what nothing wants is never produced. `game` only decides which kept trace lines to throw away:
/// a game's output is stored whole, and its level is applied when the console is drawn.
pub(super) fn apply_console_log_levels(launcher: config::ConsoleLogLevel, game: config::ConsoleLogLevel) {
    let captured = match launcher {
        config::ConsoleLogLevel::Trace => tracing::Level::TRACE,
        _ => tracing::Level::DEBUG,
    };
    app_log_layer::set_console_capture_level(captured);
    vertex_shell::set_engine_log_filter(captured);
    launcher_ui::console::discard_trace_lines(
        launcher != config::ConsoleLogLevel::Trace,
        game != config::ConsoleLogLevel::Trace,
    );
}

pub(super) fn init_tracing() -> Option<PathBuf> {
    ACTIVE_LOG_PATH.get_or_init(initialize_tracing).clone()
}

fn initialize_tracing() -> Option<PathBuf> {
    let started_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let log_dir = app_paths::logs_dir();
    tracing::debug!(
        target: "vertexlauncher/io",
        op = "create_dir_all",
        path = %log_dir.display(),
        context = "initialize logging directory"
    );
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join(format!("vertex_{started_epoch}.log"));
    let (writer, active_log_path) = open_log_writer(log_path, &log_dir);
    install_panic_hook(Arc::clone(&writer));

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let app_layer = AppLogLayer::new(writer);

    // Each layer has its own filter: the file and terminal follow `RUST_LOG`, the console follows its own setting.
    let _ = tracing_subscriber::registry()
        .with(app_layer.with_filter(env_filter))
        .with(ConsoleLayer.with_filter(tracing_subscriber::filter::filter_fn(console_captures)))
        .try_init();

    active_log_path
}

fn open_log_writer(
    primary_log_path: PathBuf,
    log_dir: &Path,
) -> (SharedLogWriter, Option<PathBuf>) {
    if let Some(writer) = try_open_log_file(&primary_log_path, "initialize main log file") {
        return (
            Arc::new(Mutex::new(Box::new(writer))),
            Some(primary_log_path),
        );
    }

    let fallback_log_path = log_dir.join("vertex_fallback.log");
    if let Some(writer) = try_open_log_file(&fallback_log_path, "initialize fallback log file") {
        return (
            Arc::new(Mutex::new(Box::new(writer))),
            Some(fallback_log_path),
        );
    }

    let temp_log_path = std::env::temp_dir().join("vertex_fallback.log");
    if let Some(writer) = try_open_log_file(&temp_log_path, "initialize temp fallback log file") {
        return (Arc::new(Mutex::new(Box::new(writer))), Some(temp_log_path));
    }

    tracing::warn!(
        target: "vertexlauncher/io",
        "Unable to open any log file; falling back to stderr/console logging only."
    );
    (Arc::new(Mutex::new(Box::new(io::sink()))), None)
}

pub(super) fn write_log_line(writer: &SharedLogWriter, line: &str) {
    let mut guard = match writer.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let _ = writeln!(guard, "{line}");
    let _ = guard.flush();
}

fn try_open_log_file(path: &Path, context: &str) -> Option<File> {
    tracing::debug!(
        target: "vertexlauncher/io",
        op = "file_create",
        path = %path.display(),
        context
    );
    match File::create(path) {
        Ok(file) => Some(file),
        Err(error) => {
            tracing::warn!(
                target: "vertexlauncher/io",
                path = %path.display(),
                error = %error,
                "{context}"
            );
            None
        }
    }
}

pub(super) fn current_date_time_parts() -> (String, String) {
    let ts = humantime::format_rfc3339_seconds(SystemTime::now()).to_string();
    if let Some((date, time)) = ts.split_once('T') {
        return (date.to_owned(), time.trim_end_matches('Z').to_owned());
    }
    ("unknown-date".to_owned(), "unknown-time".to_owned())
}

pub(super) fn format_module_path(target: &str, file: Option<&str>) -> String {
    if let Some(file) = file
        && let Some((crate_name, rest)) = file.split_once("/src/")
    {
        let crate_name = crate_name.rsplit('/').next().unwrap_or(crate_name);
        return format!("{crate_name}/{}", rest.replace('\\', "/"));
    }
    target.replace("::", "/")
}

pub(super) fn should_omit_module_path(target: &str, module_path: &str) -> bool {
    target == "log" || module_path == "log"
}

fn install_panic_hook(writer: SharedLogWriter) {
    let _ = PANIC_LOG_WRITER.set(writer);
    PANIC_HOOK_INSTALLED.call_once(|| {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            write_panic_record(panic_info);
            previous_hook(panic_info);
        }));
    });
}

fn write_panic_record(panic_info: &std::panic::PanicHookInfo<'_>) {
    let Some(writer) = PANIC_LOG_WRITER.get() else {
        return;
    };

    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    let location = panic_info
        .location()
        .map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let payload = panic_payload_text(panic_info);
    let backtrace = std::backtrace::Backtrace::force_capture();
    // The engine's side of the stack: its announced call site and scopes, which a Rust backtrace cannot show.
    let engine_trace = vertex_shell::sturdy::trace::capture();
    let (date, time) = current_date_time_parts();

    let summary = format!(
        "[{date}][{time}][ERROR][panic]: thread={thread_name} location={location} message={payload}"
    );
    let backtrace = if engine_trace.is_empty() {
        format!("{backtrace}")
    } else {
        format!("{backtrace}\nengine stack:\n{engine_trace}")
    };

    match writer.try_lock() {
        Ok(mut guard) => {
            let _ = writeln!(guard, "{summary}");
            let _ = writeln!(guard, "{backtrace}");
            let _ = guard.flush();
        }
        Err(std::sync::TryLockError::Poisoned(poisoned)) => {
            let mut guard = poisoned.into_inner();
            let _ = writeln!(guard, "{summary}");
            let _ = writeln!(guard, "{backtrace}");
            let _ = guard.flush();
        }
        Err(std::sync::TryLockError::WouldBlock) => {}
    }
}

fn panic_payload_text(panic_info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(payload) = panic_info.payload().downcast_ref::<&'static str>() {
        return (*payload).to_owned();
    }
    if let Some(payload) = panic_info.payload().downcast_ref::<String>() {
        return payload.clone();
    }
    "non-string panic payload".to_owned()
}
