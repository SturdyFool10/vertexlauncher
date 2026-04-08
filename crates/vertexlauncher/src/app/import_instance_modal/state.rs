use super::*;

#[derive(Default)]
pub struct ImportInstanceState {
    pub source_mode_index: usize,
    pub package_path: PathBuf,
    pub launcher_path: PathBuf,
    pub launcher_kind_index: usize,
    pub instance_name: String,
    pub error: Option<String>,
    pub(super) preview_in_flight: bool,
    pub(super) preview_request_serial: u64,
    pub(super) preview_results_tx: Option<mpsc::Sender<(u64, Result<ImportPreview, String>)>>,
    pub(super) preview_results_rx: Option<Arc<Mutex<mpsc::Receiver<(u64, Result<ImportPreview, String>)>>>>,
    pub import_in_flight: bool,
    pub import_latest_progress: Option<ImportProgress>,
    pub import_progress_tx: Option<mpsc::Sender<ImportProgress>>,
    pub import_progress_rx: Option<Arc<Mutex<mpsc::Receiver<ImportProgress>>>>,
    pub import_results_tx: Option<mpsc::Sender<ImportTaskResult>>,
    pub import_results_rx: Option<Arc<Mutex<mpsc::Receiver<ImportTaskResult>>>>,
    pub(super) preview: Option<ImportPreview>,
}

impl ImportInstanceState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Debug)]
pub struct ImportRequest {
    pub source: ImportSource,
    pub instance_name: String,
    pub manual_curseforge_files: HashMap<u64, PathBuf>,
    pub manual_curseforge_staging_dir: Option<PathBuf>,
    pub max_concurrent_downloads: u32,
}

#[derive(Clone, Debug)]
pub enum ImportSource {
    ManifestFile(PathBuf),
    LauncherDirectory {
        path: PathBuf,
        launcher: Option<LauncherKind>,
    },
}

#[derive(Clone, Debug)]
pub enum ModalAction {
    None,
    Cancel,
    Import(ImportRequest),
}

pub type ImportTaskResult = Result<(InstanceStore, InstanceRecord), ImportPackageError>;

#[derive(Clone, Debug)]
pub enum ImportPackageError {
    Message(String),
    ManualCurseForgeDownloads {
        requirements: Vec<CurseForgeManualDownloadRequirement>,
        staged_files: HashMap<u64, PathBuf>,
    },
}

impl ImportPackageError {
    pub(super) fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl std::fmt::Display for ImportPackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
            Self::ManualCurseForgeDownloads { requirements, .. } => write!(
                f,
                "{} CurseForge files require manual download",
                requirements.len()
            ),
        }
    }
}

impl std::error::Error for ImportPackageError {}

impl From<String> for ImportPackageError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for ImportPackageError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_owned())
    }
}

#[derive(Clone, Debug)]
pub struct ImportProgress {
    pub message: String,
    pub completed_steps: usize,
    pub total_steps: usize,
}

pub(super) fn ensure_preview_channel(state: &mut ImportInstanceState) {
    if state.preview_results_tx.is_some() && state.preview_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(u64, Result<ImportPreview, String>)>();
    state.preview_results_tx = Some(tx);
    state.preview_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn poll_preview_results(state: &mut ImportInstanceState) {
    let Some(rx) = state.preview_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/import_instance",
            request_serial = state.preview_request_serial,
            "Import preview receiver mutex was poisoned."
        );
        return;
    };
    loop {
        match receiver.try_recv() {
            Ok((request_serial, result)) => {
                if request_serial != state.preview_request_serial {
                    tracing::debug!(
                        target: "vertexlauncher/import_instance",
                        request_serial,
                        active_request_serial = state.preview_request_serial,
                        "Ignoring stale import preview result."
                    );
                    continue;
                }
                state.preview_in_flight = false;
                match result {
                    Ok(preview) => {
                        tracing::info!(
                            target: "vertexlauncher/import_instance",
                            request_serial,
                            preview_kind = %preview.kind.label(),
                            detected_name = %preview.detected_name,
                            game_version = %preview.game_version,
                            modloader = %preview.modloader,
                            modloader_version = %preview.modloader_version,
                            "Import preview completed."
                        );
                        if state.instance_name.trim().is_empty() {
                            state.instance_name = preview.detected_name.clone();
                        }
                        state.preview = Some(preview);
                        state.error = None;
                    }
                    Err(err) => {
                        tracing::warn!(
                            target: "vertexlauncher/import_instance",
                            request_serial,
                            error = %err,
                            "Import preview failed."
                        );
                        state.preview = None;
                        state.error = Some(err);
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/import_instance",
                    request_serial = state.preview_request_serial,
                    "Import preview worker channel disconnected unexpectedly."
                );
                state.preview_in_flight = false;
                state.error = Some("Import preview worker stopped unexpectedly.".to_owned());
                state.preview_results_tx = None;
                state.preview_results_rx = None;
                break;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ImportPreview {
    pub(super) kind: ImportPreviewKind,
    pub(super) detected_name: String,
    pub(super) game_version: String,
    pub(super) modloader: String,
    pub(super) modloader_version: String,
    pub(super) summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ImportPreviewKind {
    Manifest(ImportPackageKind),
    Launcher(LauncherKind),
}

impl ImportPreviewKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            ImportPreviewKind::Manifest(kind) => kind.label(),
            ImportPreviewKind::Launcher(kind) => kind.label(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ImportPackageKind {
    VertexPack,
    ModrinthPack,
    CurseForgePack,
}

impl ImportPackageKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            ImportPackageKind::VertexPack => "Vertex .vtmpack",
            ImportPackageKind::ModrinthPack => "Modrinth .mrpack",
            ImportPackageKind::CurseForgePack => "CurseForge modpack zip",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ImportMode {
    ManifestFile,
    LauncherDirectory,
}

impl ImportMode {
    pub(super) fn from_index(index: usize) -> Self {
        match index {
            1 => Self::LauncherDirectory,
            _ => Self::ManifestFile,
        }
    }

    pub(super) fn options() -> [&'static str; 2] {
        ["From manifest file", "Import from another launcher"]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LauncherKind {
    Modrinth,
    CurseForge,
    Prism,
    ATLauncher,
    Unknown,
}

impl LauncherKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Modrinth => "Modrinth launcher instance",
            Self::CurseForge => "CurseForge instance",
            Self::Prism => "Prism / MultiMC / PolyMC instance",
            Self::ATLauncher => "ATLauncher instance",
            Self::Unknown => "Generic launcher instance",
        }
    }
}

pub(super) const LAUNCHER_KIND_OPTIONS: [&str; 5] = [
    "Auto-detect",
    "Modrinth",
    "CurseForge",
    "Prism / MultiMC",
    "ATLauncher",
];

pub(super) fn selected_import_mode(state: &ImportInstanceState) -> ImportMode {
    ImportMode::from_index(state.source_mode_index)
}

pub(super) fn selected_launcher_hint(state: &ImportInstanceState) -> Option<LauncherKind> {
    match state.launcher_kind_index {
        1 => Some(LauncherKind::Modrinth),
        2 => Some(LauncherKind::CurseForge),
        3 => Some(LauncherKind::Prism),
        4 => Some(LauncherKind::ATLauncher),
        _ => None,
    }
}

pub(super) fn path_input_string(path: &Path) -> String {
    path.as_os_str().to_string_lossy().into_owned()
}

pub(super) fn update_path_from_input(path: &mut PathBuf, input: &str) -> bool {
    let trimmed = input.trim();
    let next = if trimmed.is_empty() {
        PathBuf::new()
    } else {
        PathBuf::from(trimmed)
    };
    if *path == next {
        return false;
    }
    *path = next;
    true
}

