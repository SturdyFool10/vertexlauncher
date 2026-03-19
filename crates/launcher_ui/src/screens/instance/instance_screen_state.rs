use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use crate::ui::components::lazy_image_bytes::LazyImageBytes;
use config::Config;
use content_resolver::{InstalledContentHashCache, InstalledContentKind, ResolvedInstalledContent};
use installation::{
    InstallProgress, LoaderSupportIndex, LoaderVersionIndex, MinecraftVersionEntry, VersionCatalog,
};
use vtmpack::VtmpackExportOptions;

use super::{
    ContentApplyResult, ContentLookupResult, INSTALLED_CONTENT_PAGE_SIZES, InstalledContentCache,
    RuntimePrepareOutcome, split_modloader,
};

#[derive(Clone, Debug)]
pub(super) struct InstalledContentEntryUiCache {
    pub(super) description_source: String,
    pub(super) description_width_bucket: u32,
    pub(super) truncated_description: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum InstanceScreenTab {
    #[default]
    Content,
    ScreenshotGallery,
    Logs,
}

impl InstanceScreenTab {
    pub(super) const ALL: [Self; 3] = [Self::Content, Self::ScreenshotGallery, Self::Logs];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Content => "Content",
            Self::ScreenshotGallery => "Screenshot Gallery",
            Self::Logs => "Logs",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct InstanceScreenshotEntry {
    pub(super) path: PathBuf,
    pub(super) file_name: String,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) modified_at_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub(super) struct InstanceScreenshotViewerState {
    pub(super) screenshot_key: String,
    pub(super) zoom: f32,
    pub(super) pan_uv: egui::Vec2,
}

#[derive(Clone, Debug)]
pub(super) struct InstanceLogEntry {
    pub(super) path: PathBuf,
    pub(super) file_name: String,
    pub(super) modified_at_ms: Option<u64>,
    pub(super) size_bytes: u64,
}

#[derive(Clone, Debug)]
pub(super) struct InstanceScreenState {
    pub(super) running: bool,
    pub(super) status_message: Option<String>,
    pub(super) name_input: String,
    pub(super) description_input: String,
    pub(super) thumbnail_input: String,
    pub(super) selected_modloader: usize,
    pub(super) custom_modloader: String,
    pub(super) game_version_input: String,
    pub(super) modloader_version_input: String,
    pub(super) memory_override_enabled: bool,
    pub(super) memory_override_mib: u128,
    pub(super) cli_args_input: String,
    pub(super) java_override_enabled: bool,
    pub(super) java_override_runtime_major: Option<u8>,
    pub(super) linux_set_opengl_driver: bool,
    pub(super) linux_use_zink_driver: bool,
    pub(super) selected_content_tab: InstalledContentKind,
    pub(super) installed_content_page_size: usize,
    pub(super) installed_content_page: usize,
    pub(super) installed_content_cache: InstalledContentCache,
    pub(super) installed_content_entry_ui_cache: HashMap<String, InstalledContentEntryUiCache>,
    pub(super) content_metadata_cache: HashMap<String, Option<ResolvedInstalledContent>>,
    pub(super) content_hash_cache: Option<InstalledContentHashCache>,
    pub(super) content_hash_cache_dirty: bool,
    pub(super) content_hash_cache_dirty_since: Option<Instant>,
    pub(super) content_apply_in_flight: bool,
    pub(super) content_apply_results_tx: Option<mpsc::Sender<ContentApplyResult>>,
    pub(super) content_apply_results_rx: Option<Arc<Mutex<mpsc::Receiver<ContentApplyResult>>>>,
    pub(super) content_lookup_in_flight: HashSet<String>,
    pub(super) content_lookup_request_serial: u64,
    pub(super) content_lookup_latest_serial_by_key: HashMap<String, u64>,
    pub(super) content_lookup_retry_after_by_key: HashMap<String, Instant>,
    pub(super) content_lookup_failure_count_by_key: HashMap<String, u8>,
    pub(super) content_lookup_results_tx: Option<mpsc::Sender<ContentLookupResult>>,
    pub(super) content_lookup_results_rx: Option<Arc<Mutex<mpsc::Receiver<ContentLookupResult>>>>,
    pub(super) available_game_versions: Vec<MinecraftVersionEntry>,
    pub(super) selected_game_version_index: usize,
    pub(super) loader_support: LoaderSupportIndex,
    pub(super) loader_versions: LoaderVersionIndex,
    pub(super) modloader_versions_cache: BTreeMap<String, Vec<String>>,
    pub(super) modloader_versions_in_flight: HashSet<String>,
    pub(super) modloader_versions_results_tx:
        Option<mpsc::Sender<(String, Result<Vec<String>, String>)>>,
    pub(super) modloader_versions_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, Result<Vec<String>, String>)>>>>,
    pub(super) modloader_versions_status_key: Option<String>,
    pub(super) modloader_versions_status: Option<String>,
    pub(super) version_catalog_include_snapshots: Option<bool>,
    pub(super) version_catalog_error: Option<String>,
    pub(super) version_catalog_in_flight: bool,
    pub(super) version_catalog_results_tx:
        Option<mpsc::Sender<(bool, Result<VersionCatalog, String>)>>,
    pub(super) version_catalog_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(bool, Result<VersionCatalog, String>)>>>>,
    pub(super) runtime_prepare_in_flight: bool,
    pub(super) runtime_prepare_results_tx:
        Option<mpsc::Sender<(String, String, Result<RuntimePrepareOutcome, String>)>>,
    pub(super) runtime_prepare_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, String, Result<RuntimePrepareOutcome, String>)>>>>,
    pub(super) runtime_progress_tx: Option<mpsc::Sender<InstallProgress>>,
    pub(super) runtime_progress_rx: Option<Arc<Mutex<mpsc::Receiver<InstallProgress>>>>,
    pub(super) runtime_latest_progress: Option<InstallProgress>,
    pub(super) runtime_last_notification_at: Option<Instant>,
    pub(super) runtime_prepare_instance_root: Option<String>,
    pub(super) runtime_prepare_user_key: Option<String>,
    pub(super) active_tab: InstanceScreenTab,
    pub(super) screenshots: Vec<InstanceScreenshotEntry>,
    pub(super) last_screenshot_scan_at: Option<Instant>,
    pub(super) screenshot_scan_in_flight: bool,
    pub(super) screenshot_scan_request_serial: u64,
    pub(super) screenshot_scan_results_tx:
        Option<mpsc::Sender<(u64, Vec<InstanceScreenshotEntry>)>>,
    pub(super) screenshot_scan_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(u64, Vec<InstanceScreenshotEntry>)>>>>,
    pub(super) screenshot_images: LazyImageBytes,
    pub(super) screenshot_viewer: Option<InstanceScreenshotViewerState>,
    pub(super) pending_delete_screenshot_key: Option<String>,
    pub(super) delete_screenshot_in_flight: bool,
    pub(super) delete_screenshot_results_tx:
        Option<mpsc::Sender<(String, String, Result<(), String>)>>,
    pub(super) delete_screenshot_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, String, Result<(), String>)>>>>,
    pub(super) logs: Vec<InstanceLogEntry>,
    pub(super) last_log_scan_at: Option<Instant>,
    pub(super) log_scan_in_flight: bool,
    pub(super) log_scan_request_serial: u64,
    pub(super) log_scan_results_tx: Option<mpsc::Sender<(u64, Vec<InstanceLogEntry>)>>,
    pub(super) log_scan_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(u64, Vec<InstanceLogEntry>)>>>>,
    pub(super) selected_log_path: Option<PathBuf>,
    pub(super) loaded_log_path: Option<PathBuf>,
    pub(super) loaded_log_modified_at_ms: Option<u64>,
    pub(super) loaded_log_lines: Vec<String>,
    pub(super) loaded_log_error: Option<String>,
    pub(super) loaded_log_truncated: bool,
    pub(super) log_load_in_flight: bool,
    pub(super) log_load_request_serial: u64,
    pub(super) requested_log_load_path: Option<PathBuf>,
    pub(super) requested_log_load_modified_at_ms: Option<u64>,
    pub(super) log_load_results_tx: Option<
        mpsc::Sender<(
            u64,
            PathBuf,
            Option<u64>,
            Result<(Vec<String>, bool), String>,
        )>,
    >,
    pub(super) log_load_results_rx: Option<
        Arc<
            Mutex<
                mpsc::Receiver<(
                    u64,
                    PathBuf,
                    Option<u64>,
                    Result<(Vec<String>, bool), String>,
                )>,
            >,
        >,
    >,
    pub(super) show_settings_modal: bool,
    pub(super) show_export_vtmpack_modal: bool,
    pub(super) export_vtmpack_options: VtmpackExportOptions,
    pub(super) launch_username: Option<String>,
    pub(super) launch_user_key: Option<String>,
}

impl InstanceScreenState {
    pub(super) fn from_instance(instance: &instances::InstanceRecord, config: &Config) -> Self {
        let (selected_modloader, custom_modloader) = split_modloader(&instance.modloader);
        let linux_set_opengl_driver = instances::linux_graphics_override_enabled(instance);
        let (_, linux_use_zink_driver) = instances::effective_linux_graphics_settings(
            instance,
            config.linux_set_opengl_driver(),
            config.linux_use_zink_driver(),
        );
        Self {
            running: false,
            status_message: None,
            name_input: instance.name.clone(),
            description_input: instance.description.clone().unwrap_or_default(),
            thumbnail_input: instance.thumbnail_path.clone().unwrap_or_default(),
            selected_modloader,
            custom_modloader,
            game_version_input: instance.game_version.clone(),
            modloader_version_input: instance.modloader_version.clone(),
            memory_override_enabled: instance.max_memory_mib.is_some(),
            memory_override_mib: instance
                .max_memory_mib
                .unwrap_or(config.default_instance_max_memory_mib()),
            cli_args_input: instance
                .cli_args
                .clone()
                .unwrap_or_else(|| config.default_instance_cli_args().to_owned()),
            java_override_enabled: instance.java_override_enabled,
            java_override_runtime_major: instance.java_override_runtime_major,
            linux_set_opengl_driver,
            linux_use_zink_driver,
            selected_content_tab: InstalledContentKind::Mods,
            installed_content_page_size: INSTALLED_CONTENT_PAGE_SIZES[1],
            installed_content_page: 1,
            installed_content_cache: InstalledContentCache::default(),
            installed_content_entry_ui_cache: HashMap::new(),
            content_metadata_cache: HashMap::new(),
            content_hash_cache: None,
            content_hash_cache_dirty: false,
            content_hash_cache_dirty_since: None,
            content_apply_in_flight: false,
            content_apply_results_tx: None,
            content_apply_results_rx: None,
            content_lookup_in_flight: HashSet::new(),
            content_lookup_request_serial: 0,
            content_lookup_latest_serial_by_key: HashMap::new(),
            content_lookup_retry_after_by_key: HashMap::new(),
            content_lookup_failure_count_by_key: HashMap::new(),
            content_lookup_results_tx: None,
            content_lookup_results_rx: None,
            available_game_versions: Vec::new(),
            selected_game_version_index: 0,
            loader_support: LoaderSupportIndex::default(),
            loader_versions: LoaderVersionIndex::default(),
            modloader_versions_cache: BTreeMap::new(),
            modloader_versions_in_flight: HashSet::new(),
            modloader_versions_results_tx: None,
            modloader_versions_results_rx: None,
            modloader_versions_status_key: None,
            modloader_versions_status: None,
            version_catalog_include_snapshots: None,
            version_catalog_error: None,
            version_catalog_in_flight: false,
            version_catalog_results_tx: None,
            version_catalog_results_rx: None,
            runtime_prepare_in_flight: false,
            runtime_prepare_results_tx: None,
            runtime_prepare_results_rx: None,
            runtime_progress_tx: None,
            runtime_progress_rx: None,
            runtime_latest_progress: None,
            runtime_last_notification_at: None,
            runtime_prepare_instance_root: None,
            runtime_prepare_user_key: None,
            active_tab: InstanceScreenTab::Content,
            screenshots: Vec::new(),
            last_screenshot_scan_at: None,
            screenshot_scan_in_flight: false,
            screenshot_scan_request_serial: 0,
            screenshot_scan_results_tx: None,
            screenshot_scan_results_rx: None,
            screenshot_images: LazyImageBytes::default(),
            screenshot_viewer: None,
            pending_delete_screenshot_key: None,
            delete_screenshot_in_flight: false,
            delete_screenshot_results_tx: None,
            delete_screenshot_results_rx: None,
            logs: Vec::new(),
            last_log_scan_at: None,
            log_scan_in_flight: false,
            log_scan_request_serial: 0,
            log_scan_results_tx: None,
            log_scan_results_rx: None,
            selected_log_path: None,
            loaded_log_path: None,
            loaded_log_modified_at_ms: None,
            loaded_log_lines: Vec::new(),
            loaded_log_error: None,
            loaded_log_truncated: false,
            log_load_in_flight: false,
            log_load_request_serial: 0,
            requested_log_load_path: None,
            requested_log_load_modified_at_ms: None,
            log_load_results_tx: None,
            log_load_results_rx: None,
            show_settings_modal: false,
            show_export_vtmpack_modal: false,
            export_vtmpack_options: VtmpackExportOptions::default(),
            launch_username: None,
            launch_user_key: None,
        }
    }

    pub(super) fn invalidate_installed_content_cache(&mut self) {
        self.installed_content_cache.clear();
        self.installed_content_entry_ui_cache.clear();
    }
}
