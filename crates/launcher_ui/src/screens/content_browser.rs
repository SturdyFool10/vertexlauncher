use config::Config;
use content_resolver::detect_installed_content_kind;
use curseforge::{Client as CurseForgeClient, MINECRAFT_GAME_ID};
use egui::Ui;
use installation::{
    DownloadBatchTask, DownloadPolicy, InstallProgressCallback, InstallStage,
    MinecraftVersionEntry, download_batch_with_progress, fetch_version_catalog,
};
use instances::{InstanceStore, instance_root_path};
use managed_content::{
    ContentInstallManifest, InstalledContentProject, ManagedContentSource, load_content_manifest,
    save_content_manifest,
};
use modprovider::{ContentSource, UnifiedContentEntry};
use modrinth::Client as ModrinthClient;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use textui::TextUi;
use textui_egui::prelude::*;
use ui_foundation::{UiMetrics, themed_text_input};

use crate::app::tokio_runtime;
use crate::assets;
use crate::install_activity;
use crate::notification;
use crate::ui::{
    components::{remote_tiled_image, settings_widgets},
    style,
};

use super::AppScreen;

#[path = "content_browser_install.rs"]
mod content_browser_install;
#[path = "content_browser/content_browser_output.rs"]
mod content_browser_output;
#[path = "content_browser_state.rs"]
mod content_browser_state;
#[path = "content_browser_ui.rs"]
mod content_browser_ui;
#[path = "content_browser_workers.rs"]
mod content_browser_workers;
use self::content_browser_install::*;
use self::content_browser_output::ContentBrowserOutput;
pub(crate) use self::content_browser_state::BulkContentUpdate;
pub use self::content_browser_state::ContentBrowserState;
use self::content_browser_state::*;
use self::content_browser_ui::*;
use self::content_browser_workers::*;

const CONTENT_SEARCH_PER_PROVIDER_LIMIT: u32 = 35;
const CONTENT_DOWNLOAD_REQUIRED_DEPENDENCY_RELATION_TYPE: u32 = 3;
const DEFAULT_DISCOVERY_QUERY_MOD: &str = "mod";
const MODRINTH_DOWNLOAD_MIN_SPACING: Duration = Duration::from_millis(250);
const CURSEFORGE_DOWNLOAD_MIN_SPACING: Duration = Duration::from_millis(500);
const DEFAULT_DISCOVERY_QUERY_RESOURCE_PACK: &str = "resource pack";
const DEFAULT_DISCOVERY_QUERY_SHADER: &str = "shader";
const DEFAULT_DISCOVERY_QUERY_DATA_PACK: &str = "data pack";
const DETAIL_VERSION_FETCH_PAGE_SIZE: u32 = 50;
const DETAIL_VERSION_FETCH_MAX_PAGES: u32 = 5;
const TILE_ACTION_BUTTON_WIDTH: f32 = 28.0;
const TILE_ACTION_BUTTON_HEIGHT: f32 = 28.0;
const TILE_ACTION_BUTTON_GAP_XS: f32 = 4.0;
const TILE_DOWNLOAD_PROGRESS_WIDTH: f32 = 96.0;
const CONTENT_UPDATE_LOG_TARGET: &str = "vertexlauncher/content_update";
const VERTEX_PREFETCH_DIR_NAME: &str = "vertex_prefetch";
const VERSION_CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(75);
const DETAIL_VERSIONS_FETCH_TIMEOUT: Duration = Duration::from_secs(45);
const CONTENT_BROWSER_SEARCH_CACHE_MAX_ENTRIES: usize = 12;
const CONTENT_BROWSER_VERSION_DROPDOWN_ID_KEY: &str = "content_browser_version_dropdown_id";
const CONTENT_BROWSER_SCOPE_DROPDOWN_ID_KEY: &str = "content_browser_scope_dropdown_id";
const CONTENT_BROWSER_SORT_DROPDOWN_ID_KEY: &str = "content_browser_sort_dropdown_id";
const CONTENT_BROWSER_LOADER_DROPDOWN_ID_KEY: &str = "content_browser_loader_dropdown_id";

static PENDING_EXTERNAL_DETAIL_OPEN: OnceLock<Mutex<Option<UnifiedContentEntry>>> = OnceLock::new();

pub(crate) fn request_open_detail_for_content(entry: UnifiedContentEntry) {
    let store = PENDING_EXTERNAL_DETAIL_OPEN.get_or_init(|| Mutex::new(None));
    if let Ok(mut pending) = store.lock() {
        *pending = Some(entry);
    }
}

pub fn version_dropdown_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| {
        data.get_temp::<egui::Id>(egui::Id::new(CONTENT_BROWSER_VERSION_DROPDOWN_ID_KEY))
    })
}

pub fn scope_dropdown_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::new(CONTENT_BROWSER_SCOPE_DROPDOWN_ID_KEY)))
}

pub fn sort_dropdown_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::new(CONTENT_BROWSER_SORT_DROPDOWN_ID_KEY)))
}

pub fn loader_dropdown_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| {
        data.get_temp::<egui::Id>(egui::Id::new(CONTENT_BROWSER_LOADER_DROPDOWN_ID_KEY))
    })
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    selected_instance_id: Option<&str>,
    instances: &InstanceStore,
    config: &Config,
    state: &mut ContentBrowserState,
    force_reset: bool,
) -> ContentBrowserOutput {
    let mut output = ContentBrowserOutput::default();
    if force_reset {
        *state = ContentBrowserState::default();
    }

    poll_search(state);
    poll_detail_versions(state);
    poll_downloads(state);
    poll_version_catalog(state);
    poll_identify_results(state);

    if state.search_in_flight
        || state.detail_versions_in_flight
        || state.download_in_flight
        || state.identify_in_flight
        || state.version_catalog_in_flight
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }

    let Some(instance_id) = selected_instance_id else {
        let _ = text_ui.label(
            ui,
            "content_browser_no_instance",
            "Select an instance first. Content Browser installs into the selected instance.",
            &style::muted(ui),
        );
        return output;
    };

    let Some(instance) = instances.find(instance_id) else {
        let _ = text_ui.label(
            ui,
            "content_browser_missing_instance",
            "Selected instance no longer exists.",
            &style::error_text(ui),
        );
        return output;
    };

    if state.active_instance_id.as_deref() != Some(instance.id.as_str()) {
        state.active_instance_id = Some(instance.id.clone());
        state.active_instance_name = Some(instance.name.clone());
        state.loader = browser_loader_from_modloader(instance.modloader.as_str());
        state.minecraft_version_filter = instance.game_version.clone();
        state.content_scope = ContentScope::Mods;
        state.mod_sort_mode = ModSortMode::Popularity;
        state.download_queue.clear();
        state.download_in_flight = false;
        state.active_download = None;
        state.cached_manifest = None;
        state.manifest_dirty = true;
        state.current_page = 1;
        state.current_view = ContentBrowserPage::Browse;
        state.detail_entry = None;
        state.detail_tab = ContentDetailTab::Overview;
        state.detail_versions.clear();
        state.detail_versions_project_key = None;
        state.detail_versions_error = None;
        state.detail_versions_in_flight = false;
        state.detail_loader_filter = state.loader;
        state.detail_minecraft_version_filter = instance.game_version.clone();
        state.results = BrowserSearchSnapshot::default();
        state.active_search_request = None;
        state.search_completed_tasks = 0;
        state.search_total_tasks = 0;
        state.search_in_flight = false;
        state.search_notification_active = false;
        state.auto_populated_instance_id = None;
    }

    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let instance_root = instance_root_path(&installations_root, instance);
    let game_version = instance.game_version.trim().to_owned();

    request_version_catalog(state);
    apply_pending_external_detail_open(state);

    let _ = text_ui.label(
        ui,
        ("content_browser_context", instance.id.as_str()),
        &format!(
            "Instance: {} | Minecraft {} | Loader {}",
            instance.name,
            if game_version.is_empty() {
                "n/a"
            } else {
                game_version.as_str()
            },
            instance.modloader.trim()
        ),
        &style::caption(ui),
    );
    ui.add_space(style::SPACE_MD);

    maybe_start_queued_download(state, instance.name.as_str(), instance_root.as_path());

    if let Some(status) = state.status_message.as_deref() {
        let _ = text_ui.label(
            ui,
            ("content_browser_status", instance.id.as_str()),
            status,
            &style::muted(ui),
        );
    }

    for warning in &state.results.warnings {
        let _ = text_ui.label(
            ui,
            ("content_browser_warning", instance.id.as_str(), warning),
            warning,
            &style::warning_text(ui),
        );
    }

    ui.add_space(style::SPACE_MD);
    match state.current_view {
        ContentBrowserPage::Browse => {
            if state.manifest_dirty || state.cached_manifest.is_none() {
                state.cached_manifest = Some(load_content_manifest(instance_root.as_path()));
                state.manifest_dirty = false;
            }
            let manifest = state.cached_manifest.clone().expect("just populated");
            render_controls(ui, text_ui, instance.id.as_str(), state);

            if state.auto_populated_instance_id.as_deref() != Some(instance.id.as_str())
                && !state.search_in_flight
                && state.query_input.trim().is_empty()
                && state.search_tags.is_empty()
            {
                let request = BrowserSearchRequest {
                    query: None,
                    tags: Vec::new(),
                    game_version: normalize_optional(state.minecraft_version_filter.as_str()),
                    loader: state.loader,
                    content_scope: ContentScope::Mods,
                    mod_sort_mode: ModSortMode::Popularity,
                    page: 1,
                };
                state.current_page = 1;
                request_search(state, request);
                state.auto_populated_instance_id = Some(instance.id.clone());
            }

            let results_height = (ui.available_height() - 42.0).max(140.0);
            let render_outcome = render_results(
                ui,
                text_ui,
                instance.id.as_str(),
                state,
                &manifest,
                results_height,
            );
            if let Some(page) = render_outcome.requested_page
                && page != state.current_page
            {
                state.current_page = page;
                request_search_for_current_filters(state, false);
            }
            if let Some(entry) = render_outcome.open_entry {
                open_detail_page(state, &entry);
            }
        }
        ContentBrowserPage::Detail => {
            render_detail_page(
                ui,
                text_ui,
                instance.id.as_str(),
                instance_root.as_path(),
                state,
            );
        }
    }

    ui.add_space(style::SPACE_LG);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = style::SPACE_SM;
        let button_style = style::neutral_button(ui);
        if state.current_view == ContentBrowserPage::Detail {
            if text_ui
                .button(
                    ui,
                    ("content_browser_back_to_browser", instance.id.as_str()),
                    "Back to Mod Browser",
                    &button_style,
                )
                .clicked()
            {
                state.current_view = ContentBrowserPage::Browse;
            }
        }
        if text_ui
            .button(
                ui,
                ("content_browser_back_to_instance", instance.id.as_str()),
                "Back to Instance",
                &button_style,
            )
            .clicked()
        {
            output.requested_screen = Some(AppScreen::Instance);
        }
    });
    output
}

fn request_search_for_current_filters(state: &mut ContentBrowserState, reset_page: bool) {
    if reset_page {
        state.current_page = 1;
    }
    request_search(
        state,
        BrowserSearchRequest {
            query: normalize_optional(compose_search_query(
                state.query_input.as_str(),
                state.search_tags.as_slice(),
            )),
            tags: state.search_tags.clone(),
            game_version: normalize_optional(state.minecraft_version_filter.as_str()),
            loader: state.loader,
            content_scope: state.content_scope,
            mod_sort_mode: state.mod_sort_mode,
            page: state.current_page.max(1),
        },
    );
}

fn compose_search_query(input: &str, tags: &[String]) -> String {
    let mut parts = Vec::with_capacity(tags.len() + 1);
    for tag in tags {
        let trimmed = tag.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_owned());
        }
    }
    let trimmed_input = input.trim();
    if !trimmed_input.is_empty() {
        parts.push(trimmed_input.to_owned());
    }
    parts.join(" ")
}

pub(crate) fn update_installed_content_to_version(
    instance_root: &Path,
    entry: &UnifiedContentEntry,
    installed_file_path: &Path,
    version_id: &str,
    game_version: &str,
    loader_label: &str,
) -> Result<String, String> {
    update_installed_content_to_version_with_prefetched_downloads(
        instance_root,
        entry,
        installed_file_path,
        version_id,
        game_version,
        loader_label,
        &HashMap::new(),
    )
}

pub(crate) fn bulk_update_installed_content(
    instance_root: &Path,
    updates: &[BulkContentUpdate],
    game_version: &str,
    loader_label: &str,
    download_policy: &DownloadPolicy,
    progress: Option<&InstallProgressCallback>,
) -> Result<usize, String> {
    if updates.is_empty() {
        return Ok(0);
    }
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        root_updates = updates.len(),
        game_version = %game_version,
        loader = %loader_label,
        "planning bulk content update"
    );

    let modrinth = ModrinthClient::default();
    let curseforge = CurseForgeClient::from_env();
    let loader = browser_loader_from_modloader(loader_label);
    let manifest = load_content_manifest(instance_root);
    let mut planned_versions = HashMap::new();
    let mut queued_paths = HashSet::new();
    let mut download_tasks = Vec::new();
    let mut prefetched_paths = HashMap::new();
    let mut root_updates = Vec::new();

    for update in updates {
        let browser_entry = browser_entry_from_unified_content(&update.entry)?;
        let source = ManagedContentSource::from(update.entry.source);
        let version =
            fetch_exact_version_for_entry(&browser_entry, source, update.version_id.as_str())?;
        let resolved = resolved_download_from_version(version.clone());
        let should_apply = collect_content_download_tasks_for_request(
            instance_root,
            &manifest,
            &browser_entry,
            &resolved,
            game_version,
            loader,
            &modrinth,
            curseforge.as_ref(),
            &mut planned_versions,
            &mut queued_paths,
            &mut download_tasks,
            &mut prefetched_paths,
        )?;
        if should_apply {
            tracing::debug!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                project = %update.entry.name,
                version_id = %version.version_id,
                installed_path = %update.installed_file_path.display(),
                "queued root content update"
            );
            root_updates.push((
                update.entry.clone(),
                update.installed_file_path.clone(),
                version.version_id.clone(),
            ));
        }
    }

    if !download_tasks.is_empty() {
        tracing::info!(
            target: CONTENT_UPDATE_LOG_TARGET,
            instance_root = %instance_root.display(),
            download_tasks = download_tasks.len(),
            root_updates = root_updates.len(),
            "downloading prefetched files for bulk content update"
        );
        download_batch_with_progress(
            download_tasks,
            download_policy,
            InstallStage::DownloadingCore,
            progress,
        )
        .map_err(|err| format!("failed to download queued content updates: {err}"))?;
    }

    let apply_result = (|| -> Result<usize, String> {
        let mut applied = 0usize;
        for (entry, installed_file_path, version_id) in root_updates {
            tracing::info!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                project = %entry.name,
                version_id = %version_id,
                installed_path = %installed_file_path.display(),
                "applying prefetched content update"
            );
            update_installed_content_to_version_with_prefetched_downloads(
                instance_root,
                &entry,
                installed_file_path.as_path(),
                version_id.as_str(),
                game_version,
                loader_label,
                &prefetched_paths,
            )?;
            applied += 1;
        }
        Ok(applied)
    })();
    cleanup_prefetched_downloads(instance_root)?;
    let applied = apply_result?;
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        applied,
        "finished bulk content update apply pass"
    );
    Ok(applied)
}

fn update_installed_content_to_version_with_prefetched_downloads(
    instance_root: &Path,
    entry: &UnifiedContentEntry,
    installed_file_path: &Path,
    version_id: &str,
    game_version: &str,
    loader_label: &str,
    prefetched_paths: &HashMap<PathBuf, PathBuf>,
) -> Result<String, String> {
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        project = %entry.name,
        version_id = %version_id,
        installed_path = %installed_file_path.display(),
        prefetched = !prefetched_paths.is_empty(),
        "starting content version update"
    );
    let browser_entry = browser_entry_from_unified_content(entry)?;
    let source = ManagedContentSource::from(entry.source);
    let version = fetch_exact_version_for_entry(&browser_entry, source, version_id)?;
    let target_path = content_target_path(instance_root, &browser_entry, &version);
    let manifest = load_content_manifest(instance_root);
    let (effective_installed_file_path, stale_requested_path) =
        resolve_installed_file_paths_for_update(
            instance_root,
            &manifest,
            &browser_entry,
            installed_file_path,
        );
    let additional_cleanup_paths = stale_requested_path
        .as_ref()
        .filter(|path| !paths_match_for_update(path.as_path(), target_path.as_path()))
        .cloned()
        .into_iter()
        .collect::<Vec<_>>();
    let staged_existing_path = stage_existing_file_for_update(
        effective_installed_file_path.as_path(),
        target_path.as_path(),
    )?;
    let mut outcome = match apply_content_install_request_with_prefetched_downloads(
        instance_root,
        ContentInstallRequest::Exact {
            entry: browser_entry,
            version,
            game_version: game_version.trim().to_owned(),
            loader: browser_loader_from_modloader(loader_label),
        },
        prefetched_paths,
        additional_cleanup_paths.as_slice(),
    ) {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                project = %entry.name,
                version_id = %version_id,
                installed_path = %effective_installed_file_path.display(),
                target_path = %target_path.display(),
                "content version update failed before finalize: {err}"
            );
            if let Some(staged_path) = staged_existing_path {
                restore_staged_update_file(
                    staged_path.as_path(),
                    effective_installed_file_path.as_path(),
                )
                .map_err(|restore_err| {
                    format!("{err} (also failed to restore original file: {restore_err})")
                })?;
            }
            return Err(err);
        }
    };
    if prefetched_paths.is_empty() {
        finalize_updated_file_replacement(
            effective_installed_file_path.as_path(),
            target_path.as_path(),
            staged_existing_path.as_deref(),
            &mut outcome.removed_files,
            None,
        )?;
    } else if let Some(staged_path) = staged_existing_path.as_ref()
        && staged_path.exists()
    {
        remove_content_path(staged_path.as_path())?;
        outcome
            .removed_files
            .push(staged_path.display().to_string());
    }
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        project = %entry.name,
        version_id = %version_id,
        target_path = %target_path.display(),
        added_files = outcome.added_files.len(),
        removed_files = outcome.removed_files.len(),
        "completed content version update"
    );
    Ok(format!(
        "Updated {}: {} added, {} removed.",
        outcome.project_name,
        outcome.added_files.len(),
        outcome.removed_files.len()
    ))
}

fn resolve_installed_file_paths_for_update(
    instance_root: &Path,
    manifest: &ContentInstallManifest,
    entry: &BrowserProjectEntry,
    requested_installed_file_path: &Path,
) -> (PathBuf, Option<PathBuf>) {
    let managed_installed_file_path = installed_project_for_entry(manifest, entry)
        .map(|(_, project)| instance_root.join(project.file_path.as_path()))
        .filter(|path| path.exists());
    let effective_installed_file_path = managed_installed_file_path
        .clone()
        .unwrap_or_else(|| requested_installed_file_path.to_path_buf());
    let stale_requested_path = if managed_installed_file_path.is_some()
        && requested_installed_file_path.exists()
        && !paths_match_for_update(
            effective_installed_file_path.as_path(),
            requested_installed_file_path,
        ) {
        Some(requested_installed_file_path.to_path_buf())
    } else {
        None
    };

    (effective_installed_file_path, stale_requested_path)
}

#[allow(clippy::too_many_arguments)]
fn collect_content_download_tasks_for_request(
    instance_root: &Path,
    manifest: &ContentInstallManifest,
    entry: &BrowserProjectEntry,
    resolved: &ResolvedDownload,
    game_version: &str,
    loader: BrowserLoader,
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
    planned_versions: &mut HashMap<String, String>,
    queued_paths: &mut HashSet<PathBuf>,
    download_tasks: &mut Vec<DownloadBatchTask>,
    prefetched_paths: &mut HashMap<PathBuf, PathBuf>,
) -> Result<bool, String> {
    let project_key = installed_project_for_entry(manifest, entry)
        .map(|(key, _)| key.to_owned())
        .unwrap_or_else(|| entry.dedupe_key.clone());
    let target_path = content_target_path_for_resolved_download(instance_root, entry, resolved);
    let existing = installed_project_for_entry(manifest, entry).map(|(_, project)| project);

    if let Some(planned_version_id) = planned_versions.get(project_key.as_str()) {
        if planned_version_id == &resolved.version_id {
            return Ok(false);
        }
        return Err(format!(
            "Bulk update requires conflicting versions of {}.",
            entry.name
        ));
    }

    if existing.is_some_and(|project| {
        project.selected_source == Some(resolved.source)
            && project.selected_version_id.as_deref() == Some(resolved.version_id.as_str())
            && target_path.exists()
    }) {
        return Ok(false);
    }

    planned_versions.insert(project_key, resolved.version_id.clone());
    if (existing.is_some() || !target_path.exists()) && queued_paths.insert(target_path.clone()) {
        let prefetched_target =
            prefetched_target_path(instance_root, entry.content_type, target_path.as_path());
        prefetched_paths.insert(target_path.clone(), prefetched_target.clone());
        download_tasks.push(DownloadBatchTask {
            url: resolved.file_url.clone(),
            destination: prefetched_target,
            expected_size: None,
        });
    }

    for dep_entry in
        dependency_to_browser_entries(resolved.dependencies.as_slice(), modrinth, curseforge)?
    {
        let dep_resolved =
            resolve_best_download(&dep_entry, game_version, loader, modrinth, curseforge)?
                .ok_or_else(|| {
                    format!(
                        "No compatible downloadable file found for dependency {}.",
                        dep_entry.name
                    )
                })?;
        let _ = collect_content_download_tasks_for_request(
            instance_root,
            manifest,
            &dep_entry,
            &dep_resolved,
            game_version,
            loader,
            modrinth,
            curseforge,
            planned_versions,
            queued_paths,
            download_tasks,
            prefetched_paths,
        )?;
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_root(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vertexlauncher-content-browser-{test_name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        ))
    }

    #[test]
    fn same_path_update_stages_and_removes_previous_file() {
        let root = temp_test_root("same-path");
        let mods_dir = root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let mod_path = mods_dir.join("example.jar");
        std::fs::write(mod_path.as_path(), b"old").expect("write old mod");

        let staged_path = stage_existing_file_for_update(mod_path.as_path(), mod_path.as_path())
            .expect("stage old file")
            .expect("expected staged path");
        assert!(
            !mod_path.exists(),
            "original path should be free for replacement"
        );
        assert!(
            staged_path.exists(),
            "backup path should exist after staging"
        );

        std::fs::write(mod_path.as_path(), b"new").expect("write replacement mod");
        let mut removed_files = Vec::new();
        finalize_updated_file_replacement(
            mod_path.as_path(),
            mod_path.as_path(),
            Some(staged_path.as_path()),
            &mut removed_files,
            None,
        )
        .expect("finalize replacement");

        assert!(mod_path.exists(), "new mod should remain in place");
        assert!(
            !staged_path.exists(),
            "staged backup should be removed after successful replacement"
        );
        assert_eq!(removed_files, vec![staged_path.display().to_string()]);

        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn different_path_update_removes_superseded_previous_file() {
        let root = temp_test_root("different-path");
        let mods_dir = root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let old_path = mods_dir.join("example-1.0.jar");
        let new_path = mods_dir.join("example-2.0.jar");
        std::fs::write(old_path.as_path(), b"old").expect("write old mod");
        std::fs::write(new_path.as_path(), b"new").expect("write new mod");

        let mut removed_files = Vec::new();
        finalize_updated_file_replacement(
            old_path.as_path(),
            new_path.as_path(),
            None,
            &mut removed_files,
            None,
        )
        .expect("finalize replacement");

        assert!(!old_path.exists(), "old mod should be removed after update");
        assert!(new_path.exists(), "new mod should remain after update");
        assert_eq!(removed_files, vec![old_path.display().to_string()]);

        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn deferred_cleanup_skips_active_target_paths() {
        let root = temp_test_root("deferred-cleanup");
        let mods_dir = root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let active_path = mods_dir.join("example.jar");
        let stale_path = mods_dir.join("old-example.jar");
        let staged_path = mods_dir.join(".vertex-update-backup-example.jar");
        std::fs::write(active_path.as_path(), b"new").expect("write active mod");
        std::fs::write(stale_path.as_path(), b"old").expect("write stale mod");
        std::fs::write(staged_path.as_path(), b"backup").expect("write staged backup");

        let mut manifest = ContentInstallManifest::default();
        manifest.projects.insert(
            "mod::example".to_owned(),
            InstalledContentProject {
                project_key: "mod::example".to_owned(),
                name: "Example".to_owned(),
                folder_name: "mods".to_owned(),
                file_path: PathBuf::from("mods/example.jar"),
                modrinth_project_id: None,
                curseforge_project_id: None,
                selected_source: None,
                selected_version_id: None,
                selected_version_name: None,
                pack_managed: false,
                explicitly_installed: true,
                direct_dependencies: Vec::new(),
            },
        );

        let cleanup = DeferredContentCleanup {
            stale_paths: vec![active_path.clone(), stale_path.clone()],
            staged_paths: vec![staged_path.clone()],
        };
        let mut removed_files = Vec::new();
        apply_deferred_content_cleanup(root.as_path(), &manifest, &cleanup, &mut removed_files)
            .expect("apply deferred cleanup");

        assert!(active_path.exists(), "active file should not be deleted");
        assert!(!stale_path.exists(), "stale file should be deleted");
        assert!(!staged_path.exists(), "staged backup should be deleted");
        assert_eq!(
            removed_files,
            vec![
                staged_path.display().to_string(),
                stale_path.display().to_string()
            ]
        );

        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn parse_content_type_accepts_modrinth_resourcepack_slug() {
        assert_eq!(
            parse_content_type("resourcepack"),
            Some(BrowserContentType::ResourcePack)
        );
        assert_eq!(
            parse_content_type("texturepack"),
            Some(BrowserContentType::ResourcePack)
        );
    }

    #[test]
    fn resolve_installed_file_paths_prefers_manifest_managed_path() {
        let root = temp_test_root("managed-update-path");
        let mods_dir = root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let managed_path = mods_dir.join("example-2.0.jar");
        let stale_path = mods_dir.join("example-1.0.jar");
        std::fs::write(managed_path.as_path(), b"managed").expect("write managed mod");
        std::fs::write(stale_path.as_path(), b"stale").expect("write stale mod");

        let mut manifest = ContentInstallManifest::default();
        manifest.projects.insert(
            "mod::example".to_owned(),
            InstalledContentProject {
                project_key: "mod::example".to_owned(),
                name: "Example".to_owned(),
                folder_name: "mods".to_owned(),
                file_path: PathBuf::from("mods/example-2.0.jar"),
                modrinth_project_id: Some("example-project".to_owned()),
                curseforge_project_id: None,
                selected_source: Some(ManagedContentSource::Modrinth),
                selected_version_id: Some("version-2".to_owned()),
                selected_version_name: Some("2.0".to_owned()),
                pack_managed: false,
                explicitly_installed: true,
                direct_dependencies: Vec::new(),
            },
        );
        let entry = BrowserProjectEntry {
            dedupe_key: "mod::example".to_owned(),
            name: "Example".to_owned(),
            summary: String::new(),
            content_type: BrowserContentType::Mod,
            icon_url: None,
            modrinth_project_id: Some("example-project".to_owned()),
            curseforge_project_id: None,
            sources: vec![ContentSource::Modrinth],
            popularity_score: None,
            updated_at: None,
            relevance_rank: 0,
        };

        let (effective_path, stale_path_to_remove) = resolve_installed_file_paths_for_update(
            root.as_path(),
            &manifest,
            &entry,
            stale_path.as_path(),
        );

        assert_eq!(effective_path, managed_path);
        assert_eq!(stale_path_to_remove, Some(stale_path.clone()));

        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn prefetched_target_uses_vertex_prefetch_tree() {
        let instance_root = PathBuf::from("instance-root");
        let target = PathBuf::from("resourcepacks/example-pack.zip");
        let prefetched = prefetched_target_path(
            instance_root.as_path(),
            BrowserContentType::ResourcePack,
            target.as_path(),
        );

        assert_eq!(
            prefetched,
            instance_root.join("vertex_prefetch/resourcepacks/example-pack.zip")
        );
    }
}
