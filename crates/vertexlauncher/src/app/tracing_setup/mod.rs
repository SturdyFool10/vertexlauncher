mod app_log_layer;
mod message_visitor;

use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use tracing_subscriber::prelude::*;

use crate::app::tracing_setup::app_log_layer::AppLogLayer;

pub(super) fn init_tracing() -> Option<PathBuf> {
    let started_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let log_dir = PathBuf::from("logs");
    tracing::debug!(
        target: "vertexlauncher/io",
        op = "create_dir_all",
        path = %log_dir.display(),
        context = "initialize logging directory"
    );
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join(format!("vertex_{started_epoch}.log"));
    let (writer, active_log_path) = open_log_writer(log_path, &log_dir);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_level(false)
        .with_writer(std::io::stderr);
    let app_layer = AppLogLayer { writer };

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .with(app_layer)
        .try_init();

    active_log_path
}

fn open_log_writer(
    primary_log_path: PathBuf,
    log_dir: &Path,
) -> (Arc<Mutex<Box<dyn io::Write + Send>>>, Option<PathBuf>) {
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
