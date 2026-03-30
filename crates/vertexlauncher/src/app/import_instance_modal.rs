use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use curseforge::Client as CurseForgeClient;
use eframe::egui;
use instances::{
    InstanceRecord, InstanceStore, NewInstanceSpec, create_instance, delete_instance,
    instance_root_path,
};
use launcher_runtime as tokio_runtime;
use launcher_ui::{
    ui::style,
    ui::{components::settings_widgets, modal},
};
use managed_content::{
    CONTENT_MANIFEST_FILE_NAME, ContentInstallManifest, InstalledContentProject,
    ManagedContentSource, ModpackInstallState, load_content_manifest, load_modpack_install_state,
    remove_modpack_install_state, save_content_manifest, save_modpack_install_state,
};
use modrinth::Client as ModrinthClient;
use serde::Deserialize;
use serde_json::Value;
use textui::{ButtonOptions, LabelOptions, TextUi};
use vtmpack::{VtmpackDownloadableEntry, VtmpackManifest, read_vtmpack_manifest};

const MODAL_GAP_SM: f32 = 6.0;
const MODAL_GAP_MD: f32 = 8.0;
const MODAL_GAP_LG: f32 = 10.0;
const ACTION_BUTTON_MAX_WIDTH: f32 = 260.0;
const MODRINTH_DOWNLOAD_MIN_SPACING: Duration = Duration::from_millis(250);
const CURSEFORGE_DOWNLOAD_MIN_SPACING: Duration = Duration::from_millis(500);

#[track_caller]
fn fs_create_dir_all_logged(path: &Path) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display());
    let result = fs::create_dir_all(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_read_dir_logged(path: &Path) -> std::io::Result<fs::ReadDir> {
    tracing::debug!(target: "vertexlauncher/io", op = "read_dir", path = %path.display());
    let result = fs::read_dir(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_dir", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_read_to_string_logged(path: &Path) -> std::io::Result<String> {
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    let result = fs::read_to_string(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_copy_logged(source: &Path, destination: &Path) -> std::io::Result<u64> {
    tracing::debug!(target: "vertexlauncher/io", op = "copy", from = %source.display(), to = %destination.display());
    let result = fs::copy(source, destination);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "copy", from = %source.display(), to = %destination.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_write_logged(path: &Path, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "write", path = %path.display());
    let result = fs::write(path, bytes);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "write", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_remove_dir_all_logged(path: &Path) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "remove_dir_all", path = %path.display());
    let result = fs::remove_dir_all(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "remove_dir_all", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_rename_logged(source: &Path, destination: &Path) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "rename", from = %source.display(), to = %destination.display());
    let result = fs::rename(source, destination);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "rename", from = %source.display(), to = %destination.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_file_open_logged(path: &Path) -> std::io::Result<fs::File> {
    tracing::debug!(target: "vertexlauncher/io", op = "file_open", path = %path.display());
    let result = fs::File::open(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "file_open", path = %path.display(), error = %err);
    }
    result
}

#[derive(Debug, Default)]
pub struct ImportInstanceState {
    pub source_mode_index: usize,
    pub package_path: String,
    pub launcher_path: String,
    pub launcher_kind_index: usize,
    pub instance_name: String,
    pub error: Option<String>,
    preview_in_flight: bool,
    preview_request_serial: u64,
    preview_results_tx: Option<mpsc::Sender<(u64, Result<ImportPreview, String>)>>,
    preview_results_rx: Option<Arc<Mutex<mpsc::Receiver<(u64, Result<ImportPreview, String>)>>>>,
    pub import_in_flight: bool,
    pub import_latest_progress: Option<ImportProgress>,
    pub import_progress_tx: Option<mpsc::Sender<ImportProgress>>,
    pub import_progress_rx: Option<Arc<Mutex<mpsc::Receiver<ImportProgress>>>>,
    pub import_results_tx: Option<mpsc::Sender<ImportTaskResult>>,
    pub import_results_rx: Option<Arc<Mutex<mpsc::Receiver<ImportTaskResult>>>>,
    preview: Option<ImportPreview>,
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
    fn message(message: impl Into<String>) -> Self {
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

fn ensure_preview_channel(state: &mut ImportInstanceState) {
    if state.preview_results_tx.is_some() && state.preview_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(u64, Result<ImportPreview, String>)>();
    state.preview_results_tx = Some(tx);
    state.preview_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn poll_preview_results(state: &mut ImportInstanceState) {
    let Some(rx) = state.preview_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        return;
    };
    while let Ok((request_serial, result)) = receiver.try_recv() {
        if request_serial != state.preview_request_serial {
            continue;
        }
        state.preview_in_flight = false;
        match result {
            Ok(preview) => {
                if state.instance_name.trim().is_empty() {
                    state.instance_name = preview.detected_name.clone();
                }
                state.preview = Some(preview);
                state.error = None;
            }
            Err(err) => {
                state.preview = None;
                state.error = Some(err);
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ImportPreview {
    kind: ImportPreviewKind,
    detected_name: String,
    game_version: String,
    modloader: String,
    modloader_version: String,
    summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportPreviewKind {
    Manifest(ImportPackageKind),
    Launcher(LauncherKind),
}

impl ImportPreviewKind {
    fn label(self) -> &'static str {
        match self {
            ImportPreviewKind::Manifest(kind) => kind.label(),
            ImportPreviewKind::Launcher(kind) => kind.label(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportPackageKind {
    VertexPack,
    ModrinthPack,
    CurseForgePack,
}

impl ImportPackageKind {
    fn label(self) -> &'static str {
        match self {
            ImportPackageKind::VertexPack => "Vertex .vtmpack",
            ImportPackageKind::ModrinthPack => "Modrinth .mrpack",
            ImportPackageKind::CurseForgePack => "CurseForge modpack zip",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportMode {
    ManifestFile,
    LauncherDirectory,
}

impl ImportMode {
    fn from_index(index: usize) -> Self {
        match index {
            1 => Self::LauncherDirectory,
            _ => Self::ManifestFile,
        }
    }

    fn options() -> [&'static str; 2] {
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
    fn label(self) -> &'static str {
        match self {
            Self::Modrinth => "Modrinth launcher instance",
            Self::CurseForge => "CurseForge instance",
            Self::Prism => "Prism / MultiMC / PolyMC instance",
            Self::ATLauncher => "ATLauncher instance",
            Self::Unknown => "Generic launcher instance",
        }
    }
}

const LAUNCHER_KIND_OPTIONS: [&str; 5] = [
    "Auto-detect",
    "Modrinth",
    "CurseForge",
    "Prism / MultiMC",
    "ATLauncher",
];

fn selected_import_mode(state: &ImportInstanceState) -> ImportMode {
    ImportMode::from_index(state.source_mode_index)
}

fn selected_launcher_hint(state: &ImportInstanceState) -> Option<LauncherKind> {
    match state.launcher_kind_index {
        1 => Some(LauncherKind::Modrinth),
        2 => Some(LauncherKind::CurseForge),
        3 => Some(LauncherKind::Prism),
        4 => Some(LauncherKind::ATLauncher),
        _ => None,
    }
}

pub fn render(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut ImportInstanceState,
    curseforge_api_key_configured: bool,
) -> ModalAction {
    poll_preview_results(state);
    let mut action = ModalAction::None;
    let viewport_rect = ctx.input(|i| i.content_rect());
    let modal_max_width = (viewport_rect.width() * 0.85).max(1.0);
    let modal_max_height = (viewport_rect.height() * 0.82).max(1.0);
    let modal_pos = egui::pos2(
        (viewport_rect.center().x - modal_max_width * 0.5).clamp(
            viewport_rect.left(),
            viewport_rect.right() - modal_max_width,
        ),
        (viewport_rect.center().y - modal_max_height * 0.5).clamp(
            viewport_rect.top(),
            viewport_rect.bottom() - modal_max_height,
        ),
    );

    modal::show_scrim(ctx, "import_instance_modal_scrim", viewport_rect);
    if state.preview_in_flight || state.import_in_flight {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    egui::Window::new("Import Profile")
        .id(egui::Id::new("import_instance_modal_window"))
        .order(egui::Order::Foreground)
        .fixed_pos(modal_pos)
        .fixed_size(egui::vec2(modal_max_width, modal_max_height))
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .title_bar(false)
        .hscroll(false)
        .vscroll(true)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(modal::window_frame(ctx))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(MODAL_GAP_MD, MODAL_GAP_MD);
            let text_color = ui.visuals().text_color();
            let heading_style = LabelOptions {
                font_size: 34.0,
                line_height: 38.0,
                weight: 700,
                color: text_color,
                wrap: false,
                ..LabelOptions::default()
            };
            let body_style = LabelOptions {
                font_size: 18.0,
                line_height: 24.0,
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            };

            let _ = text_ui.label(
                ui,
                "instance_import_heading",
                "Import Profile",
                &heading_style,
            );
            let _ = text_ui.label(
                ui,
                "instance_import_subheading",
                "Import from a pack manifest or copy an instance out of another launcher.",
                &body_style,
            );

            let previous_mode = state.source_mode_index;
            let _ = settings_widgets::full_width_dropdown_row(
                text_ui,
                ui,
                "instance_import_mode",
                "Import source",
                Some("Choose whether to import from a pack manifest or an existing launcher instance folder."),
                &mut state.source_mode_index,
                &ImportMode::options(),
            );
            if state.source_mode_index != previous_mode {
                state.preview = None;
                state.error = None;
            }
            ui.add_space(MODAL_GAP_SM);

            match selected_import_mode(state) {
                ImportMode::ManifestFile => {
                    let previous_path = state.package_path.clone();
                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        "instance_import_package_path",
                        "Manifest file",
                        Some("Select a .vtmpack, .mrpack, or CurseForge modpack .zip file."),
                        &mut state.package_path,
                    );
                    if state.package_path != previous_path {
                        state.preview = None;
                        state.error = None;
                    }

                    ui.horizontal(|ui| {
                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_choose_file",
                            "Choose manifest",
                            (ui.available_width() * 0.5).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            if let Some(path) = pick_import_file() {
                                state.package_path = path.display().to_string();
                                load_preview_from_state(state);
                            }
                        }

                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_inspect_file",
                            "Inspect manifest",
                            (ui.available_width()).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            load_preview_from_state(state);
                        }
                    });

                    let highlight_curseforge_notice = !curseforge_api_key_configured
                        && (matches!(
                            state.preview.as_ref().map(|preview| preview.kind),
                            Some(ImportPreviewKind::Manifest(ImportPackageKind::CurseForgePack))
                        ) || state
                            .package_path
                            .trim()
                            .to_ascii_lowercase()
                            .ends_with(".zip"));
                    let _ = text_ui.label(
                        ui,
                        "instance_import_curseforge_notice",
                        "CurseForge modpack zips are supported, but they only work if you have a CurseForge API key in Settings. Vertex will fall back to Modrinth downloads when it can resolve an exact compatible match.",
                        &LabelOptions {
                            color: if highlight_curseforge_notice {
                                ui.visuals().error_fg_color
                            } else {
                                ui.visuals().weak_text_color()
                            },
                            wrap: true,
                            ..LabelOptions::default()
                        },
                    );
                }
                ImportMode::LauncherDirectory => {
                    let previous_path = state.launcher_path.clone();
                    let previous_launcher_kind = state.launcher_kind_index;
                    let _ = settings_widgets::full_width_dropdown_row(
                        text_ui,
                        ui,
                        "instance_import_launcher_kind",
                        "Launcher",
                        Some("Use Auto-detect unless you know which launcher produced the instance."),
                        &mut state.launcher_kind_index,
                        &LAUNCHER_KIND_OPTIONS,
                    );
                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        "instance_import_launcher_path",
                        "Instance folder",
                        Some("Choose the instance directory from Modrinth, CurseForge, Prism, ATLauncher, or another launcher."),
                        &mut state.launcher_path,
                    );
                    if state.launcher_path != previous_path
                        || state.launcher_kind_index != previous_launcher_kind
                    {
                        state.preview = None;
                        state.error = None;
                    }

                    ui.horizontal(|ui| {
                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_choose_folder",
                            "Choose folder",
                            (ui.available_width() * 0.5).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            if let Some(path) = pick_import_directory() {
                                state.launcher_path = path.display().to_string();
                                load_preview_from_state(state);
                            }
                        }

                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_inspect_launcher",
                            "Inspect folder",
                            (ui.available_width()).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            load_preview_from_state(state);
                        }
                    });
                }
            }

            ui.add_space(MODAL_GAP_SM);
            let _ = settings_widgets::full_width_text_input_row(
                text_ui,
                ui,
                "instance_import_name",
                "Imported profile name",
                Some("Defaults to the package name, but you can override it."),
                &mut state.instance_name,
            );

            if let Some(preview) = state.preview.as_ref() {
                ui.add_space(MODAL_GAP_SM);
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_title",
                        "Detected package",
                        &LabelOptions {
                            font_size: 20.0,
                            line_height: 24.0,
                            weight: 600,
                            color: ui.visuals().text_color(),
                            wrap: false,
                            ..LabelOptions::default()
                        },
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_kind",
                        preview.kind.label(),
                        &body_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_versions",
                        format!(
                            "Minecraft {} • {}",
                            preview.game_version,
                            format_loader_label(
                                preview.modloader.as_str(),
                                preview.modloader_version.as_str()
                            )
                        )
                        .as_str(),
                        &body_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_summary",
                        preview.summary.as_str(),
                        &body_style,
                    );
                });
            }

            if let Some(error) = state.error.as_deref() {
                let _ = text_ui.label(
                    ui,
                    "instance_import_error",
                    error,
                    &LabelOptions {
                        color: ui.visuals().error_fg_color,
                        wrap: true,
                        ..LabelOptions::default()
                    },
                );
            }

            if state.preview_in_flight {
                let _ = text_ui.label(
                    ui,
                    "instance_import_preview_in_flight",
                    "Inspecting import source in the background...",
                    &body_style,
                );
            }

            if state.import_in_flight {
                let progress = state.import_latest_progress.as_ref();
                let progress_fraction = progress
                    .and_then(|progress| {
                        (progress.total_steps > 0)
                            .then_some(progress.completed_steps as f32 / progress.total_steps as f32)
                    })
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                let progress_message = progress
                    .map(|progress| progress.message.as_str())
                    .unwrap_or("Importing profile in the background...");
                let progress_counts = progress
                    .map(|progress| {
                        format!(
                            "{} of {} steps complete",
                            progress.completed_steps.min(progress.total_steps),
                            progress.total_steps
                        )
                    })
                    .unwrap_or_else(|| "Preparing import task...".to_owned());
                ui.horizontal(|ui| {
                    ui.spinner();
                    let _ = text_ui.label(
                        ui,
                        "instance_import_in_flight",
                        progress_message,
                        &body_style,
                    );
                });
                let _ = text_ui.label(
                    ui,
                    "instance_import_in_flight_counts",
                    progress_counts.as_str(),
                    &body_style,
                );
                ui.add(
                    egui::ProgressBar::new(progress_fraction)
                        .desired_width(ui.available_width())
                        .show_percentage(),
                );
            }

            ui.add_space(MODAL_GAP_LG);
            ui.horizontal(|ui| {
                let button_style = ButtonOptions {
                    min_size: egui::vec2(160.0, style::CONTROL_HEIGHT),
                    text_color: ui.visuals().text_color(),
                    fill: ui.visuals().widgets.inactive.bg_fill,
                    fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                    fill_active: ui.visuals().widgets.active.bg_fill,
                    fill_selected: ui.visuals().selection.bg_fill,
                    stroke: ui.visuals().widgets.inactive.bg_stroke,
                    ..ButtonOptions::default()
                };
                if ui
                    .add_enabled_ui(!state.import_in_flight, |ui| {
                        text_ui.button(ui, "instance_import_cancel", "Cancel", &button_style)
                    })
                    .inner
                    .clicked()
                {
                    action = ModalAction::Cancel;
                }

                let import_disabled = state.import_in_flight || match selected_import_mode(state) {
                    ImportMode::ManifestFile => state.package_path.trim().is_empty(),
                    ImportMode::LauncherDirectory => state.launcher_path.trim().is_empty(),
                };
                if ui
                    .add_enabled_ui(!import_disabled, |ui| {
                        text_ui.button(
                            ui,
                            "instance_import_confirm",
                            "Import profile",
                            &button_style,
                        )
                    })
                    .inner
                    .clicked()
                {
                    if state.preview.is_none() {
                        load_preview_from_state(state);
                    }
                    if let Some(preview) = state.preview.as_ref() {
                        let instance_name = non_empty(state.instance_name.as_str())
                            .unwrap_or_else(|| preview.detected_name.clone());
                        action = ModalAction::Import(ImportRequest {
                            source: match selected_import_mode(state) {
                                ImportMode::ManifestFile => {
                                    ImportSource::ManifestFile(PathBuf::from(
                                        state.package_path.trim(),
                                    ))
                                }
                                ImportMode::LauncherDirectory => {
                                    ImportSource::LauncherDirectory {
                                        path: PathBuf::from(state.launcher_path.trim()),
                                        launcher: selected_launcher_hint(state),
                                    }
                                }
                            },
                            instance_name,
                            manual_curseforge_files: HashMap::new(),
                            max_concurrent_downloads: 4,
                        });
                    }
                }
            });
        });

    action
}

pub fn import_package_with_progress<F>(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: ImportRequest,
    mut progress: F,
) -> Result<InstanceRecord, ImportPackageError>
where
    F: FnMut(ImportProgress),
{
    match &request.source {
        ImportSource::ManifestFile(path) => {
            let preview = inspect_package(path.as_path()).map_err(ImportPackageError::message)?;
            match preview.kind {
                ImportPreviewKind::Manifest(ImportPackageKind::VertexPack) => {
                    import_vtmpack(store, installations_root, &request, &mut progress)
                        .map_err(ImportPackageError::message)
                }
                ImportPreviewKind::Manifest(ImportPackageKind::ModrinthPack) => {
                    import_mrpack(store, installations_root, &request, &mut progress)
                        .map_err(ImportPackageError::message)
                }
                ImportPreviewKind::Manifest(ImportPackageKind::CurseForgePack) => {
                    import_curseforge_pack(store, installations_root, &request, &mut progress)
                }
                ImportPreviewKind::Launcher(_) => Err(ImportPackageError::message(
                    "Launcher previews are not valid for manifest imports.",
                )),
            }
        }
        ImportSource::LauncherDirectory { .. } => {
            progress(import_progress("Copying launcher instance files...", 0, 0));
            import_launcher_instance(store, installations_root, &request)
                .map_err(ImportPackageError::message)
        }
    }
}

#[allow(dead_code)]
pub fn update_mrpack_instance_with_progress<F>(
    store: &mut InstanceStore,
    installations_root: &Path,
    instance_id: &str,
    package_path: &Path,
    mut progress: F,
) -> Result<InstanceRecord, String>
where
    F: FnMut(ImportProgress),
{
    let existing_instance = store
        .find(instance_id)
        .cloned()
        .ok_or_else(|| format!("instance {instance_id} was not found"))?;
    let existing_root = instance_root_path(installations_root, &existing_instance);
    let current_modpack_state = load_modpack_install_state(existing_root.as_path())
        .ok_or_else(|| "This instance is not tied to an updatable modpack yet.".to_owned())?;

    progress(import_progress("Reading updated .mrpack manifest...", 0, 1));
    let manifest = read_mrpack_manifest(package_path)?;
    let dependency_info = resolve_mrpack_dependencies(&manifest.dependencies)?;
    let override_steps = count_mrpack_override_entries(package_path)?;
    let total_steps = 6 + override_steps + manifest.files.len();
    let temp_root = unique_temp_instance_root(installations_root, existing_instance.id.as_str());

    fs_create_dir_all_logged(temp_root.as_path()).map_err(|err| {
        format!(
            "failed to create temp instance root {}: {err}",
            temp_root.display()
        )
    })?;
    fs_create_dir_all_logged(temp_root.join("mods").as_path())
        .map_err(|err| format!("failed to create temp mods directory: {err}"))?;

    progress(import_progress(
        "Building updated modpack files...",
        1,
        total_steps,
    ));
    if let Err(err) = populate_mrpack_instance(
        package_path,
        manifest.clone(),
        temp_root.as_path(),
        total_steps,
        &mut progress,
    ) {
        let _ = fs_remove_dir_all_logged(temp_root.as_path());
        return Err(err);
    }

    let new_base_manifest = build_mrpack_base_manifest(temp_root.as_path(), &manifest)?;
    let new_modpack_state = build_mrpack_install_state(package_path, &manifest, new_base_manifest);
    let current_manifest = load_content_manifest(existing_root.as_path());
    let mut final_manifest = new_modpack_state.base_manifest.clone();
    let pack_managed_paths =
        pack_managed_path_keys(&current_manifest, &current_modpack_state.base_manifest);

    progress(import_progress(
        "Preserving user-added content...",
        total_steps.saturating_sub(4),
        total_steps,
    ));
    preserve_non_pack_managed_content(
        existing_root.as_path(),
        temp_root.as_path(),
        &pack_managed_paths,
    )?;
    for (project_key, project) in current_manifest.projects {
        if !project.pack_managed {
            final_manifest.projects.insert(project_key, project);
        }
    }

    progress(import_progress(
        "Preserving worlds and servers...",
        total_steps.saturating_sub(3),
        total_steps,
    ));
    preserve_instance_user_state(existing_root.as_path(), temp_root.as_path())?;
    save_content_manifest(temp_root.as_path(), &final_manifest)?;
    save_modpack_install_state(temp_root.as_path(), &new_modpack_state)?;

    progress(import_progress(
        "Finalizing updated instance...",
        total_steps.saturating_sub(2),
        total_steps,
    ));
    swap_instance_root(existing_root.as_path(), temp_root.as_path())?;

    let instance = store
        .find_mut(instance_id)
        .ok_or_else(|| format!("instance {instance_id} disappeared during update"))?;
    instance.game_version = dependency_info.game_version;
    instance.modloader = dependency_info.modloader;
    instance.modloader_version = dependency_info.modloader_version;

    progress(import_progress(
        "Update complete.",
        total_steps,
        total_steps,
    ));
    Ok(instance.clone())
}

fn load_preview_from_state(state: &mut ImportInstanceState) {
    ensure_preview_channel(state);
    let Some(tx) = state.preview_results_tx.as_ref().cloned() else {
        return;
    };
    let request = match selected_import_mode(state) {
        ImportMode::ManifestFile => {
            let path = PathBuf::from(state.package_path.trim());
            if path.as_os_str().is_empty() {
                state.preview = None;
                state.error = Some(
                    "Choose a .vtmpack, .mrpack, or CurseForge modpack .zip file first.".to_owned(),
                );
                return;
            }
            (path, selected_launcher_hint(state), true)
        }
        ImportMode::LauncherDirectory => {
            let path = PathBuf::from(state.launcher_path.trim());
            if path.as_os_str().is_empty() {
                state.preview = None;
                state.error = Some("Choose an instance folder first.".to_owned());
                return;
            }
            (path, selected_launcher_hint(state), false)
        }
    };

    state.preview_request_serial = state.preview_request_serial.saturating_add(1);
    let request_serial = state.preview_request_serial;
    state.preview_in_flight = true;
    state.error = None;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            let (path, launcher_hint, manifest_mode) = request;
            if manifest_mode {
                inspect_package(path.as_path())
            } else {
                inspect_launcher_instance(path.as_path(), launcher_hint)
            }
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);
        let _ = tx.send((request_serial, result));
    });
}

fn pick_import_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Launcher profiles", &["vtmpack", "mrpack", "zip"])
        .add_filter("Vertex packs", &["vtmpack"])
        .add_filter("Modrinth packs", &["mrpack"])
        .add_filter("CurseForge packs", &["zip"])
        .pick_file()
}

fn pick_import_directory() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

fn inspect_package(path: &Path) -> Result<ImportPreview, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "vtmpack" => inspect_vtmpack(path),
        "mrpack" => inspect_mrpack(path),
        "zip" => inspect_curseforge_pack(path),
        _ => Err(format!(
            "Unsupported import file {}. Expected .vtmpack, .mrpack, or a CurseForge modpack .zip.",
            path.display()
        )),
    }
}

fn inspect_vtmpack(path: &Path) -> Result<ImportPreview, String> {
    let manifest = read_vtmpack_manifest(path)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Manifest(ImportPackageKind::VertexPack),
        detected_name: manifest.instance.name.clone(),
        game_version: manifest.instance.game_version.clone(),
        modloader: manifest.instance.modloader.clone(),
        modloader_version: manifest.instance.modloader_version.clone(),
        summary: format!(
            "{} for Minecraft {} ({}) with {} downloadable items, {} bundled mods, {} config files.",
            manifest.instance.name,
            manifest.instance.game_version,
            format_loader_label(
                manifest.instance.modloader.as_str(),
                manifest.instance.modloader_version.as_str()
            ),
            manifest.downloadable_content.len(),
            manifest.bundled_mods.len(),
            manifest.configs.len()
        ),
    })
}

fn inspect_mrpack(path: &Path) -> Result<ImportPreview, String> {
    let manifest = read_mrpack_manifest(path)?;
    let dependency_info = resolve_mrpack_dependencies(&manifest.dependencies)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Manifest(ImportPackageKind::ModrinthPack),
        detected_name: non_empty(manifest.name.as_str())
            .unwrap_or_else(|| "Imported Modrinth Pack".to_owned()),
        game_version: dependency_info.game_version.clone(),
        modloader: dependency_info.modloader.clone(),
        modloader_version: dependency_info.modloader_version.clone(),
        summary: format!(
            "{} {} for Minecraft {} ({}) with {} packaged files.",
            non_empty(manifest.name.as_str()).unwrap_or_else(|| "Modrinth pack".to_owned()),
            non_empty(manifest.version_id.as_str()).unwrap_or_default(),
            dependency_info.game_version,
            format_loader_label(
                dependency_info.modloader.as_str(),
                dependency_info.modloader_version.as_str()
            ),
            manifest.files.len()
        )
        .trim()
        .to_owned(),
    })
}

fn inspect_curseforge_pack(path: &Path) -> Result<ImportPreview, String> {
    let manifest = read_curseforge_pack_manifest(path)?;
    let dependency_info = resolve_curseforge_pack_dependencies(&manifest.minecraft)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Manifest(ImportPackageKind::CurseForgePack),
        detected_name: non_empty(manifest.name.as_str())
            .unwrap_or_else(|| "Imported CurseForge Pack".to_owned()),
        game_version: dependency_info.game_version.clone(),
        modloader: dependency_info.modloader.clone(),
        modloader_version: dependency_info.modloader_version.clone(),
        summary: format!(
            "{} {} for Minecraft {} ({}) with {} packaged files.",
            non_empty(manifest.name.as_str()).unwrap_or_else(|| "CurseForge pack".to_owned()),
            non_empty(manifest.version.as_str()).unwrap_or_default(),
            dependency_info.game_version,
            format_loader_label(
                dependency_info.modloader.as_str(),
                dependency_info.modloader_version.as_str()
            ),
            manifest.files.len()
        )
        .trim()
        .to_owned(),
    })
}

#[derive(Clone, Debug)]
struct LauncherInspection {
    launcher: LauncherKind,
    name: String,
    description: Option<String>,
    game_version: String,
    modloader: String,
    modloader_version: String,
    summary: String,
    source_root: PathBuf,
    managed_manifest: ContentInstallManifest,
}

fn inspect_launcher_instance(
    path: &Path,
    launcher_hint: Option<LauncherKind>,
) -> Result<ImportPreview, String> {
    let inspection = inspect_launcher_details(path, launcher_hint)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Launcher(inspection.launcher),
        detected_name: inspection.name,
        game_version: inspection.game_version,
        modloader: inspection.modloader,
        modloader_version: inspection.modloader_version,
        summary: inspection.summary,
    })
}

fn inspect_launcher_details(
    path: &Path,
    launcher_hint: Option<LauncherKind>,
) -> Result<LauncherInspection, String> {
    if !path.exists() {
        return Err(format!(
            "Instance folder {} does not exist.",
            path.display()
        ));
    }
    if !path.is_dir() {
        return Err(format!(
            "Import source {} is not a directory.",
            path.display()
        ));
    }

    let launcher = launcher_hint.unwrap_or_else(|| detect_launcher_kind(path));
    match launcher {
        LauncherKind::Modrinth => inspect_modrinth_launcher_instance(path),
        LauncherKind::CurseForge => inspect_curseforge_launcher_instance(path),
        LauncherKind::Prism => inspect_prism_launcher_instance(path),
        LauncherKind::ATLauncher => inspect_atlauncher_instance(path),
        LauncherKind::Unknown => inspect_generic_launcher_instance(path),
    }
}

fn import_launcher_instance(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
) -> Result<InstanceRecord, String> {
    let ImportSource::LauncherDirectory { path, launcher } = &request.source else {
        return Err("Launcher import requires an instance directory source.".to_owned());
    };

    let inspection = inspect_launcher_details(path.as_path(), *launcher)?;
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: inspection.description.clone(),
            thumbnail_path: None,
            modloader: default_if_blank(inspection.modloader.as_str(), "Vanilla".to_owned()),
            game_version: default_if_blank(inspection.game_version.as_str(), "latest".to_owned()),
            modloader_version: inspection.modloader_version.clone(),
        },
    )
    .map_err(|err| format!("failed to create imported profile: {err}"))?;
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) = copy_launcher_instance_content(
        inspection.source_root.as_path(),
        instance_root.as_path(),
        &inspection.managed_manifest,
    ) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    Ok(instance)
}

fn detect_launcher_kind(path: &Path) -> LauncherKind {
    if path.join(CONTENT_MANIFEST_FILE_NAME).is_file() {
        LauncherKind::Unknown
    } else if path.join("profile.json").is_file() || looks_like_modrinth_profile_path(path) {
        LauncherKind::Modrinth
    } else if path.join("minecraftinstance.json").is_file() {
        LauncherKind::CurseForge
    } else if path.join("instance.cfg").is_file() || path.join("mmc-pack.json").is_file() {
        LauncherKind::Prism
    } else if path.join("instance.json").is_file() {
        LauncherKind::ATLauncher
    } else {
        LauncherKind::Unknown
    }
}

fn inspect_modrinth_launcher_instance(path: &Path) -> Result<LauncherInspection, String> {
    if !path.join("profile.json").is_file() {
        let mut inspection =
            inspect_generic_launcher_instance_with_launcher(path, LauncherKind::Modrinth)?;
        let inferred = infer_modrinth_profile_metadata(path);
        if let Some(game_version) = inferred.game_version {
            inspection.game_version = game_version;
        }
        if let Some(modloader) = inferred.modloader {
            inspection.modloader = modloader;
        }
        if let Some(modloader_version) = inferred.modloader_version {
            inspection.modloader_version = modloader_version;
        }
        inspection.description = Some(
            "Imported from a Modrinth instance folder without profile.json metadata.".to_owned(),
        );
        inspection.summary = format!(
            "Detected {} by location. No profile.json was present, so Minecraft and loader metadata were inferred from profile contents where possible; files will still be copied from the instance root.",
            inspection.launcher.label()
        );
        return Ok(inspection);
    }

    let profile = read_json_file(path.join("profile.json").as_path())?;
    let source_root = path.to_path_buf();
    let name = first_non_empty([
        json_string_at_path(&profile, &["metadata", "name"]),
        json_string_at_path(&profile, &["name"]),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported Modrinth Instance".to_owned());
    let game_version = first_non_empty([
        json_string_at_path(&profile, &["metadata", "game_version"]),
        json_string_at_path(&profile, &["game_version"]),
        json_string_at_path(&profile, &["metadata", "minecraft_version"]),
        json_string_at_path(&profile, &["minecraft_version"]),
    ])
    .unwrap_or_else(|| "latest".to_owned());
    let (modloader, modloader_version) = infer_loader_pair(
        first_non_empty([
            json_string_at_path(&profile, &["metadata", "loader"]),
            json_string_at_path(&profile, &["loader"]),
            json_string_at_path(&profile, &["loader_type"]),
        ]),
        first_non_empty([
            json_string_at_path(&profile, &["metadata", "loader_version"]),
            json_string_at_path(&profile, &["loader_version"]),
            json_string_at_path(&profile, &["loaderVersion"]),
        ]),
    );
    let mut managed_manifest =
        load_existing_managed_manifest(path).unwrap_or_else(|_| ContentInstallManifest::default());
    if managed_manifest.projects.is_empty() {
        managed_manifest = extract_managed_manifest_from_json(
            &profile,
            source_root.as_path(),
            ManagedContentSourceHint::Modrinth,
        );
    }
    Ok(build_launcher_inspection(
        LauncherKind::Modrinth,
        name,
        Some("Imported from an existing Modrinth launcher instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

fn looks_like_modrinth_profile_path(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(parent_name) = parent.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    if parent_name != "profiles" {
        return false;
    }
    path.ancestors().any(|ancestor| {
        ancestor
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name == "ModrinthApp")
    })
}

#[derive(Default)]
struct ModrinthProfileMetadata {
    game_version: Option<String>,
    modloader: Option<String>,
    modloader_version: Option<String>,
}

fn infer_modrinth_profile_metadata(path: &Path) -> ModrinthProfileMetadata {
    let mut metadata = ModrinthProfileMetadata::default();
    metadata.game_version = infer_modrinth_game_version_from_telemetry(path);

    let (modloader, modloader_version) = infer_modrinth_loader_from_profile(path);
    metadata.modloader = modloader;
    metadata.modloader_version = modloader_version;

    if let Some(app_root) = modrinth_app_root(path) {
        refine_modrinth_metadata_from_meta_cache(app_root.as_path(), &mut metadata);
    }

    if metadata.game_version.is_none() {
        metadata.game_version = infer_modrinth_game_version_from_filenames(path);
    }

    metadata
}

fn infer_modrinth_game_version_from_telemetry(path: &Path) -> Option<String> {
    let telemetry_dir = path.join("logs").join("telemetry");
    let mut files = fs::read_dir(telemetry_dir)
        .ok()?
        .flatten()
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| entry.file_name());
    files.reverse();

    for entry in files {
        let raw = fs::read_to_string(entry.path()).ok()?;
        for line in raw.lines().rev() {
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && let Some(game_version) = value.get("game_version").and_then(Value::as_str)
            {
                if let Some(normalized) = normalize_minecraft_game_version(game_version) {
                    return Some(normalized);
                }
            }
        }
    }

    None
}

fn infer_modrinth_loader_from_profile(path: &Path) -> (Option<String>, Option<String>) {
    if let Some((loader, version)) = infer_modrinth_loader_from_dependencies_file(
        path.join("config/fabric_loader_dependencies.json")
            .as_path(),
    ) {
        return (Some(loader), Some(version));
    }

    if let Some((loader, version)) = infer_modrinth_loader_from_mod_filenames(path) {
        return (Some(loader), version);
    }

    (None, None)
}

fn infer_modrinth_loader_from_dependencies_file(path: &Path) -> Option<(String, String)> {
    let value = read_json_file_optional(path).ok()??;
    let fabric_requirement = value
        .get("overrides")
        .and_then(|value| value.get("fabricloader"))
        .and_then(|value| value.get("+depends"))
        .and_then(|value| value.get("fabricloader"))
        .and_then(Value::as_str)
        .and_then(clean_version_requirement)?;
    Some(("Fabric".to_owned(), fabric_requirement))
}

fn clean_version_requirement(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut out = String::new();
    let mut started = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            started = true;
            out.push(ch);
            continue;
        }
        if started && ch == '.' {
            out.push(ch);
            continue;
        }
        if started {
            break;
        }
    }

    if out.is_empty() { None } else { Some(out) }
}

fn infer_modrinth_loader_from_mod_filenames(path: &Path) -> Option<(String, Option<String>)> {
    let mods_dir = path.join("mods");
    let entries = fs::read_dir(mods_dir).ok()?;
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if file_name.contains("fabric") {
            return Some(("Fabric".to_owned(), None));
        }
        if file_name.contains("quilt") {
            return Some(("Quilt".to_owned(), None));
        }
        if file_name.contains("neoforge") {
            return Some(("NeoForge".to_owned(), None));
        }
        if file_name.contains("forge") {
            return Some(("Forge".to_owned(), None));
        }
    }
    None
}

fn infer_modrinth_game_version_from_filenames(path: &Path) -> Option<String> {
    let mods_dir = path.join("mods");
    let entries = fs::read_dir(mods_dir).ok()?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if let Some(version) = find_minecraft_version_in_text(file_name.as_ref()) {
            return Some(version);
        }
    }
    None
}

fn find_minecraft_version_in_text(text: &str) -> Option<String> {
    let chars = text.chars().collect::<Vec<_>>();
    for start in 0..chars.len() {
        if !chars[start].is_ascii_digit() {
            continue;
        }
        let mut end = start;
        let mut dot_count = 0usize;
        while end < chars.len() && (chars[end].is_ascii_digit() || chars[end] == '.') {
            if chars[end] == '.' {
                dot_count += 1;
            }
            end += 1;
        }
        if dot_count >= 2 {
            let candidate = chars[start..end].iter().collect::<String>();
            if candidate.split('.').all(|segment| !segment.is_empty())
                && let Some(normalized) = normalize_minecraft_game_version(&candidate)
            {
                return Some(normalized);
            }
        }
    }
    None
}

fn modrinth_app_root(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        ancestor
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name == "ModrinthApp")
            .then(|| ancestor.to_path_buf())
    })
}

fn refine_modrinth_metadata_from_meta_cache(
    app_root: &Path,
    metadata: &mut ModrinthProfileMetadata,
) {
    let versions_dir = app_root.join("meta").join("versions");
    let Ok(entries) = fs::read_dir(versions_dir) else {
        return;
    };

    let game_version = metadata.game_version.clone();
    for entry in entries.flatten() {
        let version_name = entry.file_name().to_string_lossy().to_string();
        let Some(version_json) =
            read_meta_version_file(entry.path().as_path(), version_name.as_str())
        else {
            continue;
        };

        if let Some(expected_game_version) = game_version.as_deref()
            && !version_name.starts_with(expected_game_version)
        {
            continue;
        }

        if metadata.modloader.is_none() || metadata.modloader_version.is_none() {
            if let Some((loader, loader_version)) = infer_loader_from_meta_version(&version_json) {
                metadata.modloader.get_or_insert(loader);
                metadata.modloader_version.get_or_insert(loader_version);
            }
        }

        if metadata.game_version.is_none() {
            if let Some(version) = normalize_minecraft_game_version(&version_name)
                .or_else(|| {
                    version_json
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(normalize_minecraft_game_version)
                })
                .or_else(|| {
                    version_json
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(find_minecraft_version_in_text)
                })
            {
                metadata.game_version = Some(version);
            }
        }

        if metadata.game_version.is_some()
            && metadata.modloader.is_some()
            && metadata.modloader_version.is_some()
        {
            break;
        }
    }
}

fn read_meta_version_file(dir: &Path, dir_name: &str) -> Option<Value> {
    let path = dir.join(format!("{dir_name}.json"));
    read_json_file_optional(path.as_path()).ok().flatten()
}

fn infer_loader_from_meta_version(value: &Value) -> Option<(String, String)> {
    let libraries = value.get("libraries")?.as_array()?;
    for library in libraries {
        let name = library
            .get("name")
            .and_then(Value::as_str)?
            .to_ascii_lowercase();
        if let Some(version) = name.strip_prefix("net.fabricmc:fabric-loader:") {
            return Some(("Fabric".to_owned(), version.to_owned()));
        }
        if let Some(version) = name.strip_prefix("org.quiltmc:quilt-loader:") {
            return Some(("Quilt".to_owned(), version.to_owned()));
        }
        if let Some(version) = name.strip_prefix("net.neoforged:neoforge:") {
            return Some(("NeoForge".to_owned(), version.to_owned()));
        }
        if let Some(version) = name.strip_prefix("net.minecraftforge:forge:") {
            return Some(("Forge".to_owned(), version.to_owned()));
        }
    }
    None
}

fn inspect_curseforge_launcher_instance(path: &Path) -> Result<LauncherInspection, String> {
    let manifest = read_json_file(path.join("minecraftinstance.json").as_path())?;
    let source_root = path.to_path_buf();
    let name = first_non_empty([
        json_string_at_path(&manifest, &["name"]),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported CurseForge Instance".to_owned());
    let game_version = first_non_empty([
        json_string_at_path(&manifest, &["gameVersion"]),
        json_string_at_path(&manifest, &["minecraftVersion"]),
        json_string_at_path(&manifest, &["baseModLoader", "minecraftVersion"]),
    ])
    .unwrap_or_else(|| "latest".to_owned());
    let loader_hint = first_non_empty([
        json_string_at_path(&manifest, &["baseModLoader", "name"]),
        json_string_at_path(&manifest, &["baseModLoader", "modLoader"]),
        json_string_at_path(&manifest, &["modLoader"]),
    ]);
    let loader_version_hint = first_non_empty([
        json_string_at_path(&manifest, &["baseModLoader", "forgeVersion"]),
        json_string_at_path(&manifest, &["baseModLoader", "version"]),
        json_string_at_path(&manifest, &["modLoaderVersion"]),
    ]);
    let (modloader, modloader_version) = infer_loader_pair(loader_hint, loader_version_hint);
    let mut managed_manifest =
        load_existing_managed_manifest(path).unwrap_or_else(|_| ContentInstallManifest::default());
    if managed_manifest.projects.is_empty() {
        managed_manifest = extract_managed_manifest_from_json(
            &manifest,
            source_root.as_path(),
            ManagedContentSourceHint::CurseForge,
        );
    }
    Ok(build_launcher_inspection(
        LauncherKind::CurseForge,
        name,
        Some("Imported from an existing CurseForge instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

fn inspect_prism_launcher_instance(path: &Path) -> Result<LauncherInspection, String> {
    let source_root = if path.join(".minecraft").is_dir() {
        path.join(".minecraft")
    } else {
        path.to_path_buf()
    };
    let cfg = read_key_value_file(path.join("instance.cfg").as_path()).unwrap_or_default();
    let pack_json = read_json_file_optional(path.join("mmc-pack.json").as_path())?;
    let name = first_non_empty([
        cfg.get("name").cloned(),
        pack_json
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["name"])),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported Prism Instance".to_owned());
    let (game_version, modloader, modloader_version) =
        parse_prism_versions(pack_json.as_ref(), cfg.get("MCVersion").cloned());
    let managed_manifest =
        load_existing_managed_manifest(source_root.as_path()).unwrap_or_default();
    Ok(build_launcher_inspection(
        LauncherKind::Prism,
        name,
        Some("Imported from a Prism / MultiMC style instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

fn inspect_atlauncher_instance(path: &Path) -> Result<LauncherInspection, String> {
    let manifest = read_json_file_optional(path.join("instance.json").as_path())?;
    let source_root = path.to_path_buf();
    let name = first_non_empty([
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["name"])),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported ATLauncher Instance".to_owned());
    let game_version = first_non_empty([
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["minecraft", "version"])),
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["minecraftVersion"])),
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["version"])),
    ])
    .unwrap_or_else(|| "latest".to_owned());
    let (modloader, modloader_version) = infer_loader_pair(
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["loader"])),
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["loaderVersion"])),
    );
    let mut managed_manifest =
        load_existing_managed_manifest(path).unwrap_or_else(|_| ContentInstallManifest::default());
    if managed_manifest.projects.is_empty()
        && let Some(value) = manifest.as_ref()
    {
        managed_manifest = extract_managed_manifest_from_json(
            value,
            source_root.as_path(),
            ManagedContentSourceHint::Auto,
        );
    }
    Ok(build_launcher_inspection(
        LauncherKind::ATLauncher,
        name,
        Some("Imported from an existing ATLauncher instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

fn inspect_generic_launcher_instance(path: &Path) -> Result<LauncherInspection, String> {
    inspect_generic_launcher_instance_with_launcher(path, LauncherKind::Unknown)
}

fn inspect_generic_launcher_instance_with_launcher(
    path: &Path,
    launcher: LauncherKind,
) -> Result<LauncherInspection, String> {
    if !path.is_dir() {
        return Err(format!("{} is not a directory.", path.display()));
    }
    let managed_manifest = load_existing_managed_manifest(path).unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| "Imported Instance".to_owned());
    Ok(build_launcher_inspection(
        launcher,
        name,
        Some(format!(
            "Imported by copying files from {}.",
            launcher.label()
        )),
        "latest".to_owned(),
        "Vanilla".to_owned(),
        String::new(),
        path.to_path_buf(),
        managed_manifest,
    ))
}

fn build_launcher_inspection(
    launcher: LauncherKind,
    name: String,
    description: Option<String>,
    game_version: String,
    modloader: String,
    modloader_version: String,
    source_root: PathBuf,
    managed_manifest: ContentInstallManifest,
) -> LauncherInspection {
    let mods_count = count_regular_files(source_root.join("mods").as_path());
    let config_count = count_regular_files(source_root.join("config").as_path());
    let managed_count = managed_manifest.projects.len();
    LauncherInspection {
        launcher,
        name,
        description,
        game_version: default_if_blank(game_version.as_str(), "latest".to_owned()),
        modloader: default_if_blank(modloader.as_str(), "Vanilla".to_owned()),
        modloader_version,
        summary: format!(
            "Detected {} with {} managed projects, {} mods, and {} config files.",
            launcher.label(),
            managed_count,
            mods_count,
            config_count
        ),
        source_root,
        managed_manifest,
    }
}

fn copy_launcher_instance_content(
    source_root: &Path,
    destination_root: &Path,
    managed_manifest: &ContentInstallManifest,
) -> Result<(), String> {
    copy_dir_recursive(source_root, source_root, destination_root)?;
    if !managed_manifest.projects.is_empty() {
        let raw = toml::to_string_pretty(managed_manifest)
            .map_err(|err| format!("failed to serialize managed import manifest: {err}"))?;
        fs_write_logged(
            destination_root.join(CONTENT_MANIFEST_FILE_NAME).as_path(),
            raw,
        )
        .map_err(|err| {
            format!(
                "failed to write managed import manifest into {}: {err}",
                destination_root.display()
            )
        })?;
    }
    Ok(())
}

fn copy_dir_recursive(root: &Path, current: &Path, destination_root: &Path) -> Result<(), String> {
    let entries = fs_read_dir_logged(current)
        .map_err(|err| format!("failed to read {}: {err}", current.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|err| format!("failed to normalize {}: {err}", path.display()))?;
        if should_skip_import_path(relative) {
            continue;
        }
        let destination = destination_root.join(relative);
        if path.is_dir() {
            fs_create_dir_all_logged(destination.as_path())
                .map_err(|err| format!("failed to create {}: {err}", destination.display()))?;
            copy_dir_recursive(root, path.as_path(), destination_root)?;
        } else if path.is_file() {
            if let Some(parent) = destination.parent() {
                fs_create_dir_all_logged(parent)
                    .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
            }
            fs_copy_logged(path.as_path(), destination.as_path()).map_err(|err| {
                format!(
                    "failed to copy {} to {}: {err}",
                    path.display(),
                    destination.display()
                )
            })?;
        }
    }
    Ok(())
}

fn should_skip_import_path(relative: &Path) -> bool {
    let normalized = relative.to_string_lossy().replace('\\', "/");
    if normalized.is_empty() {
        return false;
    }
    let skip_exact = [
        "instance.cfg",
        "mmc-pack.json",
        "profile.json",
        "minecraftinstance.json",
        "instance.json",
        CONTENT_MANIFEST_FILE_NAME,
    ];
    if skip_exact
        .iter()
        .any(|candidate| normalized.eq_ignore_ascii_case(candidate))
    {
        return true;
    }
    let skip_prefixes = [
        "logs/",
        "crash-reports/",
        "versions/",
        "libraries/",
        "natives/",
        ".cache/",
        "cache/",
        "downloads/",
    ];
    skip_prefixes
        .iter()
        .any(|prefix| normalized.to_ascii_lowercase().starts_with(prefix))
}

fn read_json_file(path: &Path) -> Result<Value, String> {
    let raw = fs_read_to_string_logged(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn read_json_file_optional(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    read_json_file(path).map(Some)
}

fn read_key_value_file(path: &Path) -> Result<HashMap<String, String>, String> {
    let raw = fs_read_to_string_logged(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut values = HashMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            values.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    Ok(values)
}

fn parse_prism_versions(
    pack_json: Option<&Value>,
    cfg_game_version: Option<String>,
) -> (String, String, String) {
    let mut game_version = cfg_game_version.unwrap_or_else(|| "latest".to_owned());
    let mut loader = "Vanilla".to_owned();
    let mut loader_version = String::new();

    if let Some(Value::Array(components)) =
        pack_json.and_then(|value| value.get("components")).cloned()
    {
        for component in components {
            let uid = component
                .get("uid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let version = component
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if uid.contains("minecraft") && game_version == "latest" && !version.trim().is_empty() {
                game_version = version.clone();
            }
            if uid.contains("fabric") {
                loader = "Fabric".to_owned();
                loader_version = version;
            } else if uid.contains("neoforge") {
                loader = "NeoForge".to_owned();
                loader_version = version;
            } else if uid.contains("forge") {
                loader = "Forge".to_owned();
                loader_version = version;
            } else if uid.contains("quilt") {
                loader = "Quilt".to_owned();
                loader_version = version;
            }
        }
    }

    (game_version, loader, loader_version)
}

fn json_string_at_path(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn first_non_empty<const N: usize>(values: [Option<String>; N]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
}

fn infer_loader_pair(
    loader_hint: Option<String>,
    version_hint: Option<String>,
) -> (String, String) {
    let loader_hint = loader_hint.unwrap_or_else(|| "Vanilla".to_owned());
    let loader_hint_trimmed = loader_hint.trim().to_owned();
    let loader_hint_lower = loader_hint_trimmed.to_ascii_lowercase();
    let version_hint = version_hint.unwrap_or_default();
    if loader_hint_lower.contains("neoforge") {
        return (
            "NeoForge".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    if loader_hint_lower.contains("fabric") {
        return (
            "Fabric".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    if loader_hint_lower.contains("quilt") {
        return (
            "Quilt".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    if loader_hint_lower.contains("forge") {
        return (
            "Forge".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    (
        default_if_blank(loader_hint_trimmed.as_str(), "Vanilla".to_owned()),
        version_hint,
    )
}

fn trailing_loader_version(loader_hint: &str, explicit_version: &str) -> String {
    let explicit = explicit_version.trim();
    if !explicit.is_empty() {
        return explicit.to_owned();
    }
    loader_hint
        .split_once('-')
        .map(|(_, version)| version.trim().to_owned())
        .unwrap_or_default()
}

fn load_existing_managed_manifest(path: &Path) -> Result<ContentInstallManifest, String> {
    let manifest_path = path.join(CONTENT_MANIFEST_FILE_NAME);
    if !manifest_path.exists() {
        return Ok(ContentInstallManifest::default());
    }
    let raw = fs_read_to_string_logged(manifest_path.as_path())
        .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;
    toml::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", manifest_path.display()))
}

#[derive(Clone, Copy)]
enum ManagedContentSourceHint {
    Auto,
    Modrinth,
    CurseForge,
}

fn extract_managed_manifest_from_json(
    value: &Value,
    source_root: &Path,
    source_hint: ManagedContentSourceHint,
) -> ContentInstallManifest {
    let mut manifest = ContentInstallManifest::default();
    walk_json_for_projects(value, source_root, source_hint, &mut manifest);
    manifest
}

fn walk_json_for_projects(
    value: &Value,
    source_root: &Path,
    source_hint: ManagedContentSourceHint,
    manifest: &mut ContentInstallManifest,
) {
    maybe_add_project_from_json(value, source_root, source_hint, manifest);
    match value {
        Value::Object(map) => {
            for child in map.values() {
                walk_json_for_projects(child, source_root, source_hint, manifest);
            }
        }
        Value::Array(values) => {
            for child in values {
                walk_json_for_projects(child, source_root, source_hint, manifest);
            }
        }
        _ => {}
    }
}

fn maybe_add_project_from_json(
    value: &Value,
    source_root: &Path,
    source_hint: ManagedContentSourceHint,
    manifest: &mut ContentInstallManifest,
) {
    let Value::Object(map) = value else {
        return;
    };

    let modrinth_project_id = json_object_string(
        map,
        &[
            "project_id",
            "projectId",
            "modrinth_project_id",
            "modrinthProjectId",
        ],
    );
    let curseforge_project_id = json_object_u64(
        map,
        &[
            "addonID",
            "addonId",
            "projectID",
            "projectId",
            "curseforge_project_id",
            "curseforgeProjectId",
        ],
    );
    let source = match source_hint {
        ManagedContentSourceHint::Modrinth if modrinth_project_id.is_some() => Some("modrinth"),
        ManagedContentSourceHint::CurseForge if curseforge_project_id.is_some() => {
            Some("curseforge")
        }
        ManagedContentSourceHint::Auto => {
            if modrinth_project_id.is_some() {
                Some("modrinth")
            } else if curseforge_project_id.is_some() {
                Some("curseforge")
            } else {
                None
            }
        }
        _ => None,
    };
    if source.is_none() {
        return;
    }

    let version_id = first_non_empty([
        json_object_string(
            map,
            &[
                "version_id",
                "versionId",
                "fileId",
                "fileID",
                "gameVersionFileID",
            ],
        ),
        map.get("installedFile")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_u64)
            .map(|value| value.to_string()),
    ])
    .unwrap_or_default();
    let metadata_file_name = json_object_string(map, &["fileName", "filename", "file_name"])
        .or_else(|| {
            map.get("installedFile")
                .and_then(|value| value.get("fileName"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let version_name = json_object_string(map, &["version_name", "versionName"])
        .or_else(|| metadata_file_name.clone())
        .unwrap_or_default();
    let name = json_object_string(map, &["name", "title"])
        .or_else(|| {
            map.get("installedFile")
                .and_then(|value| value.get("displayName"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| version_name.clone());

    let Some(metadata_file_name) = metadata_file_name.as_deref() else {
        return;
    };

    let file_path = first_non_empty([
        json_object_string(map, &["path", "file_path", "filePath"]).and_then(|value| {
            resolve_existing_relative_file_path(source_root, value.as_str(), metadata_file_name)
        }),
        Some(metadata_file_name.to_owned()).and_then(|value| {
            resolve_existing_relative_file_path(source_root, value.as_str(), metadata_file_name)
        }),
    ]);
    let Some(file_path) = file_path else {
        return;
    };

    let project_key = if let Some(id) = modrinth_project_id.as_ref() {
        format!("modrinth:{id}")
    } else if let Some(id) = curseforge_project_id {
        format!("curseforge:{id}")
    } else {
        normalize_project_key(file_path.as_str())
    };
    manifest.projects.insert(
        project_key.clone(),
        InstalledContentProject {
            project_key,
            name,
            file_path,
            modrinth_project_id,
            curseforge_project_id,
            selected_source: match source {
                Some("modrinth") => Some(managed_content::ManagedContentSource::Modrinth),
                Some("curseforge") => Some(managed_content::ManagedContentSource::CurseForge),
                _ => None,
            },
            selected_version_id: non_empty(version_id.as_str()),
            selected_version_name: non_empty(version_name.as_str()),
            ..InstalledContentProject::default()
        },
    );
}

fn json_object_string(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn json_object_u64(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        map.get(*key).and_then(|value| match value {
            Value::Number(number) => number.as_u64(),
            Value::String(raw) => raw.trim().parse::<u64>().ok(),
            _ => None,
        })
    })
}

fn resolve_existing_relative_file_path(
    source_root: &Path,
    raw: &str,
    expected_file_name: &str,
) -> Option<String> {
    let normalized = normalize_project_key(raw);
    if normalized.is_empty() {
        return None;
    }

    let direct = source_root.join(normalized.as_str());
    if direct.is_file() && file_name_matches(direct.as_path(), expected_file_name) {
        return Some(normalized);
    }

    let known_dirs = ["mods", "resourcepacks", "shaderpacks", "datapacks"];
    for dir in known_dirs {
        let candidate = source_root.join(dir).join(raw);
        if candidate.is_file() && file_name_matches(candidate.as_path(), expected_file_name) {
            return Some(format!("{dir}/{}", raw.trim()));
        }
    }

    None
}

fn file_name_matches(path: &Path, expected_file_name: &str) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == expected_file_name.trim())
}

fn normalize_project_key(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_lowercase()
}

fn count_regular_files(path: &Path) -> usize {
    if !path.exists() {
        return 0;
    }
    count_regular_files_recursive(path).unwrap_or(0)
}

fn count_regular_files_recursive(path: &Path) -> Result<usize, String> {
    let mut count = 0usize;
    let entries = fs_read_dir_logged(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            count += count_regular_files_recursive(entry_path.as_path())?;
        } else if entry_path.is_file() {
            count += 1;
        }
    }
    Ok(count)
}

fn import_vtmpack(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<InstanceRecord, String> {
    let ImportSource::ManifestFile(package_path) = &request.source else {
        return Err("Vertex pack import requires a manifest file source.".to_owned());
    };
    progress(import_progress("Reading .vtmpack manifest...", 0, 1));
    let manifest = read_vtmpack_manifest(package_path.as_path())?;
    let extract_steps = count_vtmpack_payload_entries(package_path.as_path())?;
    let total_steps = 3 + extract_steps + manifest.downloadable_content.len();
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: None,
            thumbnail_path: None,
            modloader: default_if_blank(manifest.instance.modloader.as_str(), "Vanilla".to_owned()),
            game_version: default_if_blank(
                manifest.instance.game_version.as_str(),
                "latest".to_owned(),
            ),
            modloader_version: manifest.instance.modloader_version.clone(),
        },
    )
    .map_err(|err| format!("failed to create imported profile: {err}"))?;
    progress(import_progress(
        "Created imported profile. Restoring packaged files...",
        1,
        total_steps,
    ));
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) = populate_vtmpack_instance(
        package_path.as_path(),
        manifest,
        instance_root.as_path(),
        total_steps,
        progress,
    ) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    progress(import_progress(
        "Import complete.",
        total_steps,
        total_steps,
    ));
    let _ = remove_modpack_install_state(instance_root.as_path());

    Ok(instance)
}

fn import_mrpack(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<InstanceRecord, String> {
    let ImportSource::ManifestFile(package_path) = &request.source else {
        return Err("Modrinth pack import requires a manifest file source.".to_owned());
    };
    progress(import_progress("Reading .mrpack manifest...", 0, 1));
    let manifest = read_mrpack_manifest(package_path.as_path())?;
    let dependency_info = resolve_mrpack_dependencies(&manifest.dependencies)?;
    let override_steps = count_mrpack_override_entries(package_path.as_path())?;
    let total_steps = 3 + override_steps + manifest.files.len();
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: non_empty(manifest.summary.as_deref().unwrap_or_default()),
            thumbnail_path: None,
            modloader: dependency_info.modloader.clone(),
            game_version: dependency_info.game_version.clone(),
            modloader_version: dependency_info.modloader_version.clone(),
        },
    )
    .map_err(|err| format!("failed to create imported profile: {err}"))?;
    progress(import_progress(
        "Created imported profile. Restoring overrides...",
        1,
        total_steps,
    ));
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) = populate_mrpack_instance(
        package_path.as_path(),
        manifest.clone(),
        instance_root.as_path(),
        total_steps,
        progress,
    ) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    let base_manifest = match build_mrpack_base_manifest(instance_root.as_path(), &manifest) {
        Ok(manifest) => manifest,
        Err(err) => {
            let _ = delete_instance(store, instance.id.as_str(), installations_root);
            return Err(err);
        }
    };
    if let Err(err) = save_content_manifest(instance_root.as_path(), &base_manifest) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }
    let modpack_state =
        build_mrpack_install_state(package_path.as_path(), &manifest, base_manifest);
    if let Err(err) = save_modpack_install_state(instance_root.as_path(), &modpack_state) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    progress(import_progress(
        "Import complete.",
        total_steps,
        total_steps,
    ));

    Ok(instance)
}

fn import_curseforge_pack(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<InstanceRecord, ImportPackageError> {
    let ImportSource::ManifestFile(package_path) = &request.source else {
        return Err(ImportPackageError::message(
            "CurseForge pack import requires a manifest file source.",
        ));
    };
    let manifest = read_curseforge_pack_manifest(package_path.as_path())
        .map_err(ImportPackageError::message)?;
    let override_steps = count_curseforge_override_entries(
        package_path.as_path(),
        manifest.overrides.as_deref().unwrap_or("overrides"),
    )
    .map_err(ImportPackageError::message)?;
    let file_count = manifest.files.iter().filter(|file| file.required).count();
    let total_steps = 5 + override_steps + (file_count * 2);
    progress(import_progress("Read CurseForge manifest.", 1, total_steps));
    progress(import_progress(
        "Resolving CurseForge pack metadata...",
        2,
        total_steps,
    ));
    let resolved = resolve_curseforge_pack_data(&manifest).map_err(ImportPackageError::message)?;
    let staged_files =
        predownload_curseforge_pack_files(&manifest, &resolved, request, total_steps, progress)?;
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: non_empty(manifest.author.as_str())
                .map(|author| format!("Imported CurseForge pack by {author}.")),
            thumbnail_path: None,
            modloader: resolved.dependency_info.modloader.clone(),
            game_version: resolved.dependency_info.game_version.clone(),
            modloader_version: resolved.dependency_info.modloader_version.clone(),
        },
    )
    .map_err(|err| {
        ImportPackageError::message(format!("failed to create imported profile: {err}"))
    })?;
    progress(import_progress(
        &format!(
            "Downloaded {file_count}/{file_count} mods. Created imported profile. Restoring overrides..."
        ),
        3 + file_count,
        total_steps,
    ));
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) = populate_curseforge_pack_instance(
        package_path.as_path(),
        &manifest,
        &resolved,
        &staged_files,
        instance_root.as_path(),
        total_steps,
        3 + file_count,
        progress,
    ) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    progress(import_progress(
        "Writing managed metadata...",
        total_steps.saturating_sub(1),
        total_steps,
    ));
    let base_manifest = build_curseforge_base_manifest_from_resolved(&manifest, &resolved);
    if let Err(err) = save_content_manifest(instance_root.as_path(), &base_manifest) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(ImportPackageError::message(err));
    }
    let modpack_state = build_curseforge_install_state(&manifest, base_manifest);
    if let Err(err) = save_modpack_install_state(instance_root.as_path(), &modpack_state) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(ImportPackageError::message(err));
    }

    progress(import_progress(
        "Import complete.",
        total_steps,
        total_steps,
    ));
    Ok(instance)
}

pub fn attach_curseforge_modpack_install_state(
    instance_root: &Path,
    project_id: u64,
    file_id: u64,
    pack_name: &str,
    version_name: &str,
) -> Result<(), String> {
    let base_manifest = load_modpack_install_state(instance_root)
        .map(|state| state.base_manifest)
        .unwrap_or_else(|| load_content_manifest(instance_root));
    save_modpack_install_state(
        instance_root,
        &ModpackInstallState {
            format: "curseforge".to_owned(),
            pack_name: default_if_blank(pack_name, "CurseForge Pack".to_owned()),
            version_id: file_id.to_string(),
            version_name: default_if_blank(version_name, file_id.to_string()),
            modrinth_project_id: None,
            curseforge_project_id: Some(project_id),
            source: Some(ManagedContentSource::CurseForge),
            base_manifest,
        },
    )
}

pub(crate) fn format_curseforge_download_url_error(
    project_id: u64,
    file_id: u64,
    err: &curseforge::CurseForgeError,
) -> String {
    let endpoint = format!("/v1/mods/{project_id}/files/{file_id}/download-url");
    match err {
        curseforge::CurseForgeError::HttpStatus { status, body } => {
            let body = body.trim();
            if body.is_empty() {
                format!(
                    "CurseForge download URL lookup failed for project {project_id}, file {file_id} via {endpoint}: HTTP {status} with empty response body"
                )
            } else {
                format!(
                    "CurseForge download URL lookup failed for project {project_id}, file {file_id} via {endpoint}: HTTP {status}: {body}"
                )
            }
        }
        _ => format!(
            "CurseForge download URL lookup failed for project {project_id}, file {file_id} via {endpoint}: {err}"
        ),
    }
}

fn build_mrpack_base_manifest(
    instance_root: &Path,
    manifest: &MrpackManifest,
) -> Result<ContentInstallManifest, String> {
    let modrinth = ModrinthClient::default();
    let mut content_manifest = ContentInstallManifest::default();

    for file in &manifest.files {
        if matches!(
            file.env.as_ref().and_then(|env| env.client.as_deref()),
            Some("unsupported")
        ) {
            continue;
        }
        let content_folder = managed_content_folder_for_relative_path(file.path.as_str());
        let Some(folder_name) = content_folder else {
            continue;
        };
        let absolute_path = join_safe(instance_root, file.path.as_str())?;
        if !absolute_path.exists() || absolute_path.is_dir() {
            continue;
        }
        let Some((project, version)) = resolve_mrpack_manifest_project_version(&modrinth, file)
            .or_else(|| {
                resolve_modrinth_project_version_from_file(&modrinth, absolute_path.as_path())
            })
        else {
            continue;
        };
        let relative_path = absolute_path
            .strip_prefix(instance_root)
            .unwrap_or(absolute_path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        let project_key = format!("modrinth:{}", project.project_id);
        content_manifest.projects.insert(
            project_key.clone(),
            InstalledContentProject {
                project_key,
                name: project.title,
                folder_name: folder_name.to_owned(),
                file_path: relative_path,
                modrinth_project_id: Some(project.project_id),
                curseforge_project_id: None,
                selected_source: Some(ManagedContentSource::Modrinth),
                selected_version_id: Some(version.id),
                selected_version_name: non_empty(version.version_number.as_str()),
                pack_managed: true,
                explicitly_installed: false,
                direct_dependencies: Vec::new(),
            },
        );
    }

    Ok(content_manifest)
}

fn resolve_mrpack_manifest_project_version(
    client: &ModrinthClient,
    file: &MrpackFile,
) -> Option<(modrinth::Project, modrinth::ProjectVersion)> {
    let resolved = file
        .downloads
        .iter()
        .find_map(|url| parse_modrinth_download_source(url.as_str()))?;
    let project = client.get_project(resolved.project_id.as_str()).ok()?;
    let version = client.get_version(resolved.version_id.as_str()).ok()?;
    (version.project_id == resolved.project_id).then_some((project, version))
}

fn build_curseforge_base_manifest_from_resolved(
    manifest: &CurseForgePackManifest,
    resolved: &ResolvedCurseForgePackData,
) -> ContentInstallManifest {
    let mut content_manifest = ContentInstallManifest::default();
    for manifest_file in manifest.files.iter().filter(|file| file.required) {
        let Some(file) = resolved.files.get(&manifest_file.file_id) else {
            continue;
        };
        let project = resolved.projects.get(&manifest_file.project_id);
        let project_key = format!("curseforge:{}", manifest_file.project_id);
        content_manifest.projects.insert(
            project_key.clone(),
            InstalledContentProject {
                project_key,
                name: project
                    .map(|project| project.name.clone())
                    .unwrap_or_else(|| file.display_name.clone()),
                folder_name: "mods".to_owned(),
                file_path: format!("mods/{}", file.file_name),
                modrinth_project_id: None,
                curseforge_project_id: Some(manifest_file.project_id),
                selected_source: Some(ManagedContentSource::CurseForge),
                selected_version_id: Some(manifest_file.file_id.to_string()),
                selected_version_name: non_empty(file.display_name.as_str()),
                pack_managed: true,
                explicitly_installed: false,
                direct_dependencies: Vec::new(),
            },
        );
    }
    content_manifest
}

#[derive(Debug)]
struct ResolvedCurseForgePackData {
    dependency_info: MrpackDependencyInfo,
    files: HashMap<u64, curseforge::File>,
    projects: HashMap<u64, curseforge::Project>,
}

fn resolve_curseforge_pack_data(
    manifest: &CurseForgePackManifest,
) -> Result<ResolvedCurseForgePackData, String> {
    let client = CurseForgeClient::from_env().ok_or_else(|| {
        "CurseForge API key missing. Add one in Settings or set VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY to import this pack."
            .to_owned()
    })?;
    let dependency_info = resolve_curseforge_pack_dependencies(&manifest.minecraft)?;
    let required_files = manifest
        .files
        .iter()
        .filter(|file| file.required)
        .collect::<Vec<_>>();
    let files = client
        .get_files(
            required_files
                .iter()
                .map(|file| file.file_id)
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .map_err(|err| format!("failed to fetch CurseForge pack files: {err}"))?
        .into_iter()
        .map(|file| (file.id, file))
        .collect::<HashMap<_, _>>();
    let projects = client
        .get_mods(
            required_files
                .iter()
                .map(|file| file.project_id)
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .map_err(|err| format!("failed to fetch CurseForge pack projects: {err}"))?
        .into_iter()
        .map(|project| (project.id, project))
        .collect::<HashMap<_, _>>();

    Ok(ResolvedCurseForgePackData {
        dependency_info,
        files,
        projects,
    })
}

#[derive(Clone, Debug)]
struct CurseForgeDownloadPlan {
    requirement: CurseForgeManualDownloadRequirement,
    download_url: String,
    source_label: &'static str,
}

fn predownload_curseforge_pack_files(
    manifest: &CurseForgePackManifest,
    resolved: &ResolvedCurseForgePackData,
    request: &ImportRequest,
    total_steps: usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<HashMap<u64, PathBuf>, ImportPackageError> {
    let mut staged_files = request.manual_curseforge_files.clone();
    let mut download_plans = Vec::new();
    let mut manual_requirements = Vec::new();
    let client = CurseForgeClient::from_env().ok_or_else(|| {
        ImportPackageError::message(
            "CurseForge API key missing. Add one in Settings or set VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY to import this pack.",
        )
    })?;

    for manifest_file in manifest.files.iter().filter(|file| file.required) {
        if staged_files.contains_key(&manifest_file.file_id) {
            continue;
        }
        let file = resolved.files.get(&manifest_file.file_id).ok_or_else(|| {
            ImportPackageError::message(format!(
                "CurseForge file {} for project {} was not found.",
                manifest_file.file_id, manifest_file.project_id
            ))
        })?;
        let project = resolved.projects.get(&manifest_file.project_id);
        let requirement = build_curseforge_manual_download_requirement(
            manifest_file.project_id,
            manifest_file.file_id,
            file,
            project,
        );
        match resolve_curseforge_download_plan(
            &client,
            file,
            project.map(|project| project.name.as_str()),
            manifest_file.project_id,
            manifest_file.file_id,
            resolved.dependency_info.game_version.as_str(),
            resolved.dependency_info.modloader.as_str(),
        )
        .map_err(ImportPackageError::message)?
        {
            Some((download_url, source_label)) => download_plans.push(CurseForgeDownloadPlan {
                requirement,
                download_url,
                source_label,
            }),
            None => manual_requirements.push(requirement),
        }
    }

    if !download_plans.is_empty() {
        progress(import_progress(
            &format!(
                "Preparing {} CurseForge mod downloads...",
                download_plans.len()
            ),
            2,
            total_steps,
        ));
        let download_results = download_curseforge_plans_concurrently(
            download_plans,
            request.max_concurrent_downloads.max(1) as usize,
            total_steps,
            progress,
        )
        .map_err(ImportPackageError::message)?;
        staged_files.extend(download_results.staged_files);
        manual_requirements.extend(download_results.failed_requirements);
    }

    if !manual_requirements.is_empty() {
        manual_requirements.sort_by(|left, right| left.file_name.cmp(&right.file_name));
        manual_requirements.dedup_by(|left, right| left.file_id == right.file_id);
        return Err(ImportPackageError::ManualCurseForgeDownloads {
            requirements: manual_requirements,
            staged_files,
        });
    }

    Ok(staged_files)
}

fn resolve_curseforge_download_plan(
    client: &CurseForgeClient,
    curseforge_file: &curseforge::File,
    curseforge_project_name: Option<&str>,
    project_id: u64,
    file_id: u64,
    game_version: &str,
    modloader: &str,
) -> Result<Option<(String, &'static str)>, String> {
    if let Some(url) = curseforge_file
        .download_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
    {
        return Ok(Some((url.to_owned(), "CurseForge")));
    }
    match client.get_mod_file_download_url(project_id, file_id) {
        Ok(Some(url)) if !url.trim().is_empty() => return Ok(Some((url, "CurseForge"))),
        Ok(_) => {}
        Err(curseforge::CurseForgeError::HttpStatus { status: 403, .. }) => {}
        Err(err) => {
            tracing::warn!(
                target: "vertexlauncher/import",
                curseforge_project_id = project_id,
                curseforge_file_id = file_id,
                error = %format_curseforge_download_url_error(project_id, file_id, &err),
                "CurseForge download URL resolution failed during pack predownload"
            );
        }
    }
    Ok(resolve_modrinth_backup_download_url_for_curseforge_file(
        curseforge_file,
        curseforge_project_name,
        game_version,
        modloader,
    )?
    .map(|url| (url, "Modrinth backup")))
}

struct CurseForgeConcurrentDownloadResult {
    staged_files: HashMap<u64, PathBuf>,
    failed_requirements: Vec<CurseForgeManualDownloadRequirement>,
}

fn download_curseforge_plans_concurrently(
    plans: Vec<CurseForgeDownloadPlan>,
    max_concurrent_downloads: usize,
    total_steps: usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<CurseForgeConcurrentDownloadResult, String> {
    if plans.is_empty() {
        return Ok(CurseForgeConcurrentDownloadResult {
            staged_files: HashMap::new(),
            failed_requirements: Vec::new(),
        });
    }
    let staging_dir = std::env::temp_dir().join(format!(
        "vertexlauncher-cf-download-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    fs_create_dir_all_logged(staging_dir.as_path())
        .map_err(|err| format!("failed to create CurseForge staging directory: {err}"))?;
    let total_downloads = plans.len();
    let queue = Arc::new(Mutex::new(VecDeque::from(plans)));
    let (tx, rx) = mpsc::channel::<(
        CurseForgeManualDownloadRequirement,
        Result<PathBuf, String>,
        &'static str,
    )>();
    let worker_count = max_concurrent_downloads.max(1).min(total_downloads.max(1));
    let mut handles = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let queue = queue.clone();
        let tx = tx.clone();
        let staging_dir = staging_dir.clone();
        handles.push(thread::spawn(move || {
            loop {
                let next = {
                    let mut guard = match queue.lock() {
                        Ok(guard) => guard,
                        Err(_) => return,
                    };
                    guard.pop_front()
                };
                let Some(plan) = next else {
                    return;
                };
                let staged_path = staging_dir.join(format!(
                    "{}-{}",
                    plan.requirement.file_id, plan.requirement.file_name
                ));
                let result = download_file(plan.download_url.as_str(), staged_path.as_path())
                    .map(|_| staged_path);
                let _ = tx.send((plan.requirement, result, plan.source_label));
            }
        }));
    }
    drop(tx);

    let mut completed_downloads = 0usize;
    let mut staged_files = HashMap::new();
    let mut failed_requirements = Vec::new();
    while let Ok((requirement, result, source_label)) = rx.recv() {
        completed_downloads += 1;
        match result {
            Ok(path) => {
                progress(import_progress(
                    &format!(
                        "Downloaded {} via {} ({}/{total_downloads} mods)",
                        requirement.display_name, source_label, completed_downloads
                    ),
                    2 + completed_downloads,
                    total_steps,
                ));
                staged_files.insert(requirement.file_id, path);
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/import",
                    curseforge_project_id = requirement.project_id,
                    curseforge_file_id = requirement.file_id,
                    error = %err,
                    source = source_label,
                    "CurseForge pack predownload failed; requiring manual download"
                );
                progress(import_progress(
                    &format!(
                        "Queued {} for manual download ({}/{total_downloads} mods checked)",
                        requirement.display_name, completed_downloads
                    ),
                    2 + completed_downloads,
                    total_steps,
                ));
                failed_requirements.push(requirement);
            }
        }
    }
    for handle in handles {
        let _ = handle.join();
    }

    Ok(CurseForgeConcurrentDownloadResult {
        staged_files,
        failed_requirements,
    })
}

fn build_mrpack_install_state(
    package_path: &Path,
    manifest: &MrpackManifest,
    base_manifest: ContentInstallManifest,
) -> ModpackInstallState {
    let resolved = resolve_mrpack_modpack_source(package_path);
    ModpackInstallState {
        format: "mrpack".to_owned(),
        pack_name: non_empty(manifest.name.as_str()).unwrap_or_else(|| "Modpack".to_owned()),
        version_id: resolved
            .as_ref()
            .map(|resolved| resolved.version_id.clone())
            .or_else(|| non_empty(manifest.version_id.as_str()))
            .unwrap_or_else(|| "unknown".to_owned()),
        version_name: resolved
            .as_ref()
            .and_then(|resolved| non_empty(resolved.version_name.as_str()))
            .or_else(|| non_empty(manifest.version_id.as_str()))
            .unwrap_or_else(|| "unknown".to_owned()),
        modrinth_project_id: resolved
            .as_ref()
            .map(|resolved| resolved.project_id.clone()),
        curseforge_project_id: None,
        source: resolved.map(|_| ManagedContentSource::Modrinth),
        base_manifest,
    }
}

fn build_curseforge_install_state(
    manifest: &CurseForgePackManifest,
    base_manifest: ContentInstallManifest,
) -> ModpackInstallState {
    ModpackInstallState {
        format: "curseforge".to_owned(),
        pack_name: non_empty(manifest.name.as_str())
            .unwrap_or_else(|| "CurseForge Pack".to_owned()),
        version_id: non_empty(manifest.version.as_str()).unwrap_or_else(|| "unknown".to_owned()),
        version_name: non_empty(manifest.version.as_str()).unwrap_or_else(|| "unknown".to_owned()),
        modrinth_project_id: None,
        curseforge_project_id: None,
        source: Some(ManagedContentSource::CurseForge),
        base_manifest,
    }
}

#[derive(Clone, Debug)]
struct ResolvedMrpackSource {
    project_id: String,
    version_id: String,
    version_name: String,
}

#[derive(Debug, Clone)]
struct ResolvedModrinthDownloadSource {
    project_id: String,
    version_id: String,
}

fn resolve_mrpack_modpack_source(package_path: &Path) -> Option<ResolvedMrpackSource> {
    let (sha1, sha512) = modrinth::hash_file_sha1_and_sha512_hex(package_path).ok()?;
    let client = ModrinthClient::default();
    let version = client
        .get_version_from_hash(sha512.as_str(), "sha512")
        .ok()
        .flatten()
        .or_else(|| {
            client
                .get_version_from_hash(sha1.as_str(), "sha1")
                .ok()
                .flatten()
        })?;
    Some(ResolvedMrpackSource {
        project_id: version.project_id,
        version_id: version.id,
        version_name: version.version_number,
    })
}

fn resolve_modrinth_project_version_from_file(
    client: &ModrinthClient,
    path: &Path,
) -> Option<(modrinth::Project, modrinth::ProjectVersion)> {
    let (sha1, sha512) = modrinth::hash_file_sha1_and_sha512_hex(path).ok()?;
    let version = client
        .get_version_from_hash(sha512.as_str(), "sha512")
        .ok()
        .flatten()
        .or_else(|| {
            client
                .get_version_from_hash(sha1.as_str(), "sha1")
                .ok()
                .flatten()
        })?;
    let project = client.get_project(version.project_id.as_str()).ok()?;
    Some((project, version))
}

fn parse_modrinth_download_source(url: &str) -> Option<ResolvedModrinthDownloadSource> {
    let path = url.split(['?', '#']).next()?.trim_matches('/');
    let mut segments = path.split('/');
    while let Some(segment) = segments.next() {
        if segment != "data" {
            continue;
        }
        let project_id = non_empty(segments.next()?)?;
        if segments.next()? != "versions" {
            return None;
        }
        let version_id = non_empty(segments.next()?)?;
        return Some(ResolvedModrinthDownloadSource {
            project_id,
            version_id,
        });
    }
    None
}

fn managed_content_folder_for_relative_path(relative_path: &str) -> Option<&'static str> {
    let normalized = relative_path.replace('\\', "/");
    let head = normalized.split('/').next()?.to_ascii_lowercase();
    match head.as_str() {
        "mods" => Some("mods"),
        "resourcepacks" => Some("resourcepacks"),
        "shaderpacks" => Some("shaderpacks"),
        "datapacks" => Some("datapacks"),
        _ => None,
    }
}

#[allow(dead_code)]
fn pack_managed_path_keys(
    live_manifest: &ContentInstallManifest,
    base_manifest: &ContentInstallManifest,
) -> std::collections::HashSet<String> {
    live_manifest
        .projects
        .values()
        .filter(|project| project.pack_managed)
        .map(|project| managed_content::normalize_content_path_key(project.file_path.as_str()))
        .chain(
            base_manifest.projects.values().map(|project| {
                managed_content::normalize_content_path_key(project.file_path.as_str())
            }),
        )
        .collect()
}

#[allow(dead_code)]
fn preserve_non_pack_managed_content(
    existing_root: &Path,
    temp_root: &Path,
    pack_managed_paths: &std::collections::HashSet<String>,
) -> Result<(), String> {
    for folder in ["mods", "resourcepacks", "shaderpacks", "datapacks"] {
        let current_dir = existing_root.join(folder);
        let Ok(entries) = fs::read_dir(current_dir.as_path()) else {
            continue;
        };
        for entry in entries {
            let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
            let source_path = entry.path();
            let relative_path = source_path
                .strip_prefix(existing_root)
                .unwrap_or(source_path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let relative_key = managed_content::normalize_content_path_key(relative_path.as_str());
            if pack_managed_paths.contains(relative_key.as_str()) {
                continue;
            }
            let destination = temp_root.join(relative_path.as_str());
            copy_path_recursive(source_path.as_path(), destination.as_path())?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn preserve_instance_user_state(existing_root: &Path, temp_root: &Path) -> Result<(), String> {
    let saves_root = existing_root.join("saves");
    if saves_root.exists() {
        copy_path_recursive(saves_root.as_path(), temp_root.join("saves").as_path())?;
    }
    let servers_dat = existing_root.join("servers.dat");
    if servers_dat.exists() {
        copy_path_recursive(
            servers_dat.as_path(),
            temp_root.join("servers.dat").as_path(),
        )?;
    }
    Ok(())
}

#[allow(dead_code)]
fn copy_path_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    if source.is_dir() {
        fs_create_dir_all_logged(destination)
            .map_err(|err| format!("failed to create {}: {err}", destination.display()))?;
        let entries = fs_read_dir_logged(source)
            .map_err(|err| format!("failed to read {}: {err}", source.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
            copy_path_recursive(
                entry.path().as_path(),
                destination.join(entry.file_name()).as_path(),
            )?;
        }
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs_create_dir_all_logged(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs_copy_logged(source, destination).map_err(|err| {
        format!(
            "failed to copy {} to {}: {err}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

#[allow(dead_code)]
fn swap_instance_root(existing_root: &Path, temp_root: &Path) -> Result<(), String> {
    let backup_root = existing_root.with_extension("modpack-update-backup");
    if backup_root.exists() {
        fs_remove_dir_all_logged(backup_root.as_path()).map_err(|err| {
            format!(
                "failed to remove stale backup {}: {err}",
                backup_root.display()
            )
        })?;
    }
    fs_rename_logged(existing_root, backup_root.as_path()).map_err(|err| {
        format!(
            "failed to stage old instance root {}: {err}",
            existing_root.display()
        )
    })?;
    if let Err(err) = fs_rename_logged(temp_root, existing_root) {
        let _ = fs_rename_logged(backup_root.as_path(), existing_root);
        return Err(format!(
            "failed to activate updated instance root {}: {err}",
            existing_root.display()
        ));
    }
    fs_remove_dir_all_logged(backup_root.as_path()).map_err(|err| {
        format!(
            "failed to remove update backup {}: {err}",
            backup_root.display()
        )
    })?;
    Ok(())
}

#[allow(dead_code)]
fn unique_temp_instance_root(installations_root: &Path, instance_id: &str) -> PathBuf {
    for attempt in 0..1024_u32 {
        let candidate =
            installations_root.join(format!(".vertex-modpack-update-{instance_id}-{attempt}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    installations_root.join(format!(".vertex-modpack-update-{instance_id}-overflow"))
}

fn populate_vtmpack_instance(
    package_path: &Path,
    manifest: VtmpackManifest,
    instance_root: &Path,
    total_steps: usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    let mut completed_steps = 1usize;
    extract_vtmpack_payload(
        package_path,
        instance_root,
        total_steps,
        &mut completed_steps,
        progress,
    )?;

    for downloadable in &manifest.downloadable_content {
        if downloadable.file_path.trim().is_empty() {
            continue;
        }
        let destination = join_safe(instance_root, downloadable.file_path.as_str())?;
        if let Some(parent) = destination.parent() {
            fs_create_dir_all_logged(parent).map_err(|err| {
                format!(
                    "failed to create import directory {}: {err}",
                    parent.display()
                )
            })?;
        }
        completed_steps += 1;
        progress(import_progress(
            &format!("Downloading {}", downloadable.name),
            completed_steps,
            total_steps,
        ));
        download_vtmpack_entry(downloadable, destination.as_path())?;
    }

    Ok(())
}

fn extract_vtmpack_payload(
    package_path: &Path,
    instance_root: &Path,
    total_steps: usize,
    completed_steps: &mut usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    let file = fs_file_open_logged(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let decoder = xz2::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive
        .entries()
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?
    {
        let mut entry = entry.map_err(|err| {
            format!(
                "failed to read archive entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_path = entry
            .path()
            .map_err(|err| format!("failed to decode archive path: {err}"))?
            .to_path_buf();
        let entry_string = entry_path.to_string_lossy().replace('\\', "/");

        if entry_string == "manifest.toml" {
            continue;
        }
        *completed_steps += 1;
        if entry_string == format!("metadata/{CONTENT_MANIFEST_FILE_NAME}") {
            let destination = instance_root.join(CONTENT_MANIFEST_FILE_NAME);
            progress(import_progress(
                "Restoring managed metadata...",
                *completed_steps,
                total_steps,
            ));
            if let Some(parent) = destination.parent() {
                fs_create_dir_all_logged(parent).map_err(|err| {
                    format!(
                        "failed to create metadata directory {}: {err}",
                        parent.display()
                    )
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                format!(
                    "failed to restore managed metadata into {}: {err}",
                    destination.display()
                )
            })?;
            continue;
        }
        if let Some(relative) = entry_string.strip_prefix("bundled_mods/") {
            let destination = join_safe(&instance_root.join("mods"), relative)?;
            progress(import_progress(
                &format!("Restoring bundled mod {}", relative),
                *completed_steps,
                total_steps,
            ));
            if let Some(parent) = destination.parent() {
                fs_create_dir_all_logged(parent).map_err(|err| {
                    format!(
                        "failed to create bundled mod directory {}: {err}",
                        parent.display()
                    )
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                format!(
                    "failed to import bundled mod {}: {err}",
                    destination.display()
                )
            })?;
            continue;
        }
        if let Some(relative) = entry_string.strip_prefix("configs/") {
            let destination = join_safe(&instance_root.join("config"), relative)?;
            progress(import_progress(
                &format!("Restoring config {}", relative),
                *completed_steps,
                total_steps,
            ));
            if let Some(parent) = destination.parent() {
                fs_create_dir_all_logged(parent).map_err(|err| {
                    format!(
                        "failed to create config directory {}: {err}",
                        parent.display()
                    )
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                format!("failed to import config {}: {err}", destination.display())
            })?;
            continue;
        }
        if let Some(relative) = entry_string.strip_prefix("root_entries/") {
            let destination = join_safe(instance_root, relative)?;
            progress(import_progress(
                &format!("Restoring {}", relative),
                *completed_steps,
                total_steps,
            ));
            if let Some(parent) = destination.parent() {
                fs_create_dir_all_logged(parent).map_err(|err| {
                    format!(
                        "failed to create imported root entry directory {}: {err}",
                        parent.display()
                    )
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                format!(
                    "failed to import extra root entry {}: {err}",
                    destination.display()
                )
            })?;
        }
    }
    Ok(())
}

fn download_vtmpack_entry(
    entry: &VtmpackDownloadableEntry,
    destination: &Path,
) -> Result<(), String> {
    match normalize_source_name(entry.selected_source.as_deref()) {
        Some(ManagedSource::Modrinth) => {
            let version_id = entry
                .selected_version_id
                .as_deref()
                .ok_or_else(|| format!("missing Modrinth version id for {}", entry.name))?;
            let version = ModrinthClient::default()
                .get_version(version_id)
                .map_err(|err| format!("failed to fetch Modrinth version {version_id}: {err}"))?;
            let file = version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())
                .ok_or_else(|| {
                    format!("no downloadable file found for Modrinth version {version_id}")
                })?;
            download_file(file.url.as_str(), destination)
        }
        Some(ManagedSource::CurseForge) => {
            let project_id = entry
                .curseforge_project_id
                .ok_or_else(|| format!("missing CurseForge project id for {}", entry.name))?;
            let file_id = entry
                .selected_version_id
                .as_deref()
                .ok_or_else(|| format!("missing CurseForge file id for {}", entry.name))?
                .parse::<u64>()
                .map_err(|err| format!("invalid CurseForge file id for {}: {err}", entry.name))?;
            let client = CurseForgeClient::from_env().ok_or_else(|| {
                "CurseForge API key missing. Add one in Settings or set VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY to import this pack."
                    .to_owned()
            })?;
            let file = find_curseforge_file(&client, project_id, file_id)?;
            let download_url = file.download_url.ok_or_else(|| {
                format!("CurseForge file {file_id} for project {project_id} has no download URL")
            })?;
            download_file(download_url.as_str(), destination)
        }
        None => {
            if let Some(version_id) = entry.selected_version_id.as_deref() {
                let version = ModrinthClient::default()
                    .get_version(version_id)
                    .map_err(|err| {
                        format!("failed to fetch Modrinth fallback version {version_id}: {err}")
                    })?;
                let file = version
                    .files
                    .iter()
                    .find(|file| file.primary)
                    .or_else(|| version.files.first())
                    .ok_or_else(|| {
                        format!("no downloadable file found for Modrinth version {version_id}")
                    })?;
                return download_file(file.url.as_str(), destination);
            }
            Err(format!(
                "download source for {} could not be determined from the pack metadata",
                entry.name
            ))
        }
    }
}

fn find_curseforge_file(
    client: &CurseForgeClient,
    project_id: u64,
    file_id: u64,
) -> Result<curseforge::File, String> {
    client
        .get_files(&[file_id])
        .map_err(|err| format!("failed to fetch CurseForge file {file_id}: {err}"))?
        .into_iter()
        .next()
        .ok_or_else(|| format!("CurseForge file {file_id} was not found for project {project_id}"))
}

fn populate_mrpack_instance(
    package_path: &Path,
    manifest: MrpackManifest,
    instance_root: &Path,
    total_steps: usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    let mut completed_steps = 1usize;
    extract_mrpack_overrides(
        package_path,
        instance_root,
        total_steps,
        &mut completed_steps,
        progress,
    )?;
    for file in manifest.files {
        if matches!(
            file.env.as_ref().and_then(|env| env.client.as_deref()),
            Some("unsupported")
        ) {
            continue;
        }
        let destination = join_safe(instance_root, file.path.as_str())?;
        if let Some(parent) = destination.parent() {
            fs_create_dir_all_logged(parent).map_err(|err| {
                format!(
                    "failed to create import directory {}: {err}",
                    parent.display()
                )
            })?;
        }
        let download_url = file
            .downloads
            .first()
            .cloned()
            .ok_or_else(|| format!("Modrinth pack entry {} has no download URL", file.path))?;
        completed_steps += 1;
        progress(import_progress(
            &format!("Downloading {}", file.path),
            completed_steps,
            total_steps,
        ));
        download_file(download_url.as_str(), destination.as_path())?;
    }
    Ok(())
}

fn populate_curseforge_pack_instance(
    package_path: &Path,
    manifest: &CurseForgePackManifest,
    resolved: &ResolvedCurseForgePackData,
    manual_curseforge_files: &HashMap<u64, PathBuf>,
    instance_root: &Path,
    total_steps: usize,
    starting_completed_steps: usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), ImportPackageError> {
    let mut completed_steps = starting_completed_steps;
    let total_mods = manifest.files.iter().filter(|file| file.required).count();
    let mut applied_mods = 0usize;
    let overrides_root = manifest.overrides.as_deref().unwrap_or("overrides");
    extract_curseforge_overrides(
        package_path,
        instance_root,
        overrides_root,
        total_steps,
        &mut completed_steps,
        progress,
    )
    .map_err(ImportPackageError::message)?;

    for manifest_file in manifest.files.iter().filter(|file| file.required) {
        let file = resolved.files.get(&manifest_file.file_id).ok_or_else(|| {
            ImportPackageError::message(format!(
                "CurseForge file {} for project {} was not found.",
                manifest_file.file_id, manifest_file.project_id
            ))
        })?;
        let destination = instance_root.join("mods").join(file.file_name.as_str());
        if let Some(parent) = destination.parent() {
            fs_create_dir_all_logged(parent).map_err(|err| {
                ImportPackageError::message(format!("failed to create {}: {err}", parent.display()))
            })?;
        }
        let source_path = manual_curseforge_files
            .get(&manifest_file.file_id)
            .ok_or_else(|| {
                ImportPackageError::message(format!(
                    "CurseForge file {} was not predownloaded before installation.",
                    manifest_file.file_id
                ))
            })?;
        completed_steps += 1;
        applied_mods += 1;
        progress(import_progress(
            &format!(
                "Applying staged file for {} ({applied_mods}/{total_mods} mods)",
                file.display_name
            ),
            completed_steps,
            total_steps,
        ));
        fs_copy_logged(source_path, destination.as_path()).map_err(|err| {
            ImportPackageError::message(format!(
                "failed to copy predownloaded file {} into {}: {err}",
                source_path.display(),
                destination.display()
            ))
        })?;
    }

    Ok(())
}

fn extract_mrpack_overrides(
    package_path: &Path,
    instance_root: &Path,
    total_steps: usize,
    completed_steps: &mut usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    let file = fs_file_open_logged(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|err| {
            format!(
                "failed to read zip entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_name = entry.name().replace('\\', "/");
        let Some(relative) = entry_name
            .strip_prefix("overrides/")
            .or_else(|| entry_name.strip_prefix("client-overrides/"))
        else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }
        let destination = join_safe(instance_root, relative)?;
        *completed_steps += 1;
        progress(import_progress(
            &format!("Restoring override {}", relative),
            *completed_steps,
            total_steps,
        ));
        if entry.is_dir() {
            fs_create_dir_all_logged(destination.as_path()).map_err(|err| {
                format!(
                    "failed to create override directory {}: {err}",
                    destination.display()
                )
            })?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs_create_dir_all_logged(parent).map_err(|err| {
                format!(
                    "failed to create override parent {}: {err}",
                    parent.display()
                )
            })?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|err| {
            format!(
                "failed to read override {} from {}: {err}",
                entry_name,
                package_path.display()
            )
        })?;
        fs_write_logged(destination.as_path(), bytes)
            .map_err(|err| format!("failed to write override {}: {err}", destination.display()))?;
    }

    Ok(())
}

fn extract_curseforge_overrides(
    package_path: &Path,
    instance_root: &Path,
    overrides_root: &str,
    total_steps: usize,
    completed_steps: &mut usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    let file = fs_file_open_logged(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?;
    let normalized_root = format!("{}/", overrides_root.trim().trim_matches('/'));

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|err| {
            format!(
                "failed to read zip entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_name = entry.name().replace('\\', "/");
        let Some(relative) = entry_name.strip_prefix(normalized_root.as_str()) else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }
        let destination = join_safe(instance_root, relative)?;
        *completed_steps += 1;
        progress(import_progress(
            &format!("Restoring override {}", relative),
            *completed_steps,
            total_steps,
        ));
        if entry.is_dir() {
            fs_create_dir_all_logged(destination.as_path()).map_err(|err| {
                format!(
                    "failed to create override directory {}: {err}",
                    destination.display()
                )
            })?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs_create_dir_all_logged(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|err| {
            format!(
                "failed to read override {} from {}: {err}",
                entry_name,
                package_path.display()
            )
        })?;
        fs_write_logged(destination.as_path(), bytes)
            .map_err(|err| format!("failed to write override {}: {err}", destination.display()))?;
    }

    Ok(())
}

fn count_vtmpack_payload_entries(package_path: &Path) -> Result<usize, String> {
    let file = fs_file_open_logged(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let decoder = xz2::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut count = 0usize;
    for entry in archive
        .entries()
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?
    {
        let entry = entry.map_err(|err| {
            format!(
                "failed to read archive entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_path = entry
            .path()
            .map_err(|err| format!("failed to decode archive path: {err}"))?
            .to_path_buf();
        if entry_path.to_string_lossy().replace('\\', "/") != "manifest.toml" {
            count += 1;
        }
    }
    Ok(count)
}

fn count_mrpack_override_entries(package_path: &Path) -> Result<usize, String> {
    let file = fs_file_open_logged(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?;
    let mut count = 0usize;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|err| {
            format!(
                "failed to read zip entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_name = entry.name().replace('\\', "/");
        if entry_name
            .strip_prefix("overrides/")
            .or_else(|| entry_name.strip_prefix("client-overrides/"))
            .is_some_and(|relative| !relative.is_empty())
        {
            count += 1;
        }
    }
    Ok(count)
}

fn count_curseforge_override_entries(
    package_path: &Path,
    overrides_root: &str,
) -> Result<usize, String> {
    let file = fs_file_open_logged(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?;
    let normalized_root = format!("{}/", overrides_root.trim().trim_matches('/'));
    let mut count = 0usize;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|err| {
            format!(
                "failed to read zip entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_name = entry.name().replace('\\', "/");
        if entry_name.starts_with(normalized_root.as_str()) && !entry.is_dir() {
            count += 1;
        }
    }
    Ok(count)
}

fn import_progress(message: &str, completed_steps: usize, total_steps: usize) -> ImportProgress {
    ImportProgress {
        message: message.to_owned(),
        completed_steps,
        total_steps,
    }
}

fn read_mrpack_manifest(path: &Path) -> Result<MrpackManifest, String> {
    let file = fs_file_open_logged(path)
        .map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut manifest = archive
        .by_name("modrinth.index.json")
        .map_err(|err| format!("missing modrinth.index.json in {}: {err}", path.display()))?;
    let mut raw = String::new();
    manifest
        .read_to_string(&mut raw)
        .map_err(|err| format!("failed to read modrinth.index.json: {err}"))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse modrinth.index.json: {err}"))
}

fn read_curseforge_pack_manifest(path: &Path) -> Result<CurseForgePackManifest, String> {
    let file = fs_file_open_logged(path)
        .map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut manifest = archive
        .by_name("manifest.json")
        .map_err(|err| format!("missing manifest.json in {}: {err}", path.display()))?;
    let mut raw = String::new();
    manifest
        .read_to_string(&mut raw)
        .map_err(|err| format!("failed to read manifest.json: {err}"))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse manifest.json: {err}"))
}

fn resolve_mrpack_dependencies(
    dependencies: &HashMap<String, String>,
) -> Result<MrpackDependencyInfo, String> {
    let raw_game_version = dependencies
        .get("minecraft")
        .ok_or_else(|| "Modrinth pack is missing the required minecraft dependency.".to_owned())?;
    let game_version = normalize_minecraft_game_version(raw_game_version).ok_or_else(|| {
        format!(
            "Modrinth pack declared an invalid Minecraft version: {}",
            raw_game_version.trim()
        )
    })?;

    let loader_candidates = [
        ("neoforge", "NeoForge"),
        ("forge", "Forge"),
        ("fabric-loader", "Fabric"),
        ("quilt-loader", "Quilt"),
    ];
    for (key, label) in loader_candidates {
        if let Some(version) = dependencies.get(key) {
            return Ok(MrpackDependencyInfo {
                game_version,
                modloader: label.to_owned(),
                modloader_version: version.clone(),
            });
        }
    }

    Ok(MrpackDependencyInfo {
        game_version,
        modloader: "Vanilla".to_owned(),
        modloader_version: String::new(),
    })
}

fn resolve_curseforge_pack_dependencies(
    minecraft: &CurseForgePackMinecraft,
) -> Result<MrpackDependencyInfo, String> {
    let game_version =
        normalize_minecraft_game_version(minecraft.version.as_str()).ok_or_else(|| {
            format!(
                "CurseForge pack declared an invalid Minecraft version: {}",
                minecraft.version.trim()
            )
        })?;

    let loader = minecraft
        .mod_loaders
        .iter()
        .find(|loader| loader.primary)
        .or_else(|| minecraft.mod_loaders.first());
    let Some(loader) = loader else {
        return Ok(MrpackDependencyInfo {
            game_version,
            modloader: "Vanilla".to_owned(),
            modloader_version: String::new(),
        });
    };

    let id = loader.id.trim();
    let (modloader, modloader_version) = if let Some(version) = id.strip_prefix("forge-") {
        ("Forge".to_owned(), version.to_owned())
    } else if let Some(version) = id.strip_prefix("fabric-") {
        ("Fabric".to_owned(), version.to_owned())
    } else if let Some(version) = id.strip_prefix("quilt-") {
        ("Quilt".to_owned(), version.to_owned())
    } else if let Some(version) = id.strip_prefix("neoforge-") {
        ("NeoForge".to_owned(), version.to_owned())
    } else {
        (id.to_owned(), String::new())
    };

    Ok(MrpackDependencyInfo {
        game_version,
        modloader,
        modloader_version,
    })
}

fn normalize_source_name(source: Option<&str>) -> Option<ManagedSource> {
    match source?.trim().to_ascii_lowercase().as_str() {
        "modrinth" => Some(ManagedSource::Modrinth),
        "curseforge" => Some(ManagedSource::CurseForge),
        _ => None,
    }
}

fn join_safe(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    let mut clean = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "unsafe path in import package: {}",
                    relative.display()
                ));
            }
        }
    }
    Ok(root.join(clean))
}

fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    throttle_download_url(url);
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| format!("download request failed for {url}: {err}"))?;
    let mut reader = response.body_mut().as_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read download body from {url}: {err}"))?;
    fs_write_logged(destination, bytes)
        .map_err(|err| format!("failed to write {}: {err}", destination.display()))
}

fn resolve_modrinth_backup_download_url_for_curseforge_file(
    curseforge_file: &curseforge::File,
    curseforge_project_name: Option<&str>,
    game_version: &str,
    modloader: &str,
) -> Result<Option<String>, String> {
    let modrinth = ModrinthClient::default();
    if let Some(url) = resolve_modrinth_hash_backup_download_url_for_curseforge_file(
        &modrinth,
        curseforge_file,
        game_version,
        modloader,
    )? {
        return Ok(Some(url));
    }

    let queries = modrinth_fallback_queries(curseforge_file, curseforge_project_name);
    if queries.is_empty() {
        return Ok(None);
    }

    let loader_slug = modrinth_loader_slug(modloader);
    let mut loaders = Vec::new();
    if let Some(loader) = loader_slug {
        loaders.push(loader.to_owned());
    }
    let normalized_game_version = normalize_minecraft_game_version(game_version);
    let mut game_versions = Vec::new();
    if let Some(version) = normalized_game_version.as_deref() {
        game_versions.push(version.to_owned());
    }

    for query in queries {
        let projects = match modrinth.search_projects_with_filters(
            query.as_str(),
            8,
            0,
            Some("mod"),
            normalized_game_version.as_deref(),
            loader_slug,
            None,
        ) {
            Ok(projects) => projects,
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/import",
                    query = %query,
                    error = %err,
                    "Modrinth fallback search failed"
                );
                continue;
            }
        };

        for project in projects.into_iter().take(5) {
            let versions = match modrinth.list_project_versions(
                project.project_id.as_str(),
                loaders.as_slice(),
                game_versions.as_slice(),
            ) {
                Ok(versions) => versions,
                Err(_) => continue,
            };
            for version in versions {
                let Some(file) = select_modrinth_backup_file(
                    &version,
                    curseforge_file,
                    game_version,
                    modloader,
                    true,
                ) else {
                    continue;
                };
                tracing::warn!(
                    target: "vertexlauncher/import",
                    curseforge_file_id = curseforge_file.id,
                    modrinth_project_id = %project.project_id,
                    modrinth_version_id = %version.id,
                    "Using Modrinth fallback download for CurseForge file"
                );
                return Ok(Some(file.url.clone()));
            }
        }
    }
    Ok(None)
}

#[derive(Clone, Debug)]
pub(crate) struct CurseForgeManualDownloadRequirement {
    pub project_id: u64,
    pub file_id: u64,
    pub project_name: String,
    pub file_name: String,
    pub display_name: String,
    pub download_page_url: String,
}

pub(crate) fn prepare_curseforge_manual_downloads(
    request: &ImportRequest,
) -> Result<Option<Vec<CurseForgeManualDownloadRequirement>>, String> {
    let ImportSource::ManifestFile(package_path) = &request.source else {
        return Ok(None);
    };
    if inspect_package(package_path.as_path())
        .map(|preview| preview.kind)
        .ok()
        != Some(ImportPreviewKind::Manifest(
            ImportPackageKind::CurseForgePack,
        ))
    {
        return Ok(None);
    }
    let manifest = read_curseforge_pack_manifest(package_path.as_path())?;
    let resolved = resolve_curseforge_pack_data(&manifest)?;
    let mut blocked = Vec::new();
    for manifest_file in manifest.files.iter().filter(|file| file.required) {
        let Some(file) = resolved.files.get(&manifest_file.file_id) else {
            continue;
        };
        if curseforge_file_has_api_download(file) {
            continue;
        }
        blocked.push(build_curseforge_manual_download_requirement(
            manifest_file.project_id,
            manifest_file.file_id,
            file,
            resolved.projects.get(&manifest_file.project_id),
        ));
    }
    Ok((!blocked.is_empty()).then_some(blocked))
}

pub(crate) fn prepare_curseforge_manual_download_for_file(
    project_id: u64,
    file_id: u64,
) -> Result<Option<CurseForgeManualDownloadRequirement>, String> {
    let client = CurseForgeClient::from_env().ok_or_else(|| {
        "CurseForge API key missing. Add one in Settings or set VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY to import this pack."
            .to_owned()
    })?;
    let file = find_curseforge_file(&client, project_id, file_id)?;
    if curseforge_file_has_api_download(&file) {
        return Ok(None);
    }
    let project = client
        .get_mods(&[project_id])
        .map_err(|err| format!("failed to fetch CurseForge project {project_id}: {err}"))?
        .into_iter()
        .next();
    Ok(Some(build_curseforge_manual_download_requirement(
        project_id,
        file_id,
        &file,
        project.as_ref(),
    )))
}

fn curseforge_file_has_api_download(file: &curseforge::File) -> bool {
    file.download_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty())
}

fn build_curseforge_manual_download_requirement(
    project_id: u64,
    file_id: u64,
    file: &curseforge::File,
    project: Option<&curseforge::Project>,
) -> CurseForgeManualDownloadRequirement {
    let project_name = project
        .map(|project| project.name.clone())
        .unwrap_or_else(|| file.display_name.clone());
    let download_page_url = project
        .and_then(|project| project.website_url.clone())
        .map(|base| format!("{}/files/{}", base.trim_end_matches('/'), file_id))
        .unwrap_or_else(|| {
            format!("https://www.curseforge.com/minecraft/mc-mods/{project_id}/files/{file_id}")
        });
    CurseForgeManualDownloadRequirement {
        project_id,
        file_id,
        project_name,
        file_name: file.file_name.clone(),
        display_name: file.display_name.clone(),
        download_page_url,
    }
}

fn resolve_modrinth_hash_backup_download_url_for_curseforge_file(
    modrinth: &ModrinthClient,
    curseforge_file: &curseforge::File,
    game_version: &str,
    modloader: &str,
) -> Result<Option<String>, String> {
    let loader_slug = modrinth_loader_slug(modloader);
    let normalized_game_version = normalize_minecraft_game_version(game_version);

    let mut hash_candidates = Vec::new();
    if let Some(sha512) = curseforge_file.sha512_hash() {
        hash_candidates.push(("sha512", sha512));
    }
    if let Some(sha1) = curseforge_file.sha1_hash() {
        hash_candidates.push(("sha1", sha1));
    }
    if hash_candidates.is_empty() {
        return Ok(None);
    }

    for (algorithm, hash) in hash_candidates {
        let version = match modrinth.get_version_from_hash(hash, algorithm) {
            Ok(Some(version)) => version,
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/import",
                    curseforge_file_id = curseforge_file.id,
                    algorithm,
                    error = %err,
                    "Modrinth hash lookup failed during fallback"
                );
                continue;
            }
        };
        if let Some(game_version) = normalized_game_version.as_deref()
            && !version.game_versions.is_empty()
            && !version
                .game_versions
                .iter()
                .any(|value| value.eq_ignore_ascii_case(game_version))
        {
            continue;
        }
        if let Some(loader_slug) = loader_slug
            && !version.loaders.is_empty()
            && !version
                .loaders
                .iter()
                .any(|value| value.eq_ignore_ascii_case(loader_slug))
        {
            continue;
        }

        let Some(file) =
            select_modrinth_backup_file(&version, curseforge_file, game_version, modloader, false)
        else {
            continue;
        };
        tracing::warn!(
            target: "vertexlauncher/import",
            curseforge_file_id = curseforge_file.id,
            modrinth_version_id = %version.id,
            algorithm,
            "Using exact Modrinth hash fallback for CurseForge file"
        );
        return Ok(Some(file.url.clone()));
    }
    Ok(None)
}

fn modrinth_fallback_queries(
    file: &curseforge::File,
    curseforge_project_name: Option<&str>,
) -> Vec<String> {
    let mut queries = Vec::new();
    let raw_candidates = [
        curseforge_project_name.unwrap_or_default(),
        file.display_name.as_str(),
        file.file_name.as_str(),
        file.file_name
            .strip_suffix(".jar")
            .unwrap_or(file.file_name.as_str()),
    ];
    for candidate in raw_candidates {
        let query = candidate
            .replace(['[', ']', '(', ')', '{', '}'], " ")
            .replace(['_', '-'], " ")
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ");
        if !query.is_empty() && !queries.iter().any(|entry| entry == &query) {
            queries.push(query);
        }
    }
    queries
}

fn select_modrinth_backup_file<'a>(
    version: &'a modrinth::ProjectVersion,
    curseforge_file: &curseforge::File,
    game_version: &str,
    modloader: &str,
    require_exact_filename: bool,
) -> Option<&'a modrinth::ProjectVersionFile> {
    let expected_name = normalized_name(curseforge_file.file_name.as_str());
    if let Some(file) = version
        .files
        .iter()
        .find(|candidate| normalized_name(candidate.filename.as_str()) == expected_name)
    {
        return Some(file);
    }
    if require_exact_filename || version.files.len() != 1 {
        return None;
    }
    let file = version.files.first()?;
    modrinth_backup_filename_looks_compatible(file.filename.as_str(), game_version, modloader)
        .then_some(file)
}

fn modrinth_backup_filename_looks_compatible(
    filename: &str,
    game_version: &str,
    modloader: &str,
) -> bool {
    let desired_loader = modloader_loader_family(modloader);
    let candidate_loader = modloader_loader_family(filename);
    if let (Some(desired_loader), Some(candidate_loader)) = (desired_loader, candidate_loader)
        && desired_loader != candidate_loader
    {
        return false;
    }
    if let Some(candidate_game_version) = find_minecraft_version_in_text(filename)
        && let Some(desired_game_version) = normalize_minecraft_game_version(game_version)
        && candidate_game_version != desired_game_version
    {
        return false;
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModloaderFamily {
    Fabric,
    Forge,
    NeoForge,
    Quilt,
}

fn modloader_loader_family(value: &str) -> Option<ModloaderFamily> {
    let lower = value.trim().to_ascii_lowercase();
    if lower.contains("neoforge") || lower.contains("-neo-") {
        Some(ModloaderFamily::NeoForge)
    } else if lower.contains("fabric") {
        Some(ModloaderFamily::Fabric)
    } else if lower.contains("quilt") {
        Some(ModloaderFamily::Quilt)
    } else if lower.contains("forge") {
        Some(ModloaderFamily::Forge)
    } else {
        None
    }
}

fn modrinth_loader_slug(loader: &str) -> Option<&'static str> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "fabric" => Some("fabric"),
        "forge" => Some("forge"),
        "quilt" => Some("quilt"),
        "neoforge" => Some("neoforge"),
        _ => None,
    }
}

fn normalized_name(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn throttle_download_url(url: &str) {
    let Some(spacing) = download_spacing_for_url(url) else {
        return;
    };
    let lock = download_throttle_store(url);
    let Ok(mut next_allowed) = lock.lock() else {
        return;
    };
    let now = Instant::now();
    if *next_allowed > now {
        std::thread::sleep(next_allowed.saturating_duration_since(now));
    }
    *next_allowed = Instant::now() + spacing;
}

fn download_spacing_for_url(url: &str) -> Option<Duration> {
    let host = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.contains("modrinth.com") {
        Some(MODRINTH_DOWNLOAD_MIN_SPACING)
    } else if host.contains("curseforge.com") || host.contains("forgecdn.net") {
        Some(CURSEFORGE_DOWNLOAD_MIN_SPACING)
    } else {
        None
    }
}

fn download_throttle_store(url: &str) -> &'static Mutex<Instant> {
    static MODRINTH: OnceLock<Mutex<Instant>> = OnceLock::new();
    static CURSEFORGE: OnceLock<Mutex<Instant>> = OnceLock::new();
    let host = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.contains("modrinth.com") {
        MODRINTH.get_or_init(|| Mutex::new(Instant::now()))
    } else {
        CURSEFORGE.get_or_init(|| Mutex::new(Instant::now()))
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn default_if_blank(value: &str, fallback: String) -> String {
    non_empty(value).unwrap_or(fallback)
}

fn normalize_minecraft_game_version(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if looks_like_minecraft_release_version(trimmed)
        || looks_like_minecraft_pre_release_version(trimmed)
        || looks_like_minecraft_snapshot_version(trimmed)
    {
        return Some(trimmed.to_owned());
    }
    None
}

fn looks_like_minecraft_release_version(value: &str) -> bool {
    let mut segments = value.split('.');
    let Some(major) = segments.next() else {
        return false;
    };
    let Some(minor) = segments.next() else {
        return false;
    };
    if major != "1" || minor.is_empty() || !minor.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    match segments.next() {
        Some(patch) if !patch.is_empty() && patch.chars().all(|ch| ch.is_ascii_digit()) => {
            segments.next().is_none()
        }
        None => true,
        _ => false,
    }
}

fn looks_like_minecraft_pre_release_version(value: &str) -> bool {
    for marker in ["-pre", "-rc"] {
        if let Some((base, suffix)) = value.split_once(marker) {
            return looks_like_minecraft_release_version(base)
                && !suffix.is_empty()
                && suffix.chars().all(|ch| ch.is_ascii_digit());
        }
    }
    false
}

fn looks_like_minecraft_snapshot_version(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 6
        && bytes.len() <= 7
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2] == b'w'
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
        && bytes[5].is_ascii_lowercase()
        && bytes.get(6).is_none()
}

fn format_loader_label(modloader: &str, version: &str) -> String {
    let version = version.trim();
    if version.is_empty() {
        modloader.trim().to_owned()
    } else {
        format!("{} {}", modloader.trim(), version)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedSource {
    Modrinth,
    CurseForge,
}

#[derive(Debug)]
struct MrpackDependencyInfo {
    game_version: String,
    modloader: String,
    modloader_version: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MrpackManifest {
    #[serde(default)]
    name: String,
    #[serde(rename = "versionId", default)]
    version_id: String,
    #[serde(default)]
    summary: Option<String>,
    dependencies: HashMap<String, String>,
    #[serde(default)]
    files: Vec<MrpackFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct MrpackFile {
    path: String,
    #[serde(default)]
    downloads: Vec<String>,
    #[serde(default)]
    env: Option<MrpackFileEnv>,
}

#[derive(Debug, Clone, Deserialize)]
struct MrpackFileEnv {
    #[serde(default)]
    client: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CurseForgePackManifest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    author: String,
    minecraft: CurseForgePackMinecraft,
    #[serde(default)]
    files: Vec<CurseForgePackFile>,
    #[serde(default)]
    overrides: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CurseForgePackMinecraft {
    version: String,
    #[serde(rename = "modLoaders", default)]
    mod_loaders: Vec<CurseForgePackModLoader>,
}

#[derive(Debug, Clone, Deserialize)]
struct CurseForgePackModLoader {
    id: String,
    #[serde(default)]
    primary: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct CurseForgePackFile {
    #[serde(rename = "projectID")]
    project_id: u64,
    #[serde(rename = "fileID")]
    file_id: u64,
    #[serde(default)]
    required: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_mrpack_dependencies_for_fabric() {
        let dependencies = HashMap::from([
            ("minecraft".to_owned(), "1.21.1".to_owned()),
            ("fabric-loader".to_owned(), "0.16.10".to_owned()),
        ]);

        let resolved = resolve_mrpack_dependencies(&dependencies).expect("expected dependencies");
        assert_eq!(resolved.game_version, "1.21.1");
        assert_eq!(resolved.modloader, "Fabric");
        assert_eq!(resolved.modloader_version, "0.16.10");
    }

    #[test]
    fn rejects_invalid_mrpack_game_version() {
        let dependencies = HashMap::from([
            (
                "minecraft".to_owned(),
                "fabric-loader-0.16.10-1.21.1".to_owned(),
            ),
            ("fabric-loader".to_owned(), "0.16.10".to_owned()),
        ]);

        let result = resolve_mrpack_dependencies(&dependencies);
        assert!(result.is_err());
    }

    #[test]
    fn safe_join_rejects_parent_traversal() {
        let result = join_safe(Path::new("/tmp/root"), "../mods/evil.jar");
        assert!(result.is_err());
    }

    #[test]
    fn modrinth_fallback_queries_include_project_name_once() {
        let file = curseforge::File {
            id: 1,
            display_name: "Sodium".to_owned(),
            file_name: "sodium-fabric-1.0.0.jar".to_owned(),
            file_date: String::new(),
            download_count: 0,
            download_url: None,
            hashes: Vec::new(),
            dependencies: Vec::new(),
            game_versions: Vec::new(),
        };

        let queries = modrinth_fallback_queries(&file, Some("Sodium"));
        assert_eq!(queries.first().map(String::as_str), Some("Sodium"));
        assert_eq!(
            queries
                .iter()
                .filter(|query| query.as_str() == "Sodium")
                .count(),
            1
        );
    }

    #[test]
    fn search_fallback_requires_exact_filename_match() {
        let curseforge_file = curseforge::File {
            id: 1,
            display_name: "GeckoLib".to_owned(),
            file_name: "geckolib-forge-1.20.1-4.4.9.jar".to_owned(),
            file_date: String::new(),
            download_count: 0,
            download_url: None,
            hashes: Vec::new(),
            dependencies: Vec::new(),
            game_versions: Vec::new(),
        };
        let version = modrinth::ProjectVersion {
            id: "version".to_owned(),
            project_id: "project".to_owned(),
            version_number: "4.4.9".to_owned(),
            date_published: String::new(),
            downloads: 0,
            loaders: vec!["forge".to_owned()],
            game_versions: vec!["1.20.1".to_owned()],
            dependencies: Vec::new(),
            files: vec![modrinth::ProjectVersionFile {
                url: "https://example.invalid/geckolib-neoforge.jar".to_owned(),
                filename: "geckolib-neoforge-1.20.1-4.4.9.jar".to_owned(),
                primary: true,
            }],
        };

        assert!(
            select_modrinth_backup_file(&version, &curseforge_file, "1.20.1", "Forge", true)
                .is_none()
        );
    }

    #[test]
    fn hash_fallback_rejects_loader_or_game_version_mismatch() {
        let curseforge_file = curseforge::File {
            id: 1,
            display_name: "Crop Marker".to_owned(),
            file_name: "crop-marker-forge-1.20.1-1.2.2.jar".to_owned(),
            file_date: String::new(),
            download_count: 0,
            download_url: None,
            hashes: Vec::new(),
            dependencies: Vec::new(),
            game_versions: Vec::new(),
        };
        let version = modrinth::ProjectVersion {
            id: "version".to_owned(),
            project_id: "project".to_owned(),
            version_number: "1.2.2".to_owned(),
            date_published: String::new(),
            downloads: 0,
            loaders: vec!["forge".to_owned()],
            game_versions: vec!["1.20.1".to_owned()],
            dependencies: Vec::new(),
            files: vec![modrinth::ProjectVersionFile {
                url: "https://example.invalid/crop-marker-forge-1.20.4.jar".to_owned(),
                filename: "crop-marker-forge-1.20.4-1.2.2.jar".to_owned(),
                primary: true,
            }],
        };

        assert!(
            select_modrinth_backup_file(&version, &curseforge_file, "1.20.1", "Forge", false)
                .is_none()
        );
    }

    #[test]
    fn normalizes_real_minecraft_versions_only() {
        assert_eq!(
            normalize_minecraft_game_version("1.21.1").as_deref(),
            Some("1.21.1")
        );
        assert_eq!(
            normalize_minecraft_game_version("24w14a").as_deref(),
            Some("24w14a")
        );
        assert_eq!(
            normalize_minecraft_game_version("1.20.5-rc1").as_deref(),
            Some("1.20.5-rc1")
        );
        assert!(normalize_minecraft_game_version("fabric-loader-0.16.10-1.21.1").is_none());
        assert!(normalize_minecraft_game_version("2.4.0").is_none());
    }

    #[test]
    fn extracts_game_version_from_meta_style_identifiers() {
        assert_eq!(
            find_minecraft_version_in_text("fabric-loader-0.16.10-1.21.1").as_deref(),
            Some("1.21.1")
        );
    }

    #[test]
    fn curseforge_file_api_download_requires_non_empty_download_url() {
        let mut file = curseforge::File {
            id: 1,
            display_name: "Test".to_owned(),
            file_name: "test.jar".to_owned(),
            file_date: String::new(),
            download_count: 0,
            download_url: Some("https://example.invalid/test.jar".to_owned()),
            hashes: Vec::new(),
            dependencies: Vec::new(),
            game_versions: Vec::new(),
        };
        assert!(curseforge_file_has_api_download(&file));

        file.download_url = Some("   ".to_owned());
        assert!(!curseforge_file_has_api_download(&file));

        file.download_url = None;
        assert!(!curseforge_file_has_api_download(&file));
    }
}
