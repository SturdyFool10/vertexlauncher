//! The "Download OpenJDK" and "Test Java" buttons of the Java settings: background actions that report a status
//! message and, for a download, the path to select. Toolkit-free.

use vertex_i18n::tr;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use config::JavaRuntimeVersion;
use installation::ensure_openjdk_runtime_async;
use launcher_runtime as tokio_runtime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavaActionKind {
    Download,
    Validate,
}

enum Outcome {
    Downloaded(PathBuf),
    Validated(String),
}

struct ActionResult {
    kind: JavaActionKind,
    outcome: Result<Outcome, String>,
}

#[derive(Default)]
struct ActionState {
    in_flight: Option<JavaActionKind>,
    rx: Option<mpsc::Receiver<ActionResult>>,
    message: Option<String>,
}

fn states() -> &'static Mutex<HashMap<u8, ActionState>> {
    static STATES: OnceLock<Mutex<HashMap<u8, ActionState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// What the buttons of one runtime row need: what is running, and the last status line.
#[derive(Clone, Debug, Default)]
pub struct JavaActionView {
    pub in_flight: Option<JavaActionKind>,
    pub message: Option<String>,
}

/// Collects a finished action for `runtime`. A completed download is returned so the caller can select its path.
pub fn poll(runtime: JavaRuntimeVersion) -> (JavaActionView, Option<PathBuf>) {
    let mut downloaded = None;
    let Ok(mut states) = states().lock() else { return (JavaActionView::default(), None) };
    let state = states.entry(runtime.major()).or_default();
    if let Some(rx) = state.rx.as_ref() {
        match rx.try_recv() {
            Ok(result) => {
                state.in_flight = None;
                state.rx = None;
                state.message = Some(match result.outcome {
                    Ok(Outcome::Downloaded(path)) => {
                        let message = tr!("Downloaded {runtime} and selected {path}.", runtime = runtime.label(), path = path.display()).into_owned();
                        downloaded = Some(path);
                        message
                    }
                    Ok(Outcome::Validated(version)) => tr!("Java validation passed: {version}", version = version).into_owned(),
                    Err(err) => match result.kind {
                        JavaActionKind::Download => tr!("Failed to download {runtime}: {err}", runtime = runtime.label(), err = err).into_owned(),
                        JavaActionKind::Validate => tr!("Java validation failed: {err}", err = err).into_owned(),
                    },
                });
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                state.in_flight = None;
                state.rx = None;
                state.message = Some(tr!("{runtime} action worker disconnected.", runtime = runtime.label()).into_owned());
            }
        }
    }
    (JavaActionView { in_flight: state.in_flight, message: state.message.clone() }, downloaded)
}

pub fn request_download(runtime: JavaRuntimeVersion) {
    let (tx, rx) = mpsc::channel();
    if let Ok(mut states) = states().lock() {
        let state = states.entry(runtime.major()).or_default();
        state.in_flight = Some(JavaActionKind::Download);
        state.rx = Some(rx);
        state.message = Some(tr!("Downloading {runtime}...", runtime = runtime.label()).into_owned());
    }
    let major = runtime.major();
    tokio_runtime::spawn_detached(async move {
        let outcome = ensure_openjdk_runtime_async(major).await.map(Outcome::Downloaded).map_err(|err| err.to_string());
        let _ = tx.send(ActionResult { kind: JavaActionKind::Download, outcome });
    });
}

pub fn request_validation(runtime: JavaRuntimeVersion, path: String) {
    let (tx, rx) = mpsc::channel();
    if let Ok(mut states) = states().lock() {
        let state = states.entry(runtime.major()).or_default();
        state.in_flight = Some(JavaActionKind::Validate);
        state.rx = Some(rx);
        state.message = Some(tr!("Testing {path}...", path = path).into_owned());
    }
    tokio_runtime::spawn_detached(async move {
        let outcome = validate_java_executable(path.as_str()).await.map(Outcome::Validated);
        let _ = tx.send(ActionResult { kind: JavaActionKind::Validate, outcome });
    });
}

async fn validate_java_executable(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(tr!("No Java executable path is configured.").into_owned());
    }
    let path = Path::new(trimmed);
    launcher_runtime::fs::metadata(path).await.map_err(|err| tr!("Path is not readable: {trimmed}: {err}", trimmed = trimmed, err = err).into_owned())?;
    let mut command = std::process::Command::new(path);
    command.arg("-version");
    let output = launcher_runtime::process::output(command)
        .await
        .map_err(|err| tr!("Could not run {trimmed}: {err}", trimmed = trimmed, err = err).into_owned())?;
    let stdout = String::from_utf8_lossy(output.stdout.as_slice());
    let stderr = String::from_utf8_lossy(output.stderr.as_slice());
    let combined = format!("{stdout}\n{stderr}");
    let first_line = combined.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("<no version output>");
    let lower = combined.to_ascii_lowercase();
    if !output.status.success() {
        return Err(tr!("`{trimmed} -version` exited with {status}; output: {first_line}", trimmed = trimmed, status = output.status.code().map(|code| code.to_string()).unwrap_or_else(|| "signal".to_owned()), first_line = first_line).into_owned());
    }
    let looks_like_java = lower.contains("openjdk") || lower.contains("java version") || lower.contains("runtime environment") || lower.contains("java(tm)");
    if !looks_like_java {
        return Err(tr!("`{trimmed} -version` ran, but output did not look like Java: {first_line}", trimmed = trimmed, first_line = first_line).into_owned());
    }
    Ok(first_line.to_owned())
}
