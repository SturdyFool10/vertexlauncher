//! Importing instances and installing modpacks from Discover: the background tasks, the CurseForge manual-download
//! detour, and what happens when they finish. The packaging, download and install code is the same code the egui
//! front end runs (`import_flow`, `discover_flow`); this drives it from `AppState`.

use std::collections::HashMap;
use std::sync::mpsc;

use launcher_ui::notification;
use launcher_ui::screens::{DiscoverInstallRequest, DiscoverInstallSource};

use super::super::discover_flow::install_discover_modpack_in_background;
use super::super::import_flow::{
    ManualDownloadContinuation, PendingCurseForgeManualDownloadState,
    cleanup_import_request_manual_staging, cleanup_pending_curseforge_manual_download,
    import_package_in_background, scan_curseforge_manual_downloads,
    step_pending_curseforge_manual_download,
};
use super::super::import_instance_modal::{
    CurseForgeManualDownloadRequirement, ImportInstanceState, ImportPackageError, ImportProgress,
    ImportRequest, ImportSource, ImportTaskResult, prepare_curseforge_manual_download_for_file,
    prepare_curseforge_manual_downloads,
};
use super::super::startup::start_initial_instance_install_with;
use super::{AppState, Dialog};
use launcher_ui::screens::AppScreen;

type ImportPreflight = Result<Option<Vec<CurseForgeManualDownloadRequirement>>, String>;

/// All in-flight import/install work.
#[derive(Default)]
pub struct Flows {
    pub import: ImportInstanceState,
    in_flight_import_request: Option<ImportRequest>,
    import_preflight_request: Option<ImportRequest>,
    import_preflight_rx: Option<mpsc::Receiver<ImportPreflight>>,
    pub pending_manual: Option<PendingCurseForgeManualDownloadState>,
    discover_preflight_request: Option<DiscoverInstallRequest>,
    discover_preflight_in_flight: bool,
    discover_preflight: launcher_runtime::WorkerChannel<
        Result<Option<CurseForgeManualDownloadRequirement>, String>,
    >,
    discover_progress: launcher_runtime::WorkerChannel<ImportProgress>,
    discover_results: launcher_runtime::WorkerChannel<ImportTaskResult>,
}

impl Flows {
    fn manual_or_import_blocked(&self) -> bool {
        self.import.import_in_flight
            || self.import_preflight_rx.is_some()
            || self.pending_manual.is_some()
    }
}

impl AppState {
    // -- Import ---------------------------------------------------------------------------------------------------

    pub fn open_import_dialog(&mut self) {
        self.dialog = Some(Dialog::Import);
    }

    /// Starts importing `request` (a manifest file or another launcher's instance folder).
    pub fn start_import(&mut self, mut request: ImportRequest) {
        if self.flows.manual_or_import_blocked() {
            return;
        }
        request.max_concurrent_downloads = self.config.download_max_concurrent().max(1);
        let is_curseforge_zip = matches!(&request.source, ImportSource::ManifestFile(path)
            if path.extension().and_then(|value| value.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("zip")));
        if is_curseforge_zip && request.manual_curseforge_files.is_empty() {
            self.start_import_preflight(request);
            return;
        }
        self.spawn_import(request);
    }

    fn start_import_preflight(&mut self, request: ImportRequest) {
        let (tx, rx) = mpsc::channel::<ImportPreflight>();
        let flows = &mut self.flows;
        flows.import_preflight_rx = Some(rx);
        flows.import.error = None;
        flows.import.import_in_flight = true;
        flows.import.import_latest_progress = Some(ImportProgress {
            message: "Checking CurseForge download restrictions...".to_owned(),
            completed_steps: 0,
            total_steps: 1,
        });
        flows.import_preflight_request = Some(request.clone());
        launcher_runtime::spawn_blocking_detached(move || {
            let _ = tx.send(prepare_curseforge_manual_downloads(&request));
        });
    }

    fn spawn_import(&mut self, request: ImportRequest) {
        let tx = self.flows.import.import_results.sender();
        let progress_tx = self.flows.import.import_progress.sender();
        self.flows.import.error = None;
        self.flows.import.import_in_flight = true;
        self.flows.import.import_latest_progress = None;
        self.flows.in_flight_import_request = Some(request.clone());
        let store = self.instances.clone();
        let root = self.installations_root();
        launcher_runtime::spawn_blocking_detached(move || {
            let _ = tx.send(import_package_in_background(
                store,
                root,
                request,
                progress_tx,
            ));
        });
    }

    fn open_manual_downloads_for_import(
        &mut self,
        request: ImportRequest,
        requirements: Vec<CurseForgeManualDownloadRequirement>,
        staged: HashMap<u64, std::path::PathBuf>,
    ) {
        let root = self.installations_root();
        match PendingCurseForgeManualDownloadState::new(
            ManualDownloadContinuation::Import(request),
            requirements,
            staged,
            root.as_path(),
        ) {
            Ok(mut pending) => {
                if let Err(err) = scan_curseforge_manual_downloads(&mut pending) {
                    self.flows.import.error = Some(format!(
                        "Failed to prepare manual CurseForge downloads: {err}"
                    ));
                    cleanup_pending_curseforge_manual_download(Some(pending));
                    return;
                }
                if pending.pending_files.is_empty() {
                    if let ManualDownloadContinuation::Import(request) = &pending.continuation {
                        let mut request = request.clone();
                        request.manual_curseforge_files = pending.staged_files.clone();
                        request.manual_curseforge_staging_dir = Some(pending.staging_dir.clone());
                        self.spawn_import(request);
                    }
                } else {
                    self.flows.pending_manual = Some(pending);
                }
            }
            Err(err) => {
                self.flows.import.error = Some(format!(
                    "Failed to prepare manual CurseForge downloads: {err}"
                ));
            }
        }
    }

    fn poll_import_preflight(&mut self) {
        let Some(rx) = self.flows.import_preflight_rx.as_ref() else {
            return;
        };
        let Ok(result) = rx.try_recv() else { return };
        self.flows.import_preflight_rx = None;
        self.flows.import.import_in_flight = false;
        self.flows.import.import_latest_progress = None;
        let request = self.flows.import_preflight_request.take();
        match (request, result) {
            (Some(request), Ok(Some(requirements))) if !requirements.is_empty() => {
                self.open_manual_downloads_for_import(request, requirements, HashMap::new());
            }
            (Some(request), Ok(_)) => self.spawn_import(request),
            (_, Err(err)) => {
                tracing::error!(target: "vertexlauncher/se5/import", error = %err, "CurseForge manual-download preflight failed.");
                self.flows.import.error =
                    Some(format!("Failed to prepare CurseForge import: {err}"));
            }
            (None, Ok(_)) => {}
        }
    }

    fn poll_import(&mut self) {
        if let Some(progress) = self.flows.import.import_progress.drain().items.pop() {
            self.flows.import.import_latest_progress = Some(progress);
        }
        let Some(result) = self
            .flows
            .import
            .import_results
            .drain()
            .items
            .into_iter()
            .next()
        else {
            return;
        };
        self.flows.import.import_in_flight = false;
        self.flows.import.import_latest_progress = None;
        let mut original = self.flows.in_flight_import_request.take();
        match result {
            Ok((store, instance)) => {
                let root = self.installations_root();
                let config = self.config.clone();
                self.instances = store;
                cleanup_import_request_manual_staging(original.as_ref());
                cleanup_pending_curseforge_manual_download(self.flows.pending_manual.take());
                let tx = self.initial_install_sender();
                start_initial_instance_install_with(Some(tx), &instance, root.as_path(), &config);
                self.open_instance(&instance.id);
                if self.dialog == Some(Dialog::Import) {
                    self.dialog = None;
                }
                self.flows.import.reset();
            }
            Err(err) => {
                if let (
                    Some(request),
                    ImportPackageError::ManualCurseForgeDownloads {
                        requirements,
                        staged_files,
                    },
                ) = (original.take(), err.clone())
                {
                    cleanup_import_request_manual_staging(Some(&request));
                    self.open_manual_downloads_for_import(request, requirements, staged_files);
                    return;
                }
                cleanup_import_request_manual_staging(original.as_ref());
                cleanup_pending_curseforge_manual_download(self.flows.pending_manual.take());
                tracing::error!(target: "vertexlauncher/se5/import", error = %err, "Import profile task failed.");
                self.flows.import.error = Some(format!("Failed to import profile: {err}"));
            }
        }
    }

    // -- Discover installs -----------------------------------------------------------------------------------------

    /// Starts creating an instance from a Discover modpack version.
    pub fn start_discover_install(&mut self, request: DiscoverInstallRequest) {
        if self.flows.manual_or_import_blocked() || self.flows.discover_preflight_in_flight {
            return;
        }
        if matches!(
            &request.source,
            DiscoverInstallSource::CurseForge {
                manual_download_path: None,
                ..
            }
        ) {
            self.start_discover_preflight(request);
            return;
        }
        self.spawn_discover_install(request);
    }

    fn start_discover_preflight(&mut self, request: DiscoverInstallRequest) {
        let DiscoverInstallSource::CurseForge {
            project_id,
            file_id,
            ..
        } = &request.source
        else {
            self.spawn_discover_install(request);
            return;
        };
        let (project_id, file_id) = (*project_id, *file_id);
        let tx = self.flows.discover_preflight.sender();
        self.discover
            .begin_install("Checking CurseForge download restrictions...");
        self.flows.discover_preflight_request = Some(request);
        self.flows.discover_preflight_in_flight = true;
        launcher_runtime::spawn_blocking_detached(move || {
            let _ = tx.send(prepare_curseforge_manual_download_for_file(
                project_id, file_id,
            ));
        });
    }

    fn spawn_discover_install(&mut self, request: DiscoverInstallRequest) {
        let tx = self.flows.discover_results.sender();
        let progress_tx = self.flows.discover_progress.sender();
        self.discover
            .begin_install(format!("Downloading {}...", request.version_name));
        let store = self.instances.clone();
        let root = self.installations_root();
        launcher_runtime::spawn_blocking_detached(move || {
            let _ = tx.send(install_discover_modpack_in_background(
                store,
                root,
                request,
                progress_tx,
            ));
        });
    }

    fn poll_discover_preflight(&mut self) {
        if !self.flows.discover_preflight_in_flight {
            return;
        }
        let Some(result) = self
            .flows
            .discover_preflight
            .drain()
            .items
            .into_iter()
            .next()
        else {
            return;
        };
        self.flows.discover_preflight_in_flight = false;
        let request = self.flows.discover_preflight_request.take();
        match (request, result) {
            (Some(request), Ok(Some(requirement))) => {
                let root = self.installations_root();
                match PendingCurseForgeManualDownloadState::new(
                    ManualDownloadContinuation::DiscoverInstall(request),
                    vec![requirement],
                    HashMap::new(),
                    root.as_path(),
                ) {
                    Ok(mut pending) => {
                        if let Err(err) = scan_curseforge_manual_downloads(&mut pending) {
                            cleanup_pending_curseforge_manual_download(Some(pending));
                            self.discover.finish_install(Err(format!(
                                "Failed to prepare manual CurseForge download: {err}"
                            )));
                            return;
                        }
                        if pending.pending_files.is_empty() {
                            let ManualDownloadContinuation::DiscoverInstall(request) =
                                &pending.continuation
                            else {
                                return;
                            };
                            let mut request = request.clone();
                            if let DiscoverInstallSource::CurseForge {
                                manual_download_path,
                                download_url,
                                ..
                            } = &mut request.source
                            {
                                *manual_download_path =
                                    pending.staged_files.values().next().cloned();
                                *download_url = None;
                            }
                            self.flows.pending_manual = Some(pending);
                            self.spawn_discover_install(request);
                        } else {
                            self.discover
                                .begin_install("Waiting for manual CurseForge download...");
                            self.flows.pending_manual = Some(pending);
                        }
                    }
                    Err(err) => self.discover.finish_install(Err(format!(
                        "Failed to prepare manual CurseForge download: {err}"
                    ))),
                }
            }
            (Some(request), Ok(None)) => self.spawn_discover_install(request),
            (_, Err(err)) => self
                .discover
                .finish_install(Err(format!("Failed to prepare CurseForge install: {err}"))),
            (None, Ok(_)) => {}
        }
    }

    fn poll_discover_install(&mut self) {
        for progress in self.flows.discover_progress.drain().items {
            self.discover.apply_install_progress(
                progress.message,
                progress.completed_steps,
                progress.total_steps,
            );
        }
        let drained = self.flows.discover_results.drain();
        if drained.disconnected {
            self.discover
                .finish_install(Err("Install task stopped unexpectedly.".to_owned()));
            return;
        }
        let Some(result) = drained.items.into_iter().next() else {
            return;
        };
        cleanup_pending_curseforge_manual_download(self.flows.pending_manual.take());
        match result {
            Ok((store, instance)) => {
                let root = self.installations_root();
                let config = self.config.clone();
                self.instances = store;
                let tx = self.initial_install_sender();
                start_initial_instance_install_with(Some(tx), &instance, root.as_path(), &config);
                self.open_instance(&instance.id);
                self.discover
                    .finish_install(Ok("Created instance from modpack.".to_owned()));
            }
            Err(err) => {
                tracing::error!(target: "vertexlauncher/se5/discover", error = %err, "Discover modpack install failed.");
                self.discover.finish_install(Err(err.to_string()));
            }
        }
    }

    // -- Manual CurseForge downloads -------------------------------------------------------------------------------

    fn poll_manual_downloads(&mut self) {
        let Some(state) = self.flows.pending_manual.as_mut() else {
            return;
        };
        if !step_pending_curseforge_manual_download(state) {
            return;
        }
        let Some(mut pending) = self.flows.pending_manual.take() else {
            return;
        };
        pending.error = None;
        match &mut pending.continuation {
            ManualDownloadContinuation::Import(request) => {
                request.manual_curseforge_files = pending.staged_files.clone();
                request.manual_curseforge_staging_dir = Some(pending.staging_dir.clone());
                let request = request.clone();
                self.spawn_import(request);
            }
            ManualDownloadContinuation::DiscoverInstall(request) => {
                let Some(staged) = pending.staged_files.values().next().cloned() else {
                    pending.error =
                        Some("Manual CurseForge download staging was empty.".to_owned());
                    self.flows.pending_manual = Some(pending);
                    return;
                };
                if let DiscoverInstallSource::CurseForge {
                    manual_download_path,
                    download_url,
                    ..
                } = &mut request.source
                {
                    *manual_download_path = Some(staged);
                    *download_url = None;
                }
                let request = request.clone();
                cleanup_pending_curseforge_manual_download(Some(pending));
                self.spawn_discover_install(request);
            }
        }
    }

    /// Cancels the manual-download wait and whatever import or install it belonged to.
    pub fn cancel_manual_downloads(&mut self) {
        let continuation =
            self.flows
                .pending_manual
                .as_ref()
                .map(|pending| match pending.continuation {
                    ManualDownloadContinuation::Import(_) => 0u8,
                    ManualDownloadContinuation::DiscoverInstall(_) => 1u8,
                });
        cleanup_pending_curseforge_manual_download(self.flows.pending_manual.take());
        match continuation {
            Some(0) => {
                self.flows.import.reset();
                if self.dialog == Some(Dialog::Import) {
                    self.dialog = None;
                }
            }
            Some(_) => self
                .discover
                .finish_install(Err("CurseForge install canceled.".to_owned())),
            None => {}
        }
    }

    /// Polls every import/install task. Call each frame.
    pub fn poll_flows(&mut self) {
        super::super::import_instance_modal::poll_preview_results(&mut self.flows.import);
        self.poll_import_preflight();
        self.poll_import();
        self.poll_discover_preflight();
        self.poll_discover_install();
        self.poll_manual_downloads();
    }

    /// Files dropped on the window: packs go to the import dialog.
    pub fn handle_dropped_file(&mut self, path: std::path::PathBuf) {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if !matches!(extension.as_str(), "vtmpack" | "mrpack" | "zip") {
            notification::warn!(
                "import",
                "Drop a .vtmpack, .mrpack or CurseForge .zip pack to import it."
            );
            return;
        }
        if self.flows.import.import_in_flight {
            return;
        }
        self.flows.import.reset();
        self.flows.import.package_path = path;
        super::super::import_instance_modal::load_preview_from_state(&mut self.flows.import);
        self.dialog = Some(Dialog::Import);
        if self.screen == AppScreen::Discover {
            self.screen = AppScreen::Library;
        }
    }
}
