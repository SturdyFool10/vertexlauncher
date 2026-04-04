use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Read, Write};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(target_os = "windows")]
use vertex_constants::installation::CREATE_NO_WINDOW;
use vertex_constants::installation::{
    CACHE_LOADER_VERSIONS_DIR_NAME, CACHE_VERSION_CATALOG_ALL_FILE,
    CACHE_VERSION_CATALOG_RELEASES_FILE, FABRIC_GAME_VERSIONS_URL, FABRIC_VERSION_MATRIX_URL,
    FORGE_MAVEN_METADATA_URL, HTTP_RETRY_ATTEMPTS, HTTP_RETRY_BASE_DELAY_MS, HTTP_TIMEOUT_CONNECT,
    HTTP_TIMEOUT_GLOBAL, HTTP_TIMEOUT_RECV_BODY, HTTP_TIMEOUT_RECV_RESPONSE,
    MAX_CONTENT_LENGTH_PROBES_PER_BATCH, MOJANG_VERSION_MANIFEST_URL,
    NEOFORGE_LEGACY_FORGE_METADATA_URL, NEOFORGE_MAVEN_METADATA_URL, OPENJDK_USER_AGENT,
    QUILT_GAME_VERSIONS_URL, QUILT_VERSION_MATRIX_URL, USER_AGENT as DEFAULT_USER_AGENT,
    VERSION_CATALOG_CACHE_TTL,
};

pub fn display_user_path(path: &Path) -> String {
    #[cfg(target_os = "windows")]
    {
        return normalize_windows_cli_path(path.as_os_str().to_string_lossy().as_ref());
    }

    #[cfg(not(target_os = "windows"))]
    {
        path.as_os_str().to_string_lossy().into_owned()
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn normalize_windows_cli_path(raw: &str) -> String {
    if let Some(stripped) = raw.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{stripped}");
    }
    if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        return stripped.to_owned();
    }
    raw.to_owned()
}

fn normalize_child_process_path(path: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        return PathBuf::from(normalize_windows_cli_path(
            path.as_os_str().to_string_lossy().as_ref(),
        ));
    }

    #[cfg(not(target_os = "windows"))]
    {
        path.to_path_buf()
    }
}

pub fn normalize_path_key(path: &Path) -> String {
    let normalized = fs_canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    display_user_path(normalized.as_path())
}

#[track_caller]
fn fs_create_dir_all(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display());
    let result = fs::create_dir_all(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_remove_dir_all(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "remove_dir_all", path = %path.display());
    let result = fs::remove_dir_all(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "remove_dir_all", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_read_to_string(path: impl AsRef<Path>) -> std::io::Result<String> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    let result = fs::read_to_string(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_read_dir(path: impl AsRef<Path>) -> std::io::Result<fs::ReadDir> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read_dir", path = %path.display());
    let result = fs::read_dir(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_dir", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> std::io::Result<()> {
    let from = from.as_ref();
    let to = to.as_ref();
    tracing::debug!(
        target: "vertexlauncher/io",
        op = "rename",
        from = %from.display(),
        to = %to.display()
    );
    let result = fs::rename(from, to);
    if let Err(err) = &result {
        tracing::warn!(
            target: "vertexlauncher/io",
            op = "rename",
            from = %from.display(),
            to = %to.display(),
            error = %err
        );
    }
    result
}

#[track_caller]
fn fs_canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "canonicalize", path = %path.display());
    let result = fs::canonicalize(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "canonicalize", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "write", path = %path.display());
    let result = fs::write(path, contents);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "write", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_file_create(path: impl AsRef<Path>) -> std::io::Result<fs::File> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "file_create", path = %path.display());
    let result = fs::File::create(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "file_create", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_file_open(path: impl AsRef<Path>) -> std::io::Result<fs::File> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "file_open", path = %path.display());
    let result = fs::File::open(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "file_open", path = %path.display(), error = %err);
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MinecraftVersionType {
    Release,
    Snapshot,
    OldBeta,
    OldAlpha,
    Unknown,
}

impl MinecraftVersionType {
    pub fn label(self) -> &'static str {
        match self {
            MinecraftVersionType::Release => "Release",
            MinecraftVersionType::Snapshot => "Snapshot",
            MinecraftVersionType::OldBeta => "Old Beta",
            MinecraftVersionType::OldAlpha => "Old Alpha",
            MinecraftVersionType::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinecraftVersionEntry {
    pub id: String,
    pub version_type: MinecraftVersionType,
}

impl MinecraftVersionEntry {
    pub fn display_label(&self) -> String {
        format!("{} ({})", self.id, self.version_type.label())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoaderSupportIndex {
    pub fabric: HashSet<String>,
    pub forge: HashSet<String>,
    pub neoforge: HashSet<String>,
    pub quilt: HashSet<String>,
}

impl LoaderSupportIndex {
    pub fn supports_loader(&self, loader_label: &str, game_version: &str) -> bool {
        match normalized_loader_label(loader_label) {
            LoaderKind::Vanilla => true,
            LoaderKind::Fabric => self.fabric.contains(game_version),
            LoaderKind::Forge => self.forge.contains(game_version),
            LoaderKind::NeoForge => self.neoforge.contains(game_version),
            LoaderKind::Quilt => self.quilt.contains(game_version),
            LoaderKind::Custom => true,
        }
    }

    pub fn unavailable_reason(&self, loader_label: &str, game_version: &str) -> Option<String> {
        if self.supports_loader(loader_label, game_version) {
            None
        } else {
            Some(format!(
                "{loader_label} is not available for Minecraft {game_version}"
            ))
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoaderVersionIndex {
    pub fabric: BTreeMap<String, Vec<String>>,
    pub forge: BTreeMap<String, Vec<String>>,
    pub neoforge: BTreeMap<String, Vec<String>>,
    pub quilt: BTreeMap<String, Vec<String>>,
}

impl LoaderVersionIndex {
    pub fn versions_for_loader(&self, loader_label: &str, game_version: &str) -> Option<&[String]> {
        match normalized_loader_label(loader_label) {
            LoaderKind::Fabric => self.fabric.get(game_version).map(Vec::as_slice),
            LoaderKind::Forge => self.forge.get(game_version).map(Vec::as_slice),
            LoaderKind::NeoForge => self.neoforge.get(game_version).map(Vec::as_slice),
            LoaderKind::Quilt => self.quilt.get(game_version).map(Vec::as_slice),
            LoaderKind::Vanilla | LoaderKind::Custom => None,
        }
    }

    fn sort_desc(&mut self) {
        sort_loader_version_map_desc(&mut self.fabric);
        sort_loader_version_map_desc(&mut self.forge);
        sort_loader_version_map_desc(&mut self.neoforge);
        sort_loader_version_map_desc(&mut self.quilt);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionCatalog {
    pub game_versions: Vec<MinecraftVersionEntry>,
    pub loader_support: LoaderSupportIndex,
    #[serde(default)]
    pub loader_versions: LoaderVersionIndex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameSetupResult {
    pub version_json_path: PathBuf,
    pub client_jar_path: PathBuf,
    pub downloaded_files: u32,
    pub resolved_modloader_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    pub instance_root: PathBuf,
    pub game_version: String,
    pub modloader: String,
    pub modloader_version: Option<String>,
    pub account_key: Option<String>,
    pub java_executable: Option<String>,
    pub max_memory_mib: u128,
    pub extra_jvm_args: Option<String>,
    pub player_name: Option<String>,
    pub player_uuid: Option<String>,
    pub auth_access_token: Option<String>,
    pub auth_xuid: Option<String>,
    pub auth_user_type: Option<String>,
    pub quick_play_singleplayer: Option<String>,
    pub quick_play_multiplayer: Option<String>,
    pub linux_set_opengl_driver: bool,
    pub linux_use_zink_driver: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchResult {
    pub pid: u32,
    pub profile_id: String,
    pub launch_log_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishedInstanceProcess {
    pub instance_root: String,
    pub account_key: Option<String>,
    pub pid: u32,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadPolicy {
    pub max_concurrent_downloads: u32,
    pub max_download_bps: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadBatchTask {
    pub url: String,
    pub destination: PathBuf,
    pub expected_size: Option<u64>,
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: 8,
            max_download_bps: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallStage {
    PreparingFolders,
    ResolvingMetadata,
    DownloadingCore,
    InstallingModloader,
    Complete,
}

#[derive(Clone, Debug)]
pub struct InstallProgress {
    pub stage: InstallStage,
    pub message: String,
    pub downloaded_files: u32,
    pub total_files: u32,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub bytes_per_second: f64,
    pub eta_seconds: Option<u64>,
}

pub type InstallProgressCallback = Arc<dyn Fn(InstallProgress) + Send + Sync + 'static>;
type InstallProgressSink = dyn Fn(InstallProgress) + Send + Sync + 'static;

pub fn download_batch(
    tasks: Vec<DownloadBatchTask>,
    policy: &DownloadPolicy,
) -> Result<u32, InstallationError> {
    download_batch_with_progress(tasks, policy, InstallStage::DownloadingCore, None)
}

pub fn download_batch_with_progress(
    tasks: Vec<DownloadBatchTask>,
    policy: &DownloadPolicy,
    stage: InstallStage,
    progress: Option<&InstallProgressCallback>,
) -> Result<u32, InstallationError> {
    let tasks = tasks
        .into_iter()
        .map(|task| FileDownloadTask {
            url: task.url,
            destination: task.destination,
            expected_size: task.expected_size,
        })
        .collect();
    download_files_concurrent(stage, tasks, policy, 0, progress.map(Arc::as_ref))
}

#[derive(Debug, thiserror::Error)]
pub enum InstallationError {
    #[error("HTTP status {status} for {url}: {body}")]
    HttpStatus {
        url: String,
        status: u16,
        body: String,
    },
    #[error("HTTP transport error while requesting {url}: {message}")]
    Transport { url: String, message: String },
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "Java executable was not found: {executable}. Configure a valid Java path or install Java."
    )]
    JavaExecutableNotFound { executable: String },
    #[error("Minecraft version '{0}' was not found in Mojang manifest")]
    UnknownMinecraftVersion(String),
    #[error("Version metadata for '{0}' is missing client download information")]
    MissingClientDownload(String),
    #[error("No modloader version was provided for {loader} on Minecraft {game_version}")]
    MissingModloaderVersion {
        loader: String,
        game_version: String,
    },
    #[error("Java runtime is required to install {loader} but was not configured")]
    MissingJavaRuntime { loader: String },
    #[error(
        "{loader} installer failed for Minecraft {game_version} ({loader_version}); command: {command}; status: {status}; stderr: {stderr}"
    )]
    ModloaderInstallerFailed {
        loader: String,
        game_version: String,
        loader_version: String,
        command: String,
        status: String,
        stderr: String,
    },
    #[error(
        "{loader} installer did not produce a usable version profile for Minecraft {game_version} ({loader_version}) in {}",
        .versions_dir.display()
    )]
    ModloaderInstallOutputMissing {
        loader: String,
        game_version: String,
        loader_version: String,
        versions_dir: PathBuf,
    },
    #[error("OpenJDK provisioning is not supported on this platform ({0})")]
    UnsupportedPlatform(String),
    #[error("Could not resolve OpenJDK {runtime_major} package metadata from Adoptium API")]
    OpenJdkMetadataMissing { runtime_major: u8 },
    #[error("Could not resolve a launch profile for {modloader} on Minecraft {game_version}")]
    LaunchProfileMissing {
        modloader: String,
        game_version: String,
    },
    #[error("Launch profile {profile_id} is missing mainClass")]
    LaunchMainClassMissing { profile_id: String },
    #[error("Launch profile {profile_id} is missing required file: {}", .path.display())]
    LaunchFileMissing { profile_id: String, path: PathBuf },
    #[error("Minecraft exited immediately (status: {status}). See launch log: {}", .log_path.display())]
    LaunchExitedImmediately { status: String, log_path: PathBuf },
    #[error("Minecraft for {instance_root} is already running (pid {pid})")]
    InstanceAlreadyRunning { instance_root: String, pid: u32 },
    #[error("Account '{account}' is already running Minecraft in {instance_root}")]
    AccountAlreadyInUse {
        account: String,
        instance_root: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedVersionCatalog {
    fetched_at_unix_secs: u64,
    include_snapshots_and_betas: bool,
    catalog: VersionCatalog,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CachedLoaderVersions {
    fetched_at_unix_secs: u64,
    loader_label: String,
    versions_by_game_version: BTreeMap<String, Vec<String>>,
}

pub fn fetch_version_catalog(
    include_snapshots_and_betas: bool,
) -> Result<VersionCatalog, InstallationError> {
    fetch_version_catalog_with_refresh(include_snapshots_and_betas, false)
}

pub fn fetch_version_catalog_with_refresh(
    include_snapshots_and_betas: bool,
    force_refresh: bool,
) -> Result<VersionCatalog, InstallationError> {
    let cached = read_cached_version_catalog(include_snapshots_and_betas).ok();
    if !force_refresh
        && let Some(cached) = cached.as_ref()
        && !is_cache_expired(cached.fetched_at_unix_secs)
        && catalog_has_loader_version_data(&cached.catalog)
    {
        let mut catalog = cached.catalog.clone();
        normalize_version_catalog_ordering(&mut catalog);
        return Ok(catalog);
    }

    match fetch_version_catalog_uncached(include_snapshots_and_betas) {
        Ok(catalog) => {
            let _ = write_cached_version_catalog(include_snapshots_and_betas, &catalog);
            Ok(catalog)
        }
        Err(err) => {
            if let Some(cached) = cached {
                let mut catalog = cached.catalog;
                normalize_version_catalog_ordering(&mut catalog);
                Ok(catalog)
            } else {
                Err(err)
            }
        }
    }
}

pub fn ensure_openjdk_runtime(runtime_major: u8) -> Result<PathBuf, InstallationError> {
    let (os, arch, arch_cache_key) = platform_for_adoptium()?;
    let install_root = cache_root_dir()
        .join("java")
        .join(format!("openjdk-{runtime_major}-{arch_cache_key}"));
    if let Some(existing) = find_java_executable_under(install_root.as_path())? {
        return Ok(canonicalize_existing_path(existing));
    }

    fs_create_dir_all(install_root.parent().unwrap_or_else(|| Path::new(".")))?;
    if install_root.exists() {
        fs_remove_dir_all(&install_root)?;
    }
    fs_create_dir_all(&install_root)?;

    let metadata_url = format!(
        "https://api.adoptium.net/v3/assets/latest/{runtime_major}/hotspot?architecture={arch}&image_type=jdk&jvm_impl=hotspot&os={os}&vendor=eclipse"
    );
    let metadata: serde_json::Value =
        get_json_with_user_agent(metadata_url.as_str(), OPENJDK_USER_AGENT)?;
    let (package_url, package_name) = extract_adoptium_package(&metadata)
        .ok_or(InstallationError::OpenJdkMetadataMissing { runtime_major })?;

    let downloads_dir = cache_root_dir().join("downloads");
    fs_create_dir_all(&downloads_dir)?;
    let archive_path = downloads_dir.join(package_name.as_str());
    download_file_simple(package_url.as_str(), archive_path.as_path())?;
    extract_archive(archive_path.as_path(), install_root.as_path())?;

    let installed = find_java_executable_under(install_root.as_path())?
        .ok_or(InstallationError::OpenJdkMetadataMissing { runtime_major })?;
    Ok(canonicalize_existing_path(installed))
}

pub fn purge_cache() -> Result<(), InstallationError> {
    let cache_root = cache_root_dir();
    match fs_remove_dir_all(&cache_root) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(InstallationError::Io(err)),
    }
}

pub fn fetch_loader_versions_for_game(
    loader_label: &str,
    game_version: &str,
    force_refresh: bool,
) -> Result<Vec<String>, InstallationError> {
    let game_version = game_version.trim();
    if game_version.is_empty() {
        return Ok(Vec::new());
    }

    let loader_kind = normalized_loader_label(loader_label);
    if matches!(loader_kind, LoaderKind::Vanilla | LoaderKind::Custom) {
        return Ok(Vec::new());
    }

    let cached = read_cached_loader_versions(loader_kind).ok();
    if !force_refresh
        && let Some(cached) = cached.as_ref()
        && !is_cache_expired(cached.fetched_at_unix_secs)
        && let Some(versions) = cached.versions_by_game_version.get(game_version)
    {
        return Ok(sort_loader_versions_desc(versions.clone()));
    }

    match fetch_loader_versions_for_game_uncached(loader_kind, game_version) {
        Ok(result) => {
            let LoaderVersionFetchResult {
                selected_versions,
                versions_by_game_version,
            } = result;
            let mut updated_cache = cached.unwrap_or_default();
            updated_cache.fetched_at_unix_secs = now_unix_secs();
            updated_cache.loader_label = loader_label.to_owned();
            if versions_by_game_version.is_empty() {
                updated_cache
                    .versions_by_game_version
                    .insert(game_version.to_owned(), selected_versions.clone());
            } else {
                updated_cache
                    .versions_by_game_version
                    .extend(versions_by_game_version);
                updated_cache
                    .versions_by_game_version
                    .entry(game_version.to_owned())
                    .or_insert_with(|| selected_versions.clone());
            }
            sort_loader_version_map_desc(&mut updated_cache.versions_by_game_version);
            let _ = write_cached_loader_versions(loader_kind, &updated_cache);
            Ok(sort_loader_versions_desc(selected_versions))
        }
        Err(err) => {
            if let Some(cached) = cached
                && let Some(versions) = cached.versions_by_game_version.get(game_version)
            {
                Ok(sort_loader_versions_desc(versions.clone()))
            } else {
                Err(err)
            }
        }
    }
}

fn fetch_version_catalog_uncached(
    include_snapshots_and_betas: bool,
) -> Result<VersionCatalog, InstallationError> {
    let (manifest, fabric, forge, neoforge, quilt) = thread::scope(|scope| {
        let manifest_task =
            scope.spawn(|| get_json::<MojangVersionManifest>(MOJANG_VERSION_MANIFEST_URL));
        let fabric_task = scope.spawn(fetch_fabric_loader_catalog_with_fallback);
        let forge_task = scope.spawn(fetch_forge_loader_catalog_with_fallback);
        let neoforge_task = scope.spawn(fetch_neoforge_loader_catalog_with_fallback);
        let quilt_task = scope.spawn(fetch_quilt_loader_catalog_with_fallback);

        let manifest = manifest_task.join().map_err(|_| {
            InstallationError::Io(std::io::Error::new(
                ErrorKind::Other,
                "minecraft version manifest task panicked",
            ))
        })??;
        let fabric = fabric_task.join().map_err(|_| {
            InstallationError::Io(std::io::Error::other("fabric loader catalog task panicked"))
        })?;
        let forge = forge_task.join().map_err(|_| {
            InstallationError::Io(std::io::Error::other("forge loader catalog task panicked"))
        })?;
        let neoforge = neoforge_task.join().map_err(|_| {
            InstallationError::Io(std::io::Error::other(
                "neoforge loader catalog task panicked",
            ))
        })?;
        let quilt = quilt_task.join().map_err(|_| {
            InstallationError::Io(std::io::Error::other("quilt loader catalog task panicked"))
        })?;
        Ok::<_, InstallationError>((manifest, fabric, forge, neoforge, quilt))
    })?;

    let mut game_versions_with_release_time: Vec<(String, MinecraftVersionEntry)> = manifest
        .versions
        .into_iter()
        .filter_map(|entry| {
            let version_type = map_version_type(entry.version_type.as_str());
            let include = match version_type {
                MinecraftVersionType::Release => true,
                MinecraftVersionType::Snapshot
                | MinecraftVersionType::OldBeta
                | MinecraftVersionType::OldAlpha => include_snapshots_and_betas,
                MinecraftVersionType::Unknown => include_snapshots_and_betas,
            };
            if include {
                Some((
                    entry.release_time,
                    MinecraftVersionEntry {
                        id: entry.id,
                        version_type,
                    },
                ))
            } else {
                None
            }
        })
        .collect();
    game_versions_with_release_time.sort_by(|left, right| right.0.cmp(&left.0));
    let game_versions: Vec<MinecraftVersionEntry> = game_versions_with_release_time
        .into_iter()
        .map(|(_, entry)| entry)
        .collect();

    let loader_support = LoaderSupportIndex {
        fabric: fabric.supported_game_versions,
        forge: forge.supported_game_versions,
        neoforge: neoforge.supported_game_versions,
        quilt: quilt.supported_game_versions,
    };
    let mut loader_versions = LoaderVersionIndex {
        fabric: fabric.versions_by_game_version,
        forge: forge.versions_by_game_version,
        neoforge: neoforge.versions_by_game_version,
        quilt: quilt.versions_by_game_version,
    };
    loader_versions.sort_desc();

    Ok(VersionCatalog {
        game_versions,
        loader_support,
        loader_versions,
    })
}

pub fn ensure_game_files(
    instance_root: &Path,
    game_version: &str,
    modloader: &str,
    modloader_version: Option<&str>,
    java_executable: Option<&str>,
    download_policy: &DownloadPolicy,
    progress: Option<InstallProgressCallback>,
) -> Result<GameSetupResult, InstallationError> {
    let game_version = game_version.trim();
    if game_version.is_empty() {
        return Err(InstallationError::UnknownMinecraftVersion(String::new()));
    }
    tracing::info!(
        target: "vertexlauncher/installation/process",
        instance_root = %instance_root.display(),
        game_version = %game_version,
        modloader = %modloader,
        requested_modloader_version = %modloader_version.unwrap_or(""),
        "Starting ensure_game_files."
    );

    let versions_dir = instance_root.join("versions").join(game_version);
    fs_create_dir_all(&versions_dir)?;
    let version_json_path = versions_dir.join(format!("{game_version}.json"));
    let client_jar_path = versions_dir.join(format!("{game_version}.jar"));
    fs_create_dir_all(instance_root.join("mods"))?;
    fs_create_dir_all(instance_root.join("assets"))?;
    fs_create_dir_all(instance_root.join("libraries"))?;
    fs_create_dir_all(instance_root.join("resourcepacks"))?;
    fs_create_dir_all(instance_root.join("shaderpacks"))?;
    report_install_progress(
        progress.as_deref(),
        InstallProgress {
            stage: InstallStage::PreparingFolders,
            message: format!("Prepared instance folders for Minecraft {game_version}."),
            downloaded_files: 0,
            total_files: 0,
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: None,
        },
    );
    tracing::info!(
        target: "vertexlauncher/installation/process",
        instance_root = %instance_root.display(),
        game_version = %game_version,
        "Prepared instance folders."
    );

    let mut downloaded_files = 0;

    if !version_json_path.exists() || !client_jar_path.exists() {
        tracing::info!(
            target: "vertexlauncher/installation/process",
            game_version = %game_version,
            "Core version files missing; resolving metadata and scheduling core downloads."
        );
        report_install_progress(
            progress.as_deref(),
            InstallProgress {
                stage: InstallStage::ResolvingMetadata,
                message: format!("Resolving Minecraft {game_version} metadata..."),
                downloaded_files,
                total_files: 0,
                downloaded_bytes: 0,
                total_bytes: None,
                bytes_per_second: 0.0,
                eta_seconds: None,
            },
        );
        let manifest: MojangVersionManifest = get_json(MOJANG_VERSION_MANIFEST_URL)?;
        let version_entry = manifest
            .versions
            .into_iter()
            .find(|entry| entry.id == game_version)
            .ok_or_else(|| InstallationError::UnknownMinecraftVersion(game_version.to_owned()))?;

        let version_meta: MojangVersionMeta = get_json(&version_entry.url)?;
        let client_download = version_meta
            .downloads
            .and_then(|downloads| downloads.client)
            .ok_or_else(|| InstallationError::MissingClientDownload(game_version.to_owned()))?;

        let mut tasks = Vec::new();
        if !version_json_path.exists() {
            tasks.push(FileDownloadTask {
                url: version_entry.url,
                destination: version_json_path.clone(),
                expected_size: None,
            });
        }
        if !client_jar_path.exists() {
            tasks.push(FileDownloadTask {
                url: client_download.url,
                destination: client_jar_path.clone(),
                expected_size: client_download.size,
            });
        }
        downloaded_files += download_files_concurrent(
            InstallStage::DownloadingCore,
            tasks,
            download_policy,
            downloaded_files,
            progress.as_deref(),
        )
        .map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/process",
                instance_root = %instance_root.display(),
                game_version = %game_version,
                version_json_path = %version_json_path.display(),
                client_jar_path = %client_jar_path.display(),
                error = %err,
                "Core file download batch failed during ensure_game_files."
            );
            err
        })?;
        tracing::info!(
            target: "vertexlauncher/installation/process",
            game_version = %game_version,
            downloaded_files,
            "Core file download batch completed."
        );
    }
    downloaded_files += download_version_dependencies(
        instance_root,
        version_json_path.as_path(),
        download_policy,
        downloaded_files,
        progress.as_deref(),
    )
    .map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/process",
            instance_root = %instance_root.display(),
            game_version = %game_version,
            version_json_path = %version_json_path.display(),
            error = %err,
            "Version dependency download phase failed during ensure_game_files."
        );
        err
    })?;
    tracing::info!(
        target: "vertexlauncher/installation/process",
        game_version = %game_version,
        downloaded_files,
        "Version dependency download phase completed."
    );

    let resolved_modloader_version = install_selected_modloader(
        instance_root,
        game_version,
        modloader,
        modloader_version,
        java_executable,
        download_policy,
        &mut downloaded_files,
        progress.as_deref(),
    )
    .map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/process",
            instance_root = %instance_root.display(),
            game_version = %game_version,
            modloader = %modloader,
            requested_modloader_version = %modloader_version.unwrap_or(""),
            error = %err,
            "Modloader installation phase failed during ensure_game_files."
        );
        err
    })?;
    tracing::info!(
        target: "vertexlauncher/installation/process",
        game_version = %game_version,
        modloader = %modloader,
        resolved_modloader_version = %resolved_modloader_version.as_deref().unwrap_or(""),
        downloaded_files,
        "Modloader installation phase completed."
    );
    if let Some(loader_version) = resolved_modloader_version.as_deref() {
        let loader_kind = normalized_loader_label(modloader);
        if matches!(loader_kind, LoaderKind::Fabric | LoaderKind::Quilt) {
            let id_prefix = if loader_kind == LoaderKind::Fabric {
                "fabric-loader"
            } else {
                "quilt-loader"
            };
            let version_id = format!("{id_prefix}-{loader_version}-{game_version}");
            let loader_profile_path = instance_root
                .join("versions")
                .join(version_id.as_str())
                .join(format!("{version_id}.json"));
            downloaded_files += download_version_dependencies(
                instance_root,
                loader_profile_path.as_path(),
                download_policy,
                downloaded_files,
                progress.as_deref(),
            )
            .map_err(|err| {
                tracing::error!(
                    target: "vertexlauncher/installation/process",
                    instance_root = %instance_root.display(),
                    game_version = %game_version,
                    loader_profile_path = %loader_profile_path.display(),
                    error = %err,
                    "Loader profile dependency download phase failed during ensure_game_files."
                );
                err
            })?;
        }
    }

    report_install_progress(
        progress.as_deref(),
        InstallProgress {
            stage: InstallStage::Complete,
            message: format!("Installation prepared for Minecraft {game_version}."),
            downloaded_files,
            total_files: downloaded_files.max(1),
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: Some(0),
        },
    );
    tracing::info!(
        target: "vertexlauncher/installation/process",
        instance_root = %instance_root.display(),
        game_version = %game_version,
        final_downloaded_files = downloaded_files,
        resolved_modloader_version = %resolved_modloader_version.as_deref().unwrap_or(""),
        "ensure_game_files completed successfully."
    );
    Ok(GameSetupResult {
        version_json_path,
        client_jar_path,
        downloaded_files,
        resolved_modloader_version,
    })
}

struct RunningInstanceProcess {
    child: Child,
    account_key: Option<String>,
}

static RUNNING_INSTANCE_PROCESSES: OnceLock<Mutex<HashMap<String, Vec<RunningInstanceProcess>>>> =
    OnceLock::new();
static FINISHED_INSTANCE_PROCESSES: OnceLock<Mutex<Vec<FinishedInstanceProcess>>> = OnceLock::new();

fn process_registry() -> &'static Mutex<HashMap<String, Vec<RunningInstanceProcess>>> {
    RUNNING_INSTANCE_PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn finished_process_queue() -> &'static Mutex<Vec<FinishedInstanceProcess>> {
    FINISHED_INSTANCE_PROCESSES.get_or_init(|| Mutex::new(Vec::new()))
}

fn push_finished_instance_process(process: FinishedInstanceProcess) {
    if let Ok(mut finished) = finished_process_queue().lock() {
        finished.push(process);
    }
}

fn finished_instance_process(
    instance_root: &str,
    process: RunningInstanceProcess,
    status: ExitStatus,
) -> FinishedInstanceProcess {
    let pid = process.child.id();
    FinishedInstanceProcess {
        instance_root: instance_root.to_owned(),
        account_key: process.account_key,
        pid,
        exit_code: status.code(),
    }
}

pub fn take_finished_instance_processes() -> Vec<FinishedInstanceProcess> {
    if let Ok(mut processes) = process_registry().lock() {
        prune_finished_processes(&mut processes);
    }
    match finished_process_queue().lock() {
        Ok(mut finished) => std::mem::take(&mut *finished),
        Err(_) => Vec::new(),
    }
}

pub fn launch_instance(request: &LaunchRequest) -> Result<LaunchResult, InstallationError> {
    let instance_root = fs_canonicalize(request.instance_root.as_path())
        .unwrap_or_else(|_| request.instance_root.clone());
    let instance_key = normalize_path_key(instance_root.as_path());
    let requested_account = normalize_account_key(request.account_key.as_deref());
    if let Ok(mut processes) = process_registry().lock() {
        prune_finished_processes(&mut processes);
        if let Some(instance_processes) = processes.get_mut(instance_key.as_str()) {
            let same_account_already_running = instance_processes.iter_mut().find_map(|process| {
                if !matches!(process.child.try_wait(), Ok(None)) {
                    return None;
                }
                let matches_account = match requested_account.as_deref() {
                    Some(account) => process
                        .account_key
                        .as_deref()
                        .is_some_and(|running| running == account),
                    None => process.account_key.is_none(),
                };
                if matches_account {
                    Some(process.child.id())
                } else {
                    None
                }
            });
            if let Some(pid) = same_account_already_running {
                return Err(InstallationError::InstanceAlreadyRunning {
                    instance_root: instance_key,
                    pid,
                });
            }
        }
        if let Some(account) = requested_account.as_deref() {
            for (running_instance_root, instance_processes) in processes.iter_mut() {
                if running_instance_root == &instance_key {
                    continue;
                }
                for process in instance_processes {
                    if process
                        .account_key
                        .as_deref()
                        .is_some_and(|in_use| in_use == account)
                        && let Ok(None) = process.child.try_wait()
                    {
                        return Err(InstallationError::AccountAlreadyInUse {
                            account: request
                                .account_key
                                .clone()
                                .unwrap_or_else(|| account.to_owned()),
                            instance_root: running_instance_root.clone(),
                        });
                    }
                }
            }
        }
    }

    let java = normalize_java_executable(request.java_executable.as_deref());
    let (profile_id, profile_path) = resolve_launch_profile_path(
        instance_root.as_path(),
        request.game_version.as_str(),
        request.modloader.as_str(),
        request.modloader_version.as_deref(),
    )?;
    let profile_chain = load_profile_chain(instance_root.as_path(), profile_path.as_path())?;
    let main_class = resolve_main_class(&profile_chain).ok_or_else(|| {
        InstallationError::LaunchMainClassMissing {
            profile_id: profile_id.clone(),
        }
    })?;
    let natives_dir =
        prepare_natives_dir(instance_root.as_path(), profile_id.as_str(), &profile_chain)?;
    let classpath_entries = build_classpath_entries(
        instance_root.as_path(),
        profile_id.as_str(),
        request.game_version.as_str(),
        main_class.as_str(),
        &profile_chain,
    )?;
    let classpath = prepare_launch_classpath(
        instance_root.as_path(),
        profile_id.as_str(),
        &classpath_entries,
    )?;
    let (mut launch_log_file, launch_log_path) = prepare_launch_log_file(instance_root.as_path())?;
    let launch_log_for_error = display_user_path(launch_log_path.as_path());
    let _ = writeln!(
        launch_log_file,
        "[vertexlauncher] Launching Minecraft {} with profile {} in {}",
        request.game_version,
        profile_id,
        display_user_path(instance_root.as_path())
    );
    let stderr_log = launch_log_file.try_clone()?;
    let mut command_log = launch_log_file.try_clone()?;

    let mut command = Command::new(java.as_str());
    command
        .current_dir(instance_root.as_path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(launch_log_file))
        .stderr(Stdio::from(stderr_log));
    apply_linux_opengl_driver_env(
        &mut command,
        request.linux_set_opengl_driver,
        request.linux_use_zink_driver,
    );

    command.arg(format!("-Xmx{}M", request.max_memory_mib.max(512)));
    let user_jvm_args = parse_user_args(request.extra_jvm_args.as_deref());
    for arg in user_jvm_args {
        command.arg(arg);
    }

    let launch_context = build_launch_context(
        instance_root.as_path(),
        request.game_version.as_str(),
        profile_id.as_str(),
        resolve_assets_index_name(&profile_chain, request.game_version.as_str()),
        classpath.resolved.as_str(),
        natives_dir.as_path(),
        request.player_name.as_deref(),
        request.player_uuid.as_deref(),
        request.auth_access_token.as_deref(),
        request.auth_xuid.as_deref(),
        request.auth_user_type.as_deref(),
        request.quick_play_singleplayer.as_deref(),
        request.quick_play_multiplayer.as_deref(),
    );

    let mut jvm_args = collect_jvm_arguments(&profile_chain, &launch_context);
    if should_use_environment_classpath() {
        command.env("CLASSPATH", classpath.resolved.as_str());
        strip_explicit_classpath_args(&mut jvm_args);
    } else if let Some(argfile) = classpath.argfile.as_ref() {
        strip_explicit_classpath_args(&mut jvm_args);
        command.arg(format!("@{}", display_user_path(argfile.as_path())));
    } else if !has_explicit_classpath_args(&jvm_args) {
        jvm_args.push("-cp".to_owned());
        jvm_args.push(classpath.resolved.clone());
    }
    for arg in jvm_args {
        command.arg(arg);
    }

    command.arg(main_class);

    let game_args = collect_game_arguments(&profile_chain, &launch_context);
    for arg in game_args {
        command.arg(arg);
    }

    let raw_args: Vec<std::borrow::Cow<str>> = command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect();
    let mut command_args: Vec<String> = Vec::with_capacity(raw_args.len());
    let mut redact_next = false;
    for arg in &raw_args {
        if redact_next {
            command_args.push("[redacted]".to_owned());
            redact_next = false;
        } else {
            command_args.push(quote_command_arg(arg.as_ref()));
            if arg.as_ref() == "--accessToken" {
                redact_next = true;
            }
        }
    }
    let _ = writeln!(
        command_log,
        "[vertexlauncher] Command: {} {}",
        quote_command_arg(java.as_str()),
        command_args.join(" ")
    );

    let mut child = spawn_command_child(&mut command, java.as_str())?;
    thread::sleep(Duration::from_millis(1200));
    if let Some(status) = child.try_wait()? {
        return Err(InstallationError::LaunchExitedImmediately {
            status: status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated by signal".to_owned()),
            log_path: PathBuf::from(launch_log_for_error),
        });
    }
    let pid = child.id();
    if let Ok(mut processes) = process_registry().lock() {
        processes
            .entry(instance_key.clone())
            .or_default()
            .push(RunningInstanceProcess {
                child,
                account_key: requested_account,
            });
    }
    Ok(LaunchResult {
        pid,
        profile_id,
        launch_log_path,
    })
}

#[cfg(target_os = "linux")]
fn apply_linux_opengl_driver_env(
    command: &mut Command,
    set_linux_opengl_driver: bool,
    use_zink_driver: bool,
) {
    if !set_linux_opengl_driver {
        return;
    }

    command.env_remove("MESA_LOADER_DRIVER_OVERRIDE");
    command.env_remove("GALLIUM_DRIVER");

    if use_zink_driver {
        command.env("MESA_LOADER_DRIVER_OVERRIDE", "zink");
        command.env("GALLIUM_DRIVER", "zink");
    }
}

#[cfg(not(target_os = "linux"))]
fn apply_linux_opengl_driver_env(
    _command: &mut Command,
    _set_linux_opengl_driver: bool,
    _use_zink_driver: bool,
) {
}

pub fn stop_running_instance(instance_root: &Path) -> bool {
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return false;
    };
    let Some(mut instance_processes) = processes.remove(key.as_str()) else {
        return false;
    };
    let mut stopped = false;
    for process in &mut instance_processes {
        if matches!(process.child.try_wait(), Ok(None)) {
            let _ = process.child.kill();
            let _ = process.child.wait();
            stopped = true;
        }
    }
    stopped
}

pub fn stop_running_instance_for_account(instance_root: &Path, account_key: &str) -> bool {
    let Some(account) = normalize_account_key(Some(account_key)) else {
        return false;
    };
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return false;
    };
    let mut removed_any = false;
    let mut emptied = false;
    if let Some(instance_processes) = processes.get_mut(key.as_str()) {
        let mut index = 0usize;
        while index < instance_processes.len() {
            let matches_account = instance_processes[index]
                .account_key
                .as_deref()
                .is_some_and(|value| value == account);
            if matches_account {
                let mut process = instance_processes.remove(index);
                if matches!(process.child.try_wait(), Ok(None)) {
                    let _ = process.child.kill();
                    let _ = process.child.wait();
                    removed_any = true;
                }
                continue;
            }
            index += 1;
        }
        emptied = instance_processes.is_empty();
    }
    if emptied {
        let _ = processes.remove(key.as_str());
    }
    removed_any
}

pub fn is_instance_running(instance_root: &Path) -> bool {
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return false;
    };
    prune_finished_processes(&mut processes);
    processes
        .get_mut(key.as_str())
        .is_some_and(|instance_processes| {
            instance_processes
                .iter_mut()
                .any(|process| matches!(process.child.try_wait(), Ok(None)))
        })
}

pub fn is_instance_running_for_account(instance_root: &Path, account_key: &str) -> bool {
    let Some(account) = normalize_account_key(Some(account_key)) else {
        return false;
    };
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return false;
    };
    prune_finished_processes(&mut processes);
    processes
        .get_mut(key.as_str())
        .is_some_and(|instance_processes| {
            instance_processes.iter_mut().any(|process| {
                process
                    .account_key
                    .as_deref()
                    .is_some_and(|value| value == account)
                    && matches!(process.child.try_wait(), Ok(None))
            })
        })
}

pub fn running_instance_for_account(account_key: &str) -> Option<String> {
    let account = normalize_account_key(Some(account_key))?;
    let Ok(mut processes) = process_registry().lock() else {
        return None;
    };
    prune_finished_processes(&mut processes);
    processes
        .iter_mut()
        .find_map(|(instance_root, instance_processes)| {
            if instance_processes.iter_mut().any(|process| {
                process
                    .account_key
                    .as_deref()
                    .is_some_and(|value| value == account)
                    && matches!(process.child.try_wait(), Ok(None))
            }) {
                Some(instance_root.clone())
            } else {
                None
            }
        })
}

pub fn running_account_for_instance(instance_root: &Path) -> Option<String> {
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return None;
    };
    prune_finished_processes(&mut processes);
    processes
        .get_mut(key.as_str())?
        .iter_mut()
        .find_map(|process| {
            if matches!(process.child.try_wait(), Ok(None)) {
                process.account_key.clone()
            } else {
                None
            }
        })
}

pub fn running_instance_roots() -> Vec<String> {
    let Ok(mut processes) = process_registry().lock() else {
        return Vec::new();
    };
    prune_finished_processes(&mut processes);
    processes.keys().cloned().collect()
}

fn prune_finished_processes(processes: &mut HashMap<String, Vec<RunningInstanceProcess>>) {
    processes.retain(|instance_root, instance_processes| {
        let mut index = 0usize;
        while index < instance_processes.len() {
            match instance_processes[index].child.try_wait() {
                Ok(None) => index += 1,
                Ok(Some(status)) => {
                    let process = instance_processes.remove(index);
                    push_finished_instance_process(finished_instance_process(
                        instance_root,
                        process,
                        status,
                    ));
                }
                Err(_) => {
                    let _ = instance_processes.remove(index);
                }
            }
        }
        !instance_processes.is_empty()
    });
}

fn instance_process_key(instance_root: &Path) -> String {
    let normalized = fs_canonicalize(instance_root).unwrap_or_else(|_| instance_root.to_path_buf());
    display_user_path(normalized.as_path())
}

fn normalize_account_key(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

#[derive(Clone, Debug)]
struct LaunchContext {
    substitutions: HashMap<String, String>,
    features: HashMap<String, bool>,
}

fn normalize_java_executable(configured: Option<&str>) -> String {
    let mut java = configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("java")
        .to_owned();
    let path_like = java.contains('/') || java.contains('\\');
    if path_like {
        let java_path = Path::new(java.as_str());
        if !java_path.exists() {
            java = "java".to_owned();
        } else if java_path.is_relative() {
            if let Ok(canonical) = fs_canonicalize(java_path) {
                java = display_user_path(canonical.as_path());
            } else if let Ok(cwd) = std::env::current_dir() {
                java = display_user_path(cwd.join(java_path).as_path());
            }
        } else {
            java = display_user_path(java_path);
        }
    }
    java
}

fn run_command_output(cmd: &mut Command, executable: &str) -> Result<Output, InstallationError> {
    cmd.output().map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            InstallationError::JavaExecutableNotFound {
                executable: executable.to_owned(),
            }
        } else {
            InstallationError::Io(err)
        }
    })
}

fn spawn_command_child(cmd: &mut Command, executable: &str) -> Result<Child, InstallationError> {
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            InstallationError::JavaExecutableNotFound {
                executable: executable.to_owned(),
            }
        } else {
            InstallationError::Io(err)
        }
    })
}

fn prepare_launch_log_file(
    instance_root: &Path,
) -> Result<(std::fs::File, PathBuf), InstallationError> {
    let logs_dir = instance_root.join("logs");
    fs_create_dir_all(&logs_dir)?;
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let log_path = logs_dir.join(format!("launch_{timestamp_ms}.log"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    Ok((file, log_path))
}

fn resolve_launch_profile_path(
    instance_root: &Path,
    game_version: &str,
    modloader: &str,
    modloader_version: Option<&str>,
) -> Result<(String, PathBuf), InstallationError> {
    let versions_dir = instance_root.join("versions");
    let requested_loader = normalized_loader_label(modloader);
    let allow_vanilla_fallback =
        matches!(requested_loader, LoaderKind::Vanilla | LoaderKind::Custom);
    tracing::info!(
        target: "vertexlauncher/installation/launch_profile",
        requested_modloader = %modloader,
        requested_game_version = %game_version,
        requested_modloader_version = %modloader_version.unwrap_or(""),
        allow_vanilla_fallback,
        "Resolving launch profile."
    );
    let mut candidates = Vec::<(String, PathBuf)>::new();

    if allow_vanilla_fallback {
        let game_path = versions_dir
            .join(game_version)
            .join(format!("{game_version}.json"));
        if game_path.exists() {
            candidates.push((game_version.to_owned(), game_path));
        }
    }

    if matches!(requested_loader, LoaderKind::Fabric | LoaderKind::Quilt)
        && let Some(loader_version) = modloader_version.map(str::trim).filter(|v| !v.is_empty())
    {
        let prefix = if requested_loader == LoaderKind::Fabric {
            "fabric-loader"
        } else {
            "quilt-loader"
        };
        let id = format!("{prefix}-{loader_version}-{game_version}");
        let path = versions_dir.join(id.as_str()).join(format!("{id}.json"));
        if path.exists() {
            candidates.insert(0, (id, path));
        }
    }

    let loader_hint = match requested_loader {
        LoaderKind::Forge => Some("forge"),
        LoaderKind::NeoForge => Some("neoforge"),
        LoaderKind::Fabric => Some("fabric-loader"),
        LoaderKind::Quilt => Some("quilt-loader"),
        LoaderKind::Vanilla | LoaderKind::Custom => None,
    };
    if let Some(loader_hint) = loader_hint
        && versions_dir.exists()
    {
        for entry in fs_read_dir(&versions_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let lower = dir_name.to_ascii_lowercase();
            if !lower.contains(loader_hint) {
                continue;
            }
            let game_lower = game_version.to_ascii_lowercase();
            if !lower.contains(game_lower.as_str()) {
                let profile_path = entry.path().join(format!("{dir_name}.json"));
                if !profile_path.exists() {
                    continue;
                }
                let raw = fs_read_to_string(&profile_path)?;
                let parsed: serde_json::Value = serde_json::from_str(&raw)?;
                let inherits = parsed
                    .get("inheritsFrom")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if !inherits.starts_with(game_lower.as_str()) && inherits != game_lower {
                    continue;
                }
            }
            if let Some(loader_version) = modloader_version.map(str::trim).filter(|v| !v.is_empty())
            {
                let lv = loader_version.to_ascii_lowercase();
                if !lower.contains(lv.as_str()) {
                    continue;
                }
            }
            let profile_path = entry.path().join(format!("{dir_name}.json"));
            if profile_path.exists() {
                candidates.insert(0, (dir_name, profile_path));
            }
        }
    }

    let resolved = candidates
        .into_iter()
        .find(|(_, path)| path.exists())
        .ok_or_else(|| InstallationError::LaunchProfileMissing {
            modloader: modloader.to_owned(),
            game_version: game_version.to_owned(),
        })?;
    tracing::info!(
        target: "vertexlauncher/installation/launch_profile",
        profile_id = %resolved.0,
        profile_path = %resolved.1.display(),
        "Resolved launch profile."
    );
    Ok(resolved)
}

fn load_profile_chain(
    instance_root: &Path,
    profile_path: &Path,
) -> Result<Vec<serde_json::Value>, InstallationError> {
    let mut chain = Vec::new();
    let mut cursor = profile_path.to_path_buf();
    let mut guard = 0usize;
    while guard < 16 {
        let raw = fs_read_to_string(cursor.as_path())?;
        let parsed: serde_json::Value = serde_json::from_str(&raw)?;
        let inherits = parsed
            .get("inheritsFrom")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        chain.push(parsed);
        let Some(parent) = inherits else {
            break;
        };
        cursor = instance_root
            .join("versions")
            .join(parent.as_str())
            .join(format!("{parent}.json"));
        guard = guard.saturating_add(1);
    }
    chain.reverse();
    Ok(chain)
}

fn resolve_main_class(chain: &[serde_json::Value]) -> Option<String> {
    for profile in chain.iter().rev() {
        if let Some(main_class) = profile.get("mainClass").and_then(serde_json::Value::as_str) {
            let trimmed = main_class.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

fn build_classpath_entries(
    instance_root: &Path,
    profile_id: &str,
    game_version: &str,
    main_class: &str,
    chain: &[serde_json::Value],
) -> Result<Vec<PathBuf>, InstallationError> {
    let mut classpath = Vec::<PathBuf>::new();
    let mut library_indices = HashMap::<String, usize>::new();

    for profile in chain {
        let Some(libraries) = profile
            .get("libraries")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for lib in libraries {
            if !library_rules_allow(lib) {
                continue;
            }
            let artifact_path = lib
                .get("downloads")
                .and_then(|v| v.get("artifact"))
                .and_then(|v| v.get("path"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .or_else(|| resolve_library_maven_download(lib).map(|(_, path)| path));
            let Some(artifact_path) = artifact_path else {
                continue;
            };
            let full = instance_root.join("libraries").join(artifact_path.as_str());
            if full.exists() {
                let dedupe_key = library_classpath_dedupe_key(lib, artifact_path.as_str());
                if let Some(existing_index) = library_indices.get(dedupe_key.as_str()).copied() {
                    classpath[existing_index] = full;
                } else {
                    library_indices.insert(dedupe_key, classpath.len());
                    classpath.push(full);
                }
            }
        }
    }

    let launch_jar = instance_root
        .join("versions")
        .join(profile_id)
        .join(format!("{profile_id}.jar"));
    if launch_jar.exists() {
        classpath.push(launch_jar);
    } else {
        // Forge 1.17+ (BootstrapLauncher) and NeoForge use the JPMS module system.
        // They load game classes via JarJar/FML — adding the vanilla jar to the
        // classpath would create a duplicate module (_1._20._1 vs minecraft) and
        // crash at startup. Skip the game jar entirely for these loaders.
        let uses_bootstrap_launcher = main_class.contains("BootstrapLauncher");
        if !uses_bootstrap_launcher {
            let fallback_jar = instance_root
                .join("versions")
                .join(game_version)
                .join(format!("{game_version}.jar"));
            if fallback_jar.exists() {
                classpath.push(fallback_jar);
            } else {
                return Err(InstallationError::LaunchFileMissing {
                    profile_id: profile_id.to_owned(),
                    path: launch_jar,
                });
            }
        }
    }
    Ok(classpath)
}

fn join_classpath(entries: &[PathBuf]) -> String {
    entries
        .iter()
        .map(|entry| display_user_path(entry.as_path()))
        .collect::<Vec<_>>()
        .join(classpath_separator())
}

struct LaunchClasspath {
    resolved: String,
    argfile: Option<PathBuf>,
}

fn prepare_launch_classpath(
    instance_root: &Path,
    profile_id: &str,
    entries: &[PathBuf],
) -> Result<LaunchClasspath, InstallationError> {
    let joined = join_classpath(entries);
    if joined.len() <= 7000 {
        return Ok(LaunchClasspath {
            resolved: joined,
            argfile: None,
        });
    }

    let argfile = write_classpath_argfile(instance_root, profile_id, joined.as_str())?;
    Ok(LaunchClasspath {
        resolved: joined,
        argfile: Some(argfile),
    })
}

fn write_classpath_argfile(
    instance_root: &Path,
    profile_id: &str,
    classpath: &str,
) -> Result<PathBuf, InstallationError> {
    let cache_dir = instance_root.join(".vertexlauncher");
    fs_create_dir_all(&cache_dir)?;
    let argfile_path = cache_dir.join(format!("classpath-{profile_id}.args"));
    let argfile_contents = format!("-cp\n{}\n", quote_java_argfile_value(classpath));
    fs_write(argfile_path.as_path(), argfile_contents.as_bytes())?;
    Ok(argfile_path)
}

fn quote_command_arg(arg: &str) -> String {
    if arg.is_empty()
        || arg
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\''))
    {
        format!("{arg:?}")
    } else {
        arg.to_owned()
    }
}

fn quote_java_argfile_value(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn strip_explicit_classpath_args(args: &mut Vec<String>) {
    let mut filtered = Vec::with_capacity(args.len());
    let mut index = 0usize;
    while index < args.len() {
        let current = args[index].as_str();
        if current == "-cp" || current == "-classpath" {
            index += 1;
            if index < args.len() {
                index += 1;
            }
            continue;
        }
        filtered.push(args[index].clone());
        index += 1;
    }
    *args = filtered;
}

fn has_explicit_classpath_args(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "-cp" | "-classpath"))
}

fn library_classpath_dedupe_key(lib: &serde_json::Value, artifact_path: &str) -> String {
    if let Some(name) = lib.get("name").and_then(serde_json::Value::as_str) {
        let mut parts = name.split(':');
        if let (Some(group), Some(artifact)) = (parts.next(), parts.next()) {
            let group = group.trim();
            let artifact = artifact.trim();
            if !group.is_empty() && !artifact.is_empty() {
                let classifier = parts
                    .nth(1)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("");
                return format!("{group}:{artifact}:{classifier}");
            }
        }
    }
    artifact_path.to_owned()
}

fn prepare_natives_dir(
    instance_root: &Path,
    profile_id: &str,
    chain: &[serde_json::Value],
) -> Result<PathBuf, InstallationError> {
    let natives_root = instance_root.join("natives").join(profile_id);
    if natives_root.exists() {
        fs_remove_dir_all(&natives_root)?;
    }
    fs_create_dir_all(&natives_root)?;

    for profile in chain {
        let Some(libraries) = profile
            .get("libraries")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for lib in libraries {
            if !library_rules_allow(lib) {
                continue;
            }
            let Some(natives) = lib.get("natives").and_then(serde_json::Value::as_object) else {
                continue;
            };
            let os_key = current_os_natives_key();
            let Some(classifier_template) = natives.get(os_key).and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let classifier = classifier_template.replace("${arch}", current_arch_natives_value());
            let Some(path) = lib
                .get("downloads")
                .and_then(|v| v.get("classifiers"))
                .and_then(|v| v.get(classifier.as_str()))
                .and_then(|v| v.get("path"))
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let archive = instance_root.join("libraries").join(path);
            if !archive.exists() {
                continue;
            }
            let excludes = lib
                .get("extract")
                .and_then(|v| v.get("exclude"))
                .and_then(serde_json::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            extract_natives_archive(archive.as_path(), natives_root.as_path(), &excludes)?;
        }
    }
    Ok(natives_root)
}

fn extract_natives_archive(
    archive_path: &Path,
    destination: &Path,
    excludes: &[String],
) -> Result<(), InstallationError> {
    let file = fs_file_open(archive_path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err.to_string()))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        if name.starts_with("META-INF/") || excludes.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        let out = destination.join(name.as_str());
        if let Some(parent) = out.parent() {
            fs_create_dir_all(parent)?;
        }
        let mut writer = fs_file_create(out)?;
        std::io::copy(&mut entry, &mut writer)?;
    }
    Ok(())
}

fn build_launch_context(
    instance_root: &Path,
    _game_version: &str,
    profile_id: &str,
    assets_index_name: &str,
    classpath: &str,
    natives_dir: &Path,
    player_name: Option<&str>,
    player_uuid: Option<&str>,
    auth_access_token: Option<&str>,
    auth_xuid: Option<&str>,
    auth_user_type: Option<&str>,
    quick_play_singleplayer: Option<&str>,
    quick_play_multiplayer: Option<&str>,
) -> LaunchContext {
    let username = player_name
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("Player");
    let uuid = player_uuid
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("00000000000000000000000000000000");
    let access_token = auth_access_token
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("0");
    let xuid = auth_xuid
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("0");
    let user_type = auth_user_type
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("legacy");
    let mut substitutions = HashMap::new();
    substitutions.insert("auth_player_name".to_owned(), username.to_owned());
    substitutions.insert("version_name".to_owned(), profile_id.to_owned());
    substitutions.insert(
        "game_directory".to_owned(),
        display_user_path(instance_root),
    );
    substitutions.insert(
        "assets_root".to_owned(),
        display_user_path(instance_root.join("assets").as_path()),
    );
    substitutions.insert("assets_index_name".to_owned(), assets_index_name.to_owned());
    // Legacy token used by pre-1.13 minecraftArguments as --assetsDir value.
    substitutions.insert(
        "game_assets".to_owned(),
        display_user_path(
            instance_root
                .join("assets")
                .join("virtual")
                .join(assets_index_name)
                .as_path(),
        ),
    );
    substitutions.insert("auth_uuid".to_owned(), uuid.to_owned());
    substitutions.insert("auth_access_token".to_owned(), access_token.to_owned());
    substitutions.insert("clientid".to_owned(), "0".to_owned());
    substitutions.insert("auth_xuid".to_owned(), xuid.to_owned());
    substitutions.insert("user_type".to_owned(), user_type.to_owned());
    substitutions.insert("version_type".to_owned(), "release".to_owned());
    substitutions.insert("user_properties".to_owned(), "{}".to_owned());
    substitutions.insert("classpath".to_owned(), classpath.to_owned());
    substitutions.insert(
        "classpath_separator".to_owned(),
        classpath_separator().to_owned(),
    );
    substitutions.insert(
        "library_directory".to_owned(),
        display_user_path(instance_root.join("libraries").as_path()),
    );
    substitutions.insert(
        "natives_directory".to_owned(),
        display_user_path(natives_dir),
    );
    substitutions.insert("launcher_name".to_owned(), "vertexlauncher".to_owned());
    substitutions.insert("launcher_version".to_owned(), "0.1".to_owned());
    substitutions.insert(
        "quickPlayPath".to_owned(),
        display_user_path(instance_root.join("quickPlay").as_path()),
    );
    if let Some(world) = quick_play_singleplayer
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        substitutions.insert("quickPlaySingleplayer".to_owned(), world.to_owned());
    }
    if let Some(server) = quick_play_multiplayer
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        substitutions.insert("quickPlayMultiplayer".to_owned(), server.to_owned());
    }
    let mut features = HashMap::new();
    features.insert("is_demo_user".to_owned(), false);
    features.insert("has_custom_resolution".to_owned(), false);
    let has_quick_play_singleplayer = quick_play_singleplayer
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let has_quick_play_multiplayer = quick_play_multiplayer
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    features.insert(
        "has_quick_plays_support".to_owned(),
        has_quick_play_singleplayer || has_quick_play_multiplayer,
    );
    features.insert(
        "is_quick_play_singleplayer".to_owned(),
        has_quick_play_singleplayer,
    );
    features.insert(
        "is_quick_play_multiplayer".to_owned(),
        has_quick_play_multiplayer,
    );
    features.insert("is_quick_play_realms".to_owned(), false);
    LaunchContext {
        substitutions,
        features,
    }
}

fn resolve_assets_index_name<'a>(chain: &'a [serde_json::Value], fallback: &'a str) -> &'a str {
    for profile in chain.iter().rev() {
        if let Some(id) = profile
            .get("assetIndex")
            .and_then(|value| value.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return id;
        }
    }
    fallback
}

fn collect_jvm_arguments(chain: &[serde_json::Value], context: &LaunchContext) -> Vec<String> {
    let mut args = Vec::new();
    for profile in chain {
        if let Some(values) = profile
            .get("arguments")
            .and_then(|v| v.get("jvm"))
            .and_then(serde_json::Value::as_array)
        {
            args.extend(collect_argument_array(values, context));
        }
    }
    if args.is_empty() {
        args.push("-Djava.library.path=${natives_directory}".to_owned());
        args.push("-cp".to_owned());
        args.push("${classpath}".to_owned());
    }
    args.into_iter()
        .map(|entry| substitute_tokens(entry.as_str(), context))
        .collect()
}

fn collect_game_arguments(chain: &[serde_json::Value], context: &LaunchContext) -> Vec<String> {
    let mut args = Vec::new();
    for profile in chain {
        if let Some(values) = profile
            .get("arguments")
            .and_then(|v| v.get("game"))
            .and_then(serde_json::Value::as_array)
        {
            args.extend(collect_argument_array(values, context));
        }
    }
    if args.is_empty() {
        for profile in chain.iter().rev() {
            if let Some(raw) = profile
                .get("minecraftArguments")
                .and_then(serde_json::Value::as_str)
            {
                args.extend(raw.split_whitespace().map(str::to_owned));
                break;
            }
        }
    }
    let resolved: Vec<String> = args
        .into_iter()
        .map(|entry| substitute_tokens(entry.as_str(), context))
        .collect();
    normalize_quick_play_arguments(resolved, context)
}

fn normalize_quick_play_arguments(args: Vec<String>, context: &LaunchContext) -> Vec<String> {
    let quick_play_path = context
        .substitutions
        .get("quickPlayPath")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let quick_play_singleplayer = context
        .substitutions
        .get("quickPlaySingleplayer")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let quick_play_multiplayer = context
        .substitutions
        .get("quickPlayMultiplayer")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let requested_quick_play_mode = quick_play_singleplayer
        .map(|world| ("--quickPlaySingleplayer", world))
        .or_else(|| quick_play_multiplayer.map(|server| ("--quickPlayMultiplayer", server)));

    let mut out = Vec::new();
    let mut cursor = 0usize;
    let mut has_quick_play_path = false;
    let mut quick_play_mode_selected = false;

    while cursor < args.len() {
        let current = args[cursor].as_str();
        let is_quick_play_flag = matches!(
            current,
            "--quickPlayPath"
                | "--quickPlaySingleplayer"
                | "--quickPlayMultiplayer"
                | "--quickPlayRealms"
        );
        if !is_quick_play_flag {
            out.push(args[cursor].clone());
            cursor += 1;
            continue;
        }

        let value = args.get(cursor + 1).map(String::as_str).unwrap_or_default();
        let unresolved_placeholder =
            value.starts_with("${quickPlay") && value.ends_with('}') && value.len() > 2;
        if value.trim().is_empty() || unresolved_placeholder {
            cursor = cursor.saturating_add(2);
            continue;
        }

        if current == "--quickPlayPath" {
            if has_quick_play_path {
                cursor = cursor.saturating_add(2);
                continue;
            }
            has_quick_play_path = true;
            out.push(args[cursor].clone());
            out.push(args[cursor + 1].clone());
            cursor += 2;
            continue;
        }

        if quick_play_mode_selected {
            cursor = cursor.saturating_add(2);
            continue;
        }

        out.push(args[cursor].clone());
        out.push(args[cursor + 1].clone());
        quick_play_mode_selected = true;
        cursor += 2;
    }

    if let Some((flag, value)) = requested_quick_play_mode
        && !quick_play_mode_selected
    {
        if !has_quick_play_path && let Some(path) = quick_play_path {
            out.push("--quickPlayPath".to_owned());
            out.push(path.to_owned());
        }
        out.push(flag.to_owned());
        out.push(value.to_owned());
    }

    out
}

fn collect_argument_array(values: &[serde_json::Value], context: &LaunchContext) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        if let Some(raw) = value.as_str() {
            out.push(substitute_tokens(raw, context));
            continue;
        }
        let Some(object) = value.as_object() else {
            continue;
        };
        if !rules_allow_for_launch(object.get("rules"), context) {
            continue;
        }
        let Some(arg_value) = object.get("value") else {
            continue;
        };
        if let Some(single) = arg_value.as_str() {
            out.push(substitute_tokens(single, context));
        } else if let Some(array) = arg_value.as_array() {
            for entry in array {
                if let Some(single) = entry.as_str() {
                    out.push(substitute_tokens(single, context));
                }
            }
        }
    }
    out
}

fn library_rules_allow(library: &serde_json::Value) -> bool {
    rules_allow_os_only(library.get("rules"))
}

fn rules_allow_for_launch(
    rules_value: Option<&serde_json::Value>,
    context: &LaunchContext,
) -> bool {
    let Some(rules) = rules_value.and_then(serde_json::Value::as_array) else {
        return true;
    };
    let mut allowed = false;
    for rule in rules {
        let Some(object) = rule.as_object() else {
            continue;
        };
        let action = object
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("allow");
        let applies = rule_applies_to_current_os(object.get("os"))
            && rule_features_match(object.get("features"), context);
        if applies {
            allowed = action == "allow";
        }
    }
    allowed
}

fn rules_allow_os_only(rules_value: Option<&serde_json::Value>) -> bool {
    let Some(rules) = rules_value.and_then(serde_json::Value::as_array) else {
        return true;
    };
    let mut allowed = false;
    for rule in rules {
        let Some(object) = rule.as_object() else {
            continue;
        };
        let action = object
            .get("action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("allow");
        let applies = rule_applies_to_current_os(object.get("os"));
        if applies {
            allowed = action == "allow";
        }
    }
    allowed
}

fn rule_features_match(
    features_value: Option<&serde_json::Value>,
    context: &LaunchContext,
) -> bool {
    let Some(features) = features_value.and_then(serde_json::Value::as_object) else {
        return true;
    };
    for (feature, expected) in features {
        let Some(expected) = expected.as_bool() else {
            continue;
        };
        let actual = context.features.get(feature).copied().unwrap_or(false);
        if actual != expected {
            return false;
        }
    }
    true
}

fn rule_applies_to_current_os(os_value: Option<&serde_json::Value>) -> bool {
    let Some(os_object) = os_value.and_then(serde_json::Value::as_object) else {
        return true;
    };
    if let Some(name) = os_object.get("name").and_then(serde_json::Value::as_str)
        && name != current_os_natives_key()
    {
        return false;
    }
    if let Some(arch) = os_object.get("arch").and_then(serde_json::Value::as_str)
        && !arch_matches_current_target(arch)
    {
        return false;
    }
    true
}

fn substitute_tokens(raw: &str, context: &LaunchContext) -> String {
    let mut result = raw.to_owned();
    for (key, value) in &context.substitutions {
        let token = format!("${{{key}}}");
        result = result.replace(token.as_str(), value.as_str());
    }
    result
}

fn parse_user_args(raw: Option<&str>) -> Vec<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn classpath_separator() -> &'static str {
    if cfg!(target_os = "windows") {
        ";"
    } else {
        ":"
    }
}

fn should_use_environment_classpath() -> bool {
    false
}

fn current_os_natives_key() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

fn current_arch_natives_value() -> &'static str {
    if cfg!(target_pointer_width = "64") {
        "64"
    } else {
        "32"
    }
}

fn arch_matches_current_target(expected: &str) -> bool {
    let normalized = expected.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "x86" | "i386" | "i686" | "32" => cfg!(target_arch = "x86"),
        "x86_64" | "amd64" | "64" => cfg!(target_arch = "x86_64"),
        "arm64" | "aarch64" => cfg!(target_arch = "aarch64"),
        "arm" => cfg!(target_arch = "arm"),
        other => std::env::consts::ARCH.eq_ignore_ascii_case(other),
    }
}

fn report_install_progress(progress: Option<&InstallProgressSink>, event: InstallProgress) {
    if let Some(callback) = progress {
        callback(event);
    }
}

fn download_version_dependencies(
    instance_root: &Path,
    version_json_path: &Path,
    policy: &DownloadPolicy,
    downloaded_files_offset: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    if !version_json_path.exists() {
        return Ok(0);
    }
    tracing::info!(
        target: "vertexlauncher/installation/dependencies",
        instance_root = %instance_root.display(),
        version_json_path = %version_json_path.display(),
        "Starting version dependency resolution."
    );
    let raw = fs_read_to_string(version_json_path).map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/dependencies",
            instance_root = %instance_root.display(),
            version_json_path = %version_json_path.display(),
            error = %err,
            "Failed to read version metadata JSON."
        );
        InstallationError::Io(err)
    })?;
    let version_meta: serde_json::Value = serde_json::from_str(&raw)?;
    let mut downloaded = 0u32;

    let mut library_tasks = Vec::new();
    collect_library_download_tasks(instance_root, &version_meta, &mut library_tasks);
    tracing::info!(
        target: "vertexlauncher/installation/dependencies",
        library_task_count = library_tasks.len(),
        "Collected library download tasks."
    );
    downloaded += download_files_concurrent(
        InstallStage::DownloadingCore,
        library_tasks,
        policy,
        downloaded_files_offset.saturating_add(downloaded),
        progress,
    )
    .map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/dependencies",
            instance_root = %instance_root.display(),
            version_json_path = %version_json_path.display(),
            error = %err,
            "Library dependency download batch failed."
        );
        err
    })?;

    let mut asset_index_task = Vec::new();
    let asset_index_path =
        collect_asset_index_download_task(instance_root, &version_meta, &mut asset_index_task);
    tracing::info!(
        target: "vertexlauncher/installation/dependencies",
        asset_index_task_count = asset_index_task.len(),
        asset_index_path = %asset_index_path.as_ref().map(|path| path.display().to_string()).unwrap_or_default(),
        "Collected asset index download task."
    );
    downloaded += download_files_concurrent(
        InstallStage::DownloadingCore,
        asset_index_task,
        policy,
        downloaded_files_offset.saturating_add(downloaded),
        progress,
    )
    .map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/dependencies",
            instance_root = %instance_root.display(),
            version_json_path = %version_json_path.display(),
            asset_index_path = %asset_index_path.as_ref().map(|path| path.display().to_string()).unwrap_or_default(),
            error = %err,
            "Asset index download batch failed."
        );
        err
    })?;

    if let Some(asset_index_path) = asset_index_path {
        let mut object_tasks = Vec::new();
        collect_asset_object_download_tasks(
            instance_root,
            asset_index_path.as_path(),
            &mut object_tasks,
        )
        .map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/dependencies",
                instance_root = %instance_root.display(),
                asset_index_path = %asset_index_path.display(),
                error = %err,
                "Failed while collecting asset object download tasks from asset index."
            );
            err
        })?;
        tracing::info!(
            target: "vertexlauncher/installation/dependencies",
            asset_index_path = %asset_index_path.display(),
            asset_object_task_count = object_tasks.len(),
            "Collected asset object download tasks."
        );
        downloaded += download_files_concurrent(
            InstallStage::DownloadingCore,
            object_tasks,
            policy,
            downloaded_files_offset.saturating_add(downloaded),
            progress,
        )
        .map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/dependencies",
                instance_root = %instance_root.display(),
                asset_index_path = %asset_index_path.display(),
                error = %err,
                "Asset object download batch failed."
            );
            err
        })?;
    }

    tracing::info!(
        target: "vertexlauncher/installation/dependencies",
        instance_root = %instance_root.display(),
        version_json_path = %version_json_path.display(),
        downloaded,
        "Completed version dependency resolution."
    );
    Ok(downloaded)
}

fn collect_library_download_tasks(
    instance_root: &Path,
    version_meta: &serde_json::Value,
    tasks: &mut Vec<FileDownloadTask>,
) {
    let Some(libraries) = version_meta
        .get("libraries")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    for library in libraries {
        if let Some(downloads) = library.get("downloads") {
            if let Some(artifact) = downloads.get("artifact") {
                push_download_task_from_download_entry(
                    instance_root.join("libraries").as_path(),
                    artifact,
                    tasks,
                );
            }
            if let Some(classifiers) = downloads
                .get("classifiers")
                .and_then(serde_json::Value::as_object)
            {
                for entry in classifiers.values() {
                    push_download_task_from_download_entry(
                        instance_root.join("libraries").as_path(),
                        entry,
                        tasks,
                    );
                }
            }
        } else if let Some((url, relative_path)) = resolve_library_maven_download(library) {
            let destination = instance_root.join("libraries").join(relative_path.as_str());
            if !destination.exists() {
                tasks.push(FileDownloadTask {
                    url,
                    destination,
                    expected_size: library
                        .get("size")
                        .and_then(serde_json::Value::as_u64)
                        .filter(|size| *size > 0),
                });
            }
        }
    }
}

fn resolve_library_maven_download(library: &serde_json::Value) -> Option<(String, String)> {
    let name = library.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let mut parts = name.split(':');
    let group = parts.next()?.trim();
    let artifact = parts.next()?.trim();
    let version_and_ext = parts.next()?.trim();
    let classifier = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if group.is_empty() || artifact.is_empty() || version_and_ext.is_empty() {
        return None;
    }

    let (version, extension) = if let Some((version, ext)) = version_and_ext.split_once('@') {
        (version.trim(), ext.trim())
    } else {
        (version_and_ext, "jar")
    };
    if version.is_empty() || extension.is_empty() {
        return None;
    }

    let base_url = library
        .get("url")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("https://libraries.minecraft.net/");
    let group_path = group.replace('.', "/");
    let file_name = if let Some(classifier) = classifier {
        format!("{artifact}-{version}-{classifier}.{extension}")
    } else {
        format!("{artifact}-{version}.{extension}")
    };
    let relative_path = format!("{group_path}/{artifact}/{version}/{file_name}");
    let url = format!(
        "{}{relative_path}",
        base_url.trim_end_matches('/').to_owned() + "/"
    );
    Some((url, relative_path))
}

fn collect_asset_index_download_task(
    instance_root: &Path,
    version_meta: &serde_json::Value,
    tasks: &mut Vec<FileDownloadTask>,
) -> Option<PathBuf> {
    let asset_index = version_meta.get("assetIndex")?;
    let url = asset_index.get("url")?.as_str()?.trim();
    if url.is_empty() {
        return None;
    }
    let id = asset_index
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default");
    let destination = instance_root
        .join("assets")
        .join("indexes")
        .join(format!("{id}.json"));
    if !destination.exists() {
        tasks.push(FileDownloadTask {
            url: url.to_owned(),
            destination: destination.clone(),
            expected_size: asset_index
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .filter(|size| *size > 0),
        });
    }
    Some(destination)
}

fn collect_asset_object_download_tasks(
    instance_root: &Path,
    asset_index_path: &Path,
    tasks: &mut Vec<FileDownloadTask>,
) -> Result<(), InstallationError> {
    if !asset_index_path.exists() {
        return Ok(());
    }
    let raw = fs_read_to_string(asset_index_path).map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/dependencies",
            instance_root = %instance_root.display(),
            asset_index_path = %asset_index_path.display(),
            error = %err,
            "Failed to read asset index JSON."
        );
        InstallationError::Io(err)
    })?;
    let index: serde_json::Value = serde_json::from_str(&raw)?;
    let Some(objects) = index.get("objects").and_then(serde_json::Value::as_object) else {
        return Ok(());
    };
    let mut seen_destinations = HashSet::new();
    let mut duplicate_count = 0usize;
    for entry in objects.values() {
        let Some(hash) = entry.get("hash").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let hash = hash.trim();
        if hash.len() < 2 {
            continue;
        }
        let prefix = &hash[..2];
        let destination = instance_root
            .join("assets")
            .join("objects")
            .join(prefix)
            .join(hash);
        if destination.exists() {
            continue;
        }
        if !seen_destinations.insert(destination.clone()) {
            duplicate_count += 1;
            continue;
        }
        tasks.push(FileDownloadTask {
            url: format!("https://resources.download.minecraft.net/{prefix}/{hash}"),
            destination,
            expected_size: entry
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .filter(|size| *size > 0),
        });
    }
    if duplicate_count > 0 {
        tracing::info!(
            target: "vertexlauncher/installation/dependencies",
            asset_index_path = %asset_index_path.display(),
            duplicate_count,
            deduped_task_count = tasks.len(),
            "Deduplicated repeated asset object downloads that pointed to the same destination."
        );
    }
    Ok(())
}

fn push_download_task_from_download_entry(
    root: &Path,
    download_entry: &serde_json::Value,
    tasks: &mut Vec<FileDownloadTask>,
) {
    let Some(url) = download_entry
        .get("url")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let Some(path) = download_entry
        .get("path")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let url = url.trim();
    let path = path.trim();
    if url.is_empty() || path.is_empty() {
        return;
    }
    let destination = root.join(path);
    if destination.exists() {
        return;
    }
    tasks.push(FileDownloadTask {
        url: url.to_owned(),
        destination,
        expected_size: download_entry
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .filter(|size| *size > 0),
    });
}

#[derive(Clone, Debug)]
struct FileDownloadTask {
    url: String,
    destination: PathBuf,
    expected_size: Option<u64>,
}

#[derive(Debug)]
struct BandwidthLimiter {
    bits_per_second: u64,
    state: Mutex<BandwidthState>,
}

#[derive(Debug)]
struct BandwidthState {
    window_start: Instant,
    bits_sent: u128,
}

impl BandwidthLimiter {
    fn new(bits_per_second: u64) -> Self {
        Self {
            bits_per_second: bits_per_second.max(1),
            state: Mutex::new(BandwidthState {
                window_start: Instant::now(),
                bits_sent: 0,
            }),
        }
    }

    fn consume(&self, bytes: usize) {
        let requested_bits = (bytes as u128).saturating_mul(8);
        loop {
            let wait_duration = {
                let Ok(mut state) = self.state.lock() else {
                    return;
                };
                let elapsed = state.window_start.elapsed();
                if elapsed >= Duration::from_secs(1) {
                    state.window_start = Instant::now();
                    state.bits_sent = 0;
                }
                let max_bits = self.bits_per_second as u128;
                if state.bits_sent.saturating_add(requested_bits) <= max_bits {
                    state.bits_sent = state.bits_sent.saturating_add(requested_bits);
                    None
                } else {
                    Some(Duration::from_secs(1).saturating_sub(elapsed))
                }
            };
            if let Some(wait) = wait_duration {
                thread::sleep(wait.max(Duration::from_millis(1)));
                continue;
            }
            return;
        }
    }
}

#[derive(Debug)]
struct DownloadTelemetry {
    started_at: Instant,
    total_files: u32,
    completed_files: AtomicU32,
    downloaded_bytes: AtomicU64,
    known_total_bytes: AtomicU64,
    last_emit_millis: AtomicU64,
    eta_state: Mutex<ProgressEtaState>,
}

impl DownloadTelemetry {
    fn new(total_files: u32, known_total_bytes: u64) -> Self {
        Self {
            started_at: Instant::now(),
            total_files,
            completed_files: AtomicU32::new(0),
            downloaded_bytes: AtomicU64::new(0),
            known_total_bytes: AtomicU64::new(known_total_bytes),
            last_emit_millis: AtomicU64::new(0),
            eta_state: Mutex::new(ProgressEtaState::default()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ProgressEtaPoint {
    fraction: f64,
    at_millis: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct ProgressEtaState {
    last_point: Option<ProgressEtaPoint>,
    last_eta_seconds: Option<u64>,
}

impl ProgressEtaState {
    fn observe(&mut self, point: ProgressEtaPoint) -> Option<u64> {
        let fraction = point.fraction.clamp(0.0, 1.0);
        if fraction >= 1.0 {
            self.last_point = Some(point);
            self.last_eta_seconds = Some(0);
            return Some(0);
        }

        let Some(previous) = self.last_point else {
            self.last_point = Some(point);
            self.last_eta_seconds = None;
            return None;
        };

        if fraction < previous.fraction || point.at_millis <= previous.at_millis {
            self.last_point = Some(point);
            self.last_eta_seconds = None;
            return None;
        }

        let delta_fraction = fraction - previous.fraction;
        if delta_fraction <= f64::EPSILON {
            return self.last_eta_seconds;
        }

        let delta_seconds = (point.at_millis - previous.at_millis) as f64 / 1000.0;
        if delta_seconds <= 0.0 {
            return self.last_eta_seconds;
        }

        let fraction_per_second = delta_fraction / delta_seconds;
        let eta_seconds = if fraction_per_second > 0.0 {
            Some(((1.0 - fraction) / fraction_per_second).ceil().max(0.0) as u64)
        } else {
            None
        };

        self.last_point = Some(point);
        self.last_eta_seconds = eta_seconds;
        eta_seconds
    }
}

fn install_progress_fraction(
    downloaded_files: u32,
    total_files: u32,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> f64 {
    if let Some(total_bytes) = total_bytes
        && total_bytes > 0
    {
        return (downloaded_bytes as f64 / total_bytes as f64).clamp(0.0, 1.0);
    }
    if total_files > 0 {
        return (downloaded_files as f64 / total_files as f64).clamp(0.0, 1.0);
    }
    0.0
}

fn emit_download_progress(
    progress: Option<&InstallProgressSink>,
    telemetry: &DownloadTelemetry,
    stage: InstallStage,
    downloaded_files_offset: u32,
) {
    let Some(progress) = progress else {
        return;
    };

    let now_millis = telemetry.started_at.elapsed().as_millis() as u64;
    let last_millis = telemetry.last_emit_millis.load(Ordering::Relaxed);
    if now_millis > 0 && now_millis.saturating_sub(last_millis) < 200 {
        return;
    }
    telemetry
        .last_emit_millis
        .store(now_millis, Ordering::Relaxed);

    let completed_files = telemetry.completed_files.load(Ordering::Relaxed);
    let downloaded_bytes = telemetry.downloaded_bytes.load(Ordering::Relaxed);
    let known_total_bytes = telemetry.known_total_bytes.load(Ordering::Relaxed);
    let elapsed = telemetry.started_at.elapsed().as_secs_f64().max(0.001);
    let bytes_per_second = downloaded_bytes as f64 / elapsed;
    let total_bytes = (known_total_bytes > 0).then_some(known_total_bytes);
    let downloaded_files = downloaded_files_offset.saturating_add(completed_files);
    let total_files = downloaded_files_offset.saturating_add(telemetry.total_files);
    let fraction =
        install_progress_fraction(downloaded_files, total_files, downloaded_bytes, total_bytes);
    let eta_seconds = telemetry.eta_state.lock().ok().and_then(|mut state| {
        state.observe(ProgressEtaPoint {
            fraction,
            at_millis: now_millis,
        })
    });

    progress(InstallProgress {
        stage,
        message: format!(
            "Downloading files ({}/{})...",
            downloaded_files, total_files
        ),
        downloaded_files,
        total_files,
        downloaded_bytes,
        total_bytes,
        bytes_per_second,
        eta_seconds,
    });
}

fn prefetch_batch_total_bytes(
    tasks: &mut [FileDownloadTask],
    probe_workers: usize,
    max_probe_count: usize,
) -> Result<u64, InstallationError> {
    let mut total_known_bytes = 0u64;
    let mut unknown = std::collections::VecDeque::new();
    let mut skipped_probe_count = 0usize;
    for (index, task) in tasks.iter().enumerate() {
        if let Some(size) = task.expected_size {
            total_known_bytes = total_known_bytes.saturating_add(size);
        } else {
            if unknown.len() < max_probe_count {
                unknown.push_back((index, task.url.clone()));
            } else {
                skipped_probe_count += 1;
            }
        }
    }
    if unknown.is_empty() {
        return Ok(total_known_bytes);
    }
    if skipped_probe_count > 0 {
        tracing::debug!(
            target: "vertexlauncher/installation/downloads",
            probed = unknown.len(),
            skipped = skipped_probe_count,
            "Skipping some HEAD size probes to reduce batch startup latency"
        );
    }

    let queue = Arc::new(Mutex::new(unknown));
    let discovered = Arc::new(Mutex::new(Vec::<(usize, u64)>::new()));
    thread::scope(|scope| -> Result<(), InstallationError> {
        let mut workers = Vec::new();
        for _ in 0..probe_workers.max(1) {
            let queue = Arc::clone(&queue);
            let discovered = Arc::clone(&discovered);
            workers.push(scope.spawn(move || {
                loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some((index, url)) = next else {
                        break;
                    };
                    if let Some(size) = probe_content_length(url.as_str())
                        && let Ok(mut guard) = discovered.lock()
                    {
                        guard.push((index, size));
                    }
                }
            }));
        }
        for worker in workers {
            worker.join().map_err(|_| {
                InstallationError::Io(std::io::Error::other(
                    "content-length probe worker panicked",
                ))
            })?;
        }
        Ok(())
    })?;

    if let Ok(discovered) = discovered.lock() {
        for (index, size) in discovered.iter().copied() {
            if let Some(task) = tasks.get_mut(index)
                && task.expected_size.is_none()
            {
                task.expected_size = Some(size);
                total_known_bytes = total_known_bytes.saturating_add(size);
            }
        }
    }

    Ok(total_known_bytes)
}

fn probe_content_length(url: &str) -> Option<u64> {
    let response = match http_agent()
        .head(url)
        .header("User-Agent", DEFAULT_USER_AGENT)
        .config()
        .http_status_as_error(false)
        .build()
        .call()
    {
        Ok(response) => response,
        Err(err) => {
            tracing::debug!(
                target: "vertexlauncher/installation/downloads",
                "Size prefetch HEAD transport error for {}: {}",
                url,
                err
            );
            return None;
        }
    };
    if response.status().as_u16() >= 400 {
        tracing::debug!(
            target: "vertexlauncher/installation/downloads",
            "Size prefetch HEAD failed for {} with status {}",
            url,
            response.status().as_u16()
        );
        return None;
    }
    response
        .headers()
        .get("Content-Length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|size| *size > 0)
}

fn download_files_concurrent(
    stage: InstallStage,
    tasks: Vec<FileDownloadTask>,
    policy: &DownloadPolicy,
    downloaded_files_offset: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    if tasks.is_empty() {
        return Ok(0);
    }

    let total_files = tasks.len() as u32;
    let mut tasks = tasks;
    let batch_started_at = Instant::now();
    let worker_count = policy.max_concurrent_downloads.clamp(1, 64) as usize;
    let size_probe_workers = worker_count.min(8).max(1);
    let max_size_probes = MAX_CONTENT_LENGTH_PROBES_PER_BATCH.max(size_probe_workers);
    let prefetched_total_bytes =
        prefetch_batch_total_bytes(&mut tasks, size_probe_workers, max_size_probes)?;
    // Prioritize larger files so long-running transfers start earlier.
    tasks.sort_by_key(|task| std::cmp::Reverse(task.expected_size.unwrap_or(0)));
    tracing::info!(
        target: "vertexlauncher/installation/downloads",
        "Starting {:?} batch: {} file(s), prefetched_total_bytes={}, max_concurrent_downloads={}, speed_limit_bps={:?}.",
        stage,
        total_files,
        prefetched_total_bytes,
        policy.max_concurrent_downloads,
        policy.max_download_bps
    );
    let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(tasks)));
    let bandwidth_limiter = policy
        .max_download_bps
        .map(BandwidthLimiter::new)
        .map(Arc::new);
    let telemetry = Arc::new(DownloadTelemetry::new(total_files, prefetched_total_bytes));

    emit_download_progress(progress, &telemetry, stage, downloaded_files_offset);

    let downloaded_files = thread::scope(|scope| -> Result<u32, InstallationError> {
        let mut workers = Vec::new();
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let bandwidth_limiter = bandwidth_limiter.as_ref().map(Arc::clone);
            let telemetry = Arc::clone(&telemetry);
            workers.push(scope.spawn(move || -> Result<u32, InstallationError> {
                let mut completed = 0u32;
                loop {
                    let next_task = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some(task) = next_task else {
                        break;
                    };
                    download_to_file(
                        task,
                        bandwidth_limiter.as_deref(),
                        &telemetry,
                        downloaded_files_offset,
                        stage,
                        progress,
                    )?;
                    completed += 1;
                }
                Ok(completed)
            }));
        }

        let mut total = 0u32;
        for worker in workers {
            match worker.join() {
                Ok(Ok(count)) => total += count,
                Ok(Err(err)) => return Err(err),
                Err(_) => {
                    return Err(InstallationError::Io(std::io::Error::other(
                        "download worker panicked",
                    )));
                }
            }
        }
        Ok(total)
    })?;

    emit_download_progress(progress, &telemetry, stage, downloaded_files_offset);
    tracing::info!(
        target: "vertexlauncher/installation/downloads",
        "Completed {:?} batch: {} file(s) in {:.2}s.",
        stage,
        downloaded_files,
        batch_started_at.elapsed().as_secs_f64()
    );
    Ok(downloaded_files)
}

fn download_to_file(
    task: FileDownloadTask,
    bandwidth_limiter: Option<&BandwidthLimiter>,
    telemetry: &DownloadTelemetry,
    downloaded_files_offset: u32,
    stage: InstallStage,
    progress: Option<&InstallProgressSink>,
) -> Result<(), InstallationError> {
    if let Some(parent) = task.destination.parent() {
        fs_create_dir_all(parent)?;
    }
    let started_at = Instant::now();
    tracing::debug!(
        target: "vertexlauncher/installation/downloads",
        "Download start: {} -> {}",
        task.url,
        task.destination.display()
    );

    let mut response = call_get_response_with_retry(task.url.as_str(), DEFAULT_USER_AGENT)?;
    if task.expected_size.is_none()
        && let Some(content_length) = response
            .headers()
            .get("Content-Length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|size| *size > 0)
    {
        telemetry
            .known_total_bytes
            .fetch_add(content_length, Ordering::Relaxed);
    }

    let temp_path = temporary_download_path(task.destination.as_path());
    let mut reader = response.body_mut().as_reader();
    let mut file = fs_file_create(&temp_path)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/downloads",
                url = %task.url,
                temp_path = %temp_path.display(),
                destination = %task.destination.display(),
                error = %err,
                "Failed while reading HTTP response body for download."
            );
            InstallationError::Io(err)
        })?;
        if read == 0 {
            break;
        }
        if let Some(limiter) = bandwidth_limiter {
            limiter.consume(read);
        }
        telemetry
            .downloaded_bytes
            .fetch_add(read as u64, Ordering::Relaxed);
        emit_download_progress(progress, telemetry, stage, downloaded_files_offset);
        file.write_all(&buffer[..read]).map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/downloads",
                url = %task.url,
                temp_path = %temp_path.display(),
                destination = %task.destination.display(),
                error = %err,
                "Failed while writing download chunk to temporary file."
            );
            InstallationError::Io(err)
        })?;
    }
    file.flush().map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/downloads",
            url = %task.url,
            temp_path = %temp_path.display(),
            destination = %task.destination.display(),
            error = %err,
            "Failed while flushing temporary download file."
        );
        InstallationError::Io(err)
    })?;
    fs_rename(temp_path.as_path(), task.destination.as_path()).map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/downloads",
            url = %task.url,
            temp_path = %temp_path.display(),
            destination = %task.destination.display(),
            error = %err,
            "Failed while promoting temporary download file into place."
        );
        err
    })?;
    telemetry.completed_files.fetch_add(1, Ordering::Relaxed);
    emit_download_progress(progress, telemetry, stage, downloaded_files_offset);
    tracing::debug!(
        target: "vertexlauncher/installation/downloads",
        "Download complete: {} ({:.2}s)",
        task.destination.display(),
        started_at.elapsed().as_secs_f64()
    );
    Ok(())
}

fn install_selected_modloader(
    instance_root: &Path,
    game_version: &str,
    modloader: &str,
    modloader_version: Option<&str>,
    java_executable: Option<&str>,
    policy: &DownloadPolicy,
    downloaded_files: &mut u32,
    progress: Option<&InstallProgressSink>,
) -> Result<Option<String>, InstallationError> {
    let loader_kind = normalized_loader_label(modloader);
    tracing::info!(
        target: "vertexlauncher/installation/modloader",
        requested_modloader = %modloader,
        requested_game_version = %game_version,
        requested_modloader_version = %modloader_version.unwrap_or(""),
        "Selecting modloader installation strategy."
    );
    match loader_kind {
        LoaderKind::Vanilla | LoaderKind::Custom => Ok(None),
        LoaderKind::Fabric | LoaderKind::Quilt => {
            let loader_label = if loader_kind == LoaderKind::Fabric {
                "Fabric"
            } else {
                "Quilt"
            };
            let resolved =
                resolve_loader_version(loader_kind, loader_label, game_version, modloader_version)?;
            if has_fabric_or_quilt_profile(instance_root, game_version, loader_kind, &resolved)? {
                tracing::info!(
                    target: "vertexlauncher/installation/modloader",
                    loader = %loader_label,
                    game_version = %game_version,
                    resolved = %resolved,
                    "Modloader profile already present; skipping profile install."
                );
                return Ok(Some(resolved));
            }
            emit_installing_modloader_progress(
                loader_label,
                &resolved,
                *downloaded_files,
                progress,
            );
            *downloaded_files += install_fabric_or_quilt_profile(
                instance_root,
                game_version,
                loader_kind,
                &resolved,
                policy,
                *downloaded_files,
                progress,
            )?;
            Ok(Some(resolved))
        }
        LoaderKind::Forge => {
            let resolved =
                resolve_loader_version(loader_kind, "Forge", game_version, modloader_version)?;
            if verify_modloader_profile(instance_root, loader_kind, game_version, &resolved)? {
                tracing::info!(
                    target: "vertexlauncher/installation/modloader",
                    loader = "Forge",
                    game_version = %game_version,
                    resolved = %resolved,
                    "Modloader profile already present; skipping installer execution."
                );
                return Ok(Some(resolved));
            }
            emit_installing_modloader_progress("Forge", &resolved, *downloaded_files, progress);
            *downloaded_files += install_forge_installer(
                instance_root,
                game_version,
                &resolved,
                java_executable,
                policy,
                *downloaded_files,
                progress,
            )?;
            Ok(Some(resolved))
        }
        LoaderKind::NeoForge => {
            let resolved =
                resolve_loader_version(loader_kind, "NeoForge", game_version, modloader_version)?;
            if verify_modloader_profile(instance_root, loader_kind, game_version, &resolved)? {
                tracing::info!(
                    target: "vertexlauncher/installation/modloader",
                    loader = "NeoForge",
                    game_version = %game_version,
                    resolved = %resolved,
                    "Modloader profile already present; skipping installer execution."
                );
                return Ok(Some(resolved));
            }
            emit_installing_modloader_progress("NeoForge", &resolved, *downloaded_files, progress);
            *downloaded_files += install_neoforge_installer(
                instance_root,
                game_version,
                &resolved,
                java_executable,
                policy,
                *downloaded_files,
                progress,
            )?;
            Ok(Some(resolved))
        }
    }
}

fn resolve_loader_version(
    _loader_kind: LoaderKind,
    loader_label: &str,
    game_version: &str,
    requested: Option<&str>,
) -> Result<String, InstallationError> {
    let versions = fetch_loader_versions_for_game(loader_label, game_version, false)?;
    if let Some(value) = requested.map(str::trim).filter(|value| !value.is_empty())
        && !is_latest_loader_version_alias(value)
    {
        let supported = versions.iter().any(|candidate| candidate == value);
        if !supported {
            tracing::warn!(
                target: "vertexlauncher/installation/modloader",
                loader = %loader_label,
                game_version = %game_version,
                requested = %value,
                supported_versions = ?versions,
                "Requested modloader version is not compatible with selected Minecraft version."
            );
            return Err(InstallationError::MissingModloaderVersion {
                loader: loader_label.to_owned(),
                game_version: game_version.to_owned(),
            });
        }
        tracing::info!(
            target: "vertexlauncher/installation/modloader",
            loader = %loader_label,
            game_version = %game_version,
            requested = %value,
            "Using explicitly requested compatible modloader version."
        );
        return Ok(value.to_owned());
    }
    let resolved =
        versions
            .first()
            .cloned()
            .ok_or_else(|| InstallationError::MissingModloaderVersion {
                loader: loader_label.to_owned(),
                game_version: game_version.to_owned(),
            })?;
    tracing::info!(
        target: "vertexlauncher/installation/modloader",
        loader = %loader_label,
        game_version = %game_version,
        resolved = %resolved,
        "Resolved latest compatible modloader version."
    );
    Ok(resolved)
}

fn is_latest_loader_version_alias(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "latest" | "latest available" | "use latest version" | "auto" | "default"
    )
}

fn emit_installing_modloader_progress(
    loader_label: &str,
    loader_version: &str,
    downloaded_files: u32,
    progress: Option<&InstallProgressSink>,
) {
    report_install_progress(
        progress,
        InstallProgress {
            stage: InstallStage::InstallingModloader,
            message: format!("Installing {loader_label} {loader_version} artifacts..."),
            downloaded_files,
            total_files: downloaded_files.max(1),
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: None,
        },
    );
}

fn has_fabric_or_quilt_profile(
    instance_root: &Path,
    game_version: &str,
    loader_kind: LoaderKind,
    loader_version: &str,
) -> Result<bool, InstallationError> {
    let id_prefix = match loader_kind {
        LoaderKind::Fabric => "fabric-loader",
        LoaderKind::Quilt => "quilt-loader",
        _ => return Ok(false),
    };
    let version_id = format!("{id_prefix}-{loader_version}-{game_version}");
    let profile_path = instance_root
        .join("versions")
        .join(version_id.as_str())
        .join(format!("{version_id}.json"));
    if !profile_path.exists() {
        return Ok(false);
    }
    let raw = match fs_read_to_string(profile_path.as_path()) {
        Ok(contents) => contents,
        Err(_) => return Ok(false),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let id = parsed
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if id.eq_ignore_ascii_case(version_id.as_str()) {
        return Ok(true);
    }
    let inherits = parsed
        .get("inheritsFrom")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let game_version_lower = game_version.to_ascii_lowercase();
    let loader_version_lower = loader_version.to_ascii_lowercase();
    let id_lower = id.to_ascii_lowercase();
    Ok(id_lower.contains(loader_version_lower.as_str())
        && id_lower.contains(id_prefix)
        && (inherits == game_version_lower || inherits.starts_with(game_version_lower.as_str())))
}

fn install_fabric_or_quilt_profile(
    instance_root: &Path,
    game_version: &str,
    loader_kind: LoaderKind,
    loader_version: &str,
    policy: &DownloadPolicy,
    downloaded_files_offset: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    let profile_url = match loader_kind {
        LoaderKind::Fabric => format!(
            "{}/{}/{}/profile/json",
            FABRIC_VERSION_MATRIX_URL.trim_end_matches('/'),
            url_encode_component(game_version),
            url_encode_component(loader_version),
        ),
        LoaderKind::Quilt => format!(
            "{}/{}/{}/profile/json",
            QUILT_VERSION_MATRIX_URL.trim_end_matches('/'),
            url_encode_component(game_version),
            url_encode_component(loader_version),
        ),
        _ => return Ok(0),
    };

    let id_prefix = match loader_kind {
        LoaderKind::Fabric => "fabric-loader",
        LoaderKind::Quilt => "quilt-loader",
        _ => "loader",
    };
    let version_id = format!("{id_prefix}-{loader_version}-{game_version}");
    let profile_path = instance_root
        .join("versions")
        .join(version_id.as_str())
        .join(format!("{version_id}.json"));
    let task = FileDownloadTask {
        url: profile_url,
        destination: profile_path,
        expected_size: None,
    };
    download_files_concurrent(
        InstallStage::InstallingModloader,
        vec![task],
        policy,
        downloaded_files_offset,
        progress,
    )
}

fn install_forge_installer(
    instance_root: &Path,
    game_version: &str,
    loader_version: &str,
    java_executable: Option<&str>,
    policy: &DownloadPolicy,
    downloaded_files_offset: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    let artifact_version = format!("{game_version}-{loader_version}");
    let installer_file = format!("forge-{artifact_version}-installer.jar");
    let url = format!(
        "https://maven.minecraftforge.net/net/minecraftforge/forge/{artifact_version}/{installer_file}"
    );
    let destination = instance_root
        .join("loaders")
        .join("forge")
        .join(game_version)
        .join(loader_version)
        .join(installer_file);
    let mut tasks = Vec::new();
    if !destination.exists() {
        tasks.push(FileDownloadTask {
            url,
            destination,
            expected_size: None,
        });
    }
    let downloaded = download_files_concurrent(
        InstallStage::InstallingModloader,
        tasks,
        policy,
        downloaded_files_offset,
        progress,
    )?;
    run_modloader_installer_and_verify(
        instance_root,
        LoaderKind::Forge,
        game_version,
        loader_version,
        java_executable,
    )?;
    Ok(downloaded)
}

fn install_neoforge_installer(
    instance_root: &Path,
    game_version: &str,
    loader_version: &str,
    java_executable: Option<&str>,
    policy: &DownloadPolicy,
    downloaded_files_offset: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    let installer_file = format!("neoforge-{loader_version}-installer.jar");
    let url = format!(
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/{loader_version}/{installer_file}"
    );
    let destination = instance_root
        .join("loaders")
        .join("neoforge")
        .join(game_version)
        .join(loader_version)
        .join(installer_file);
    let mut tasks = Vec::new();
    if !destination.exists() {
        tasks.push(FileDownloadTask {
            url,
            destination,
            expected_size: None,
        });
    }
    let downloaded = download_files_concurrent(
        InstallStage::InstallingModloader,
        tasks,
        policy,
        downloaded_files_offset,
        progress,
    )?;
    run_modloader_installer_and_verify(
        instance_root,
        LoaderKind::NeoForge,
        game_version,
        loader_version,
        java_executable,
    )?;
    Ok(downloaded)
}

fn run_modloader_installer_and_verify(
    instance_root: &Path,
    loader_kind: LoaderKind,
    game_version: &str,
    loader_version: &str,
    java_executable: Option<&str>,
) -> Result<(), InstallationError> {
    let loader_label = match loader_kind {
        LoaderKind::Forge => "Forge",
        LoaderKind::NeoForge => "NeoForge",
        _ => return Ok(()),
    };
    let configured_java = java_executable
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| InstallationError::MissingJavaRuntime {
            loader: loader_label.to_owned(),
        })?;
    let java = normalize_java_executable(Some(configured_java.as_str()));
    if java == "java" && configured_java != "java" {
        tracing::warn!(
            target: "vertexlauncher/installation/modloader",
            "Configured Java path missing ({}), falling back to `java` from PATH.",
            configured_java
        );
    }
    let installer_path =
        find_installer_jar(instance_root, loader_kind, game_version, loader_version)?.ok_or_else(
            || InstallationError::ModloaderInstallOutputMissing {
                loader: loader_label.to_owned(),
                game_version: game_version.to_owned(),
                loader_version: loader_version.to_owned(),
                versions_dir: instance_root.join("versions"),
            },
        )?;
    let installer_path = match fs_canonicalize(installer_path.as_path()) {
        Ok(path) => path,
        Err(_) => installer_path,
    };
    ensure_launcher_profiles(instance_root)?;
    let installer_target =
        fs_canonicalize(instance_root).unwrap_or_else(|_| instance_root.to_path_buf());
    let installer_path = normalize_child_process_path(installer_path.as_path());
    let installer_target = normalize_child_process_path(installer_target.as_path());
    let installer_path_arg = display_user_path(installer_path.as_path());
    let installer_target_arg = display_user_path(installer_target.as_path());

    // Try both flag variants used by Forge/NeoForge installers.
    let mut last_failure = None;
    for flag in ["--installClient", "--install-client"] {
        let mut cmd = Command::new(java.as_str());
        cmd.arg("-jar")
            .arg(installer_path.as_os_str())
            .arg(flag)
            .arg(installer_target.as_os_str())
            .current_dir(installer_target.as_path());
        let command_line = format!(
            "{} -jar {} {} {}",
            java, installer_path_arg, flag, installer_target_arg
        );
        let output = run_command_output(&mut cmd, java.as_str())?;
        if output.status.success() {
            if verify_modloader_profile(instance_root, loader_kind, game_version, loader_version)? {
                return Ok(());
            }
            return Err(InstallationError::ModloaderInstallOutputMissing {
                loader: loader_label.to_owned(),
                game_version: game_version.to_owned(),
                loader_version: loader_version.to_owned(),
                versions_dir: instance_root.join("versions"),
            });
        }
        last_failure = Some((command_line, output.status.code(), output.stderr));
    }

    let (command, status_code, stderr_bytes) = last_failure.unwrap_or_default();
    Err(InstallationError::ModloaderInstallerFailed {
        loader: loader_label.to_owned(),
        game_version: game_version.to_owned(),
        loader_version: loader_version.to_owned(),
        command,
        status: status_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "terminated by signal".to_owned()),
        stderr: String::from_utf8_lossy(&stderr_bytes).trim().to_owned(),
    })
}

fn ensure_launcher_profiles(instance_root: &Path) -> Result<(), InstallationError> {
    let profile_path = instance_root.join("launcher_profiles.json");
    if profile_path.exists() {
        return Ok(());
    }
    let profile = serde_json::json!({
        "profiles": {},
        "selectedProfile": null,
        "clientToken": "vertexlauncher",
        "authenticationDatabase": {},
        "launcherVersion": {
            "name": "Vertex Launcher",
            "format": 21
        },
        "settings": {}
    });
    fs_write(profile_path, serde_json::to_string_pretty(&profile)?)?;
    Ok(())
}

fn find_installer_jar(
    instance_root: &Path,
    loader_kind: LoaderKind,
    game_version: &str,
    loader_version: &str,
) -> Result<Option<PathBuf>, InstallationError> {
    let file_name = match loader_kind {
        LoaderKind::Forge => format!("forge-{game_version}-{loader_version}-installer.jar"),
        LoaderKind::NeoForge => format!("neoforge-{loader_version}-installer.jar"),
        _ => return Ok(None),
    };
    let loader_dir = match loader_kind {
        LoaderKind::Forge => "forge",
        LoaderKind::NeoForge => "neoforge",
        _ => "",
    };
    let path = instance_root
        .join("loaders")
        .join(loader_dir)
        .join(game_version)
        .join(loader_version)
        .join(file_name);
    Ok(path.exists().then_some(path))
}

fn verify_modloader_profile(
    instance_root: &Path,
    loader_kind: LoaderKind,
    game_version: &str,
    loader_version: &str,
) -> Result<bool, InstallationError> {
    let versions_dir = instance_root.join("versions");
    if !versions_dir.exists() {
        return Ok(false);
    }
    let loader_hint = match loader_kind {
        LoaderKind::Forge => "forge",
        LoaderKind::NeoForge => "neoforge",
        _ => return Ok(true),
    };
    for entry in fs_read_dir(&versions_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir_name = entry.file_name();
        let dir_name = dir_name.to_string_lossy();
        let profile_path = entry.path().join(format!("{dir_name}.json"));
        if !profile_path.exists() {
            continue;
        }
        let raw = fs_read_to_string(&profile_path)?;
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let id = parsed
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let inherits = parsed
            .get("inheritsFrom")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let game_version_lower = game_version.to_ascii_lowercase();
        let loader_version_lower = loader_version.to_ascii_lowercase();
        let matches_loader = id.contains(loader_hint)
            || (loader_kind == LoaderKind::NeoForge && id.contains("forge"));
        let matches_version = id.contains(loader_version_lower.as_str());
        let matches_game = id.contains(game_version_lower.as_str())
            || inherits == game_version_lower
            || inherits.starts_with(game_version_lower.as_str());
        if matches_loader && matches_version && matches_game {
            return Ok(true);
        }
    }
    Ok(false)
}

fn cache_root_dir() -> PathBuf {
    app_paths::cache_root()
}

fn canonicalize_existing_path(path: PathBuf) -> PathBuf {
    fs_canonicalize(path.as_path()).unwrap_or(path)
}

fn platform_for_adoptium() -> Result<(&'static str, &'static str, &'static str), InstallationError>
{
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        return Err(InstallationError::UnsupportedPlatform(
            std::env::consts::OS.to_owned(),
        ));
    };
    let arch = current_runtime_architecture().ok_or_else(|| {
        InstallationError::UnsupportedPlatform(
            detected_runtime_architecture().unwrap_or_else(|| std::env::consts::ARCH.to_owned()),
        )
    })?;
    Ok((os, arch.adoptium_value(), arch.cache_key()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeArchitecture {
    X86,
    X64,
    Arm,
    Aarch64,
}

impl RuntimeArchitecture {
    fn adoptium_value(self) -> &'static str {
        match self {
            RuntimeArchitecture::X86 => "x32",
            RuntimeArchitecture::X64 => "x64",
            RuntimeArchitecture::Arm => "arm",
            RuntimeArchitecture::Aarch64 => "aarch64",
        }
    }

    fn cache_key(self) -> &'static str {
        match self {
            RuntimeArchitecture::X86 => "x86",
            RuntimeArchitecture::X64 => "x64",
            RuntimeArchitecture::Arm => "arm",
            RuntimeArchitecture::Aarch64 => "aarch64",
        }
    }
}

fn current_runtime_architecture() -> Option<RuntimeArchitecture> {
    normalize_runtime_architecture(
        detected_runtime_architecture()
            .unwrap_or_else(|| std::env::consts::ARCH.to_owned())
            .as_str(),
    )
}

fn normalize_runtime_architecture(raw: &str) -> Option<RuntimeArchitecture> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "x86" | "i386" | "i486" | "i586" | "i686" => Some(RuntimeArchitecture::X86),
        "x86_64" | "amd64" => Some(RuntimeArchitecture::X64),
        "arm" | "armv7" | "armv7l" => Some(RuntimeArchitecture::Arm),
        "arm64" | "aarch64" => Some(RuntimeArchitecture::Aarch64),
        _ => None,
    }
}

fn detected_runtime_architecture() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        return std::env::var("PROCESSOR_ARCHITEW6432")
            .ok()
            .or_else(|| std::env::var("PROCESSOR_ARCHITECTURE").ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
    }

    #[cfg(target_os = "macos")]
    {
        let uname = command_stdout_trimmed("uname", ["-m"])?;
        if normalize_runtime_architecture(uname.as_str()) == Some(RuntimeArchitecture::X64)
            && command_stdout_trimmed("sysctl", ["-in", "hw.optional.arm64"]).as_deref()
                == Some("1")
        {
            return Some("arm64".to_owned());
        }
        return Some(uname);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return command_stdout_trimmed("uname", ["-m"]);
    }

    #[allow(unreachable_code)]
    None
}

#[cfg(not(target_os = "windows"))]
fn command_stdout_trimmed<const N: usize>(program: &str, args: [&str; N]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

#[cfg(test)]
mod runtime_architecture_tests {
    use super::{RuntimeArchitecture, normalize_runtime_architecture};

    #[test]
    fn normalizes_common_x64_aliases() {
        assert_eq!(
            normalize_runtime_architecture("x86_64"),
            Some(RuntimeArchitecture::X64)
        );
        assert_eq!(
            normalize_runtime_architecture("AMD64"),
            Some(RuntimeArchitecture::X64)
        );
    }

    #[test]
    fn normalizes_common_arm64_aliases() {
        assert_eq!(
            normalize_runtime_architecture("arm64"),
            Some(RuntimeArchitecture::Aarch64)
        );
        assert_eq!(
            normalize_runtime_architecture("aarch64"),
            Some(RuntimeArchitecture::Aarch64)
        );
    }

    #[test]
    fn normalizes_x86_aliases() {
        assert_eq!(
            normalize_runtime_architecture("i686"),
            Some(RuntimeArchitecture::X86)
        );
    }

    #[test]
    fn rejects_unknown_architecture_strings() {
        assert_eq!(normalize_runtime_architecture("sparc64"), None);
    }
}

fn extract_adoptium_package(metadata: &serde_json::Value) -> Option<(String, String)> {
    let package = metadata
        .as_array()?
        .first()?
        .get("binary")?
        .get("package")?;
    let link = package.get("link")?.as_str()?.to_owned();
    let name = package.get("name")?.as_str()?.to_owned();
    Some((link, name))
}

fn download_file_simple(url: &str, destination: &Path) -> Result<(), InstallationError> {
    if destination.exists() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs_create_dir_all(parent)?;
    }
    let response = ureq::get(url)
        .header("User-Agent", OPENJDK_USER_AGENT)
        .call()
        .map_err(map_ureq_error)?;
    let (_, body) = response.into_parts();
    let mut reader = body.into_reader();
    let temp = temporary_download_path(destination);
    let mut file = fs_file_create(&temp)?;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/downloads",
                url,
                temp_path = %temp.display(),
                destination = %destination.display(),
                error = %err,
                "Failed while reading OpenJDK download response body."
            );
            InstallationError::Io(err)
        })?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/downloads",
                url,
                temp_path = %temp.display(),
                destination = %destination.display(),
                error = %err,
                "Failed while writing OpenJDK download chunk to temporary file."
            );
            InstallationError::Io(err)
        })?;
    }
    file.flush().map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/downloads",
            url,
            temp_path = %temp.display(),
            destination = %destination.display(),
            error = %err,
            "Failed while flushing OpenJDK temporary download file."
        );
        InstallationError::Io(err)
    })?;
    fs_rename(temp.as_path(), destination).map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/downloads",
            url,
            temp_path = %temp.display(),
            destination = %destination.display(),
            error = %err,
            "Failed while promoting OpenJDK temporary download file into place."
        );
        err
    })?;
    Ok(())
}

fn temporary_download_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}.downloading"))
        .unwrap_or_else(|| "download.downloading".to_owned());
    destination.with_file_name(file_name)
}

fn extract_archive(archive_path: &Path, destination: &Path) -> Result<(), InstallationError> {
    if destination.exists() {
        fs_remove_dir_all(destination)?;
    }
    fs_create_dir_all(destination)?;
    let file_name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if file_name.ends_with(".zip") {
        let file = fs_file_open(archive_path)?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|err| InstallationError::Io(std::io::Error::other(err.to_string())))?;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|err| InstallationError::Io(std::io::Error::other(err.to_string())))?;
            let Some(enclosed) = entry.enclosed_name() else {
                continue;
            };
            let out_path = destination.join(enclosed);
            if entry.is_dir() {
                fs_create_dir_all(&out_path)?;
                continue;
            }
            if let Some(parent) = out_path.parent() {
                fs_create_dir_all(parent)?;
            }
            let mut out = fs_file_create(&out_path)?;
            std::io::copy(&mut entry, &mut out)?;
        }
        return Ok(());
    }

    if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") {
        let tar_gz = fs_file_open(archive_path)?;
        let decoder = flate2::read::GzDecoder::new(tar_gz);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(destination)?;
        return Ok(());
    }

    Err(InstallationError::Io(std::io::Error::new(
        ErrorKind::InvalidInput,
        format!("unsupported archive format: {}", archive_path.display()),
    )))
}

fn find_java_executable_under(root: &Path) -> Result<Option<PathBuf>, InstallationError> {
    if !root.exists() {
        return Ok(None);
    }
    let binary = if cfg!(target_os = "windows") {
        "java.exe"
    } else {
        "java"
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs_read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.eq_ignore_ascii_case(binary)
                && path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|n| n.to_str())
                    .is_some_and(|part| part.eq_ignore_ascii_case("bin"))
            {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

fn cache_file_path(include_snapshots_and_betas: bool) -> PathBuf {
    let file_name = if include_snapshots_and_betas {
        CACHE_VERSION_CATALOG_ALL_FILE
    } else {
        CACHE_VERSION_CATALOG_RELEASES_FILE
    };
    cache_root_dir().join(file_name)
}

fn read_cached_version_catalog(
    include_snapshots_and_betas: bool,
) -> Result<CachedVersionCatalog, InstallationError> {
    let raw = fs_read_to_string(cache_file_path(include_snapshots_and_betas))?;
    Ok(serde_json::from_str(&raw)?)
}

fn write_cached_version_catalog(
    include_snapshots_and_betas: bool,
    catalog: &VersionCatalog,
) -> Result<(), InstallationError> {
    let path = cache_file_path(include_snapshots_and_betas);
    if let Some(parent) = path.parent() {
        fs_create_dir_all(parent)?;
    }

    let payload = CachedVersionCatalog {
        fetched_at_unix_secs: now_unix_secs(),
        include_snapshots_and_betas,
        catalog: catalog.clone(),
    };
    let file = fs_file_create(path)?;
    serde_json::to_writer_pretty(file, &payload)?;
    Ok(())
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn is_cache_expired(fetched_at_unix_secs: u64) -> bool {
    let now = now_unix_secs();
    now.saturating_sub(fetched_at_unix_secs) > VERSION_CATALOG_CACHE_TTL.as_secs()
}

fn catalog_has_loader_version_data(catalog: &VersionCatalog) -> bool {
    let loader_versions = &catalog.loader_versions;
    [
        &loader_versions.fabric,
        &loader_versions.forge,
        &loader_versions.neoforge,
        &loader_versions.quilt,
    ]
    .into_iter()
    .any(|versions_by_game_version| {
        versions_by_game_version
            .values()
            .any(|versions| !versions.is_empty())
    })
}

fn normalize_version_catalog_ordering(catalog: &mut VersionCatalog) {
    catalog
        .game_versions
        .sort_by(|left, right| compare_version_like_desc(left.id.as_str(), right.id.as_str()));
    catalog.loader_versions.sort_desc();
}

fn fetch_fabric_versions() -> Result<HashSet<String>, InstallationError> {
    let versions: Vec<FabricGameVersion> = get_json(FABRIC_GAME_VERSIONS_URL)?;
    Ok(versions
        .into_iter()
        .map(|version| version.version.trim().to_owned())
        .filter(|version| !version.is_empty())
        .collect())
}

#[derive(Clone, Debug, Default)]
struct LoaderVersionCatalog {
    supported_game_versions: HashSet<String>,
    versions_by_game_version: BTreeMap<String, Vec<String>>,
}

impl LoaderVersionCatalog {
    fn finalize(mut self) -> Self {
        self.supported_game_versions = self.versions_by_game_version.keys().cloned().collect();
        sort_loader_version_map_desc(&mut self.versions_by_game_version);
        self
    }
}

#[derive(Clone, Debug, Default)]
struct LoaderVersionFetchResult {
    selected_versions: Vec<String>,
    versions_by_game_version: BTreeMap<String, Vec<String>>,
}

fn fetch_fabric_loader_catalog() -> Result<LoaderVersionCatalog, InstallationError> {
    let matrix: serde_json::Value = get_json(FABRIC_VERSION_MATRIX_URL)?;
    Ok(parse_loader_version_matrix(&matrix))
}

fn fetch_quilt_versions() -> Result<HashSet<String>, InstallationError> {
    let versions: Vec<QuiltGameVersion> = get_json(QUILT_GAME_VERSIONS_URL)?;
    Ok(versions
        .into_iter()
        .filter_map(|version| {
            let id = version.version.or(version.id)?;
            let trimmed = id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        })
        .collect())
}

fn fetch_quilt_loader_catalog() -> Result<LoaderVersionCatalog, InstallationError> {
    let matrix: serde_json::Value = get_json(QUILT_VERSION_MATRIX_URL)?;
    Ok(parse_loader_version_matrix(&matrix))
}

fn fetch_forge_versions() -> Result<HashSet<String>, InstallationError> {
    let metadata = get_text(FORGE_MAVEN_METADATA_URL)?;
    Ok(parse_minecraft_versions_from_maven_metadata(
        &metadata, true,
    ))
}

fn fetch_forge_loader_catalog() -> Result<LoaderVersionCatalog, InstallationError> {
    let metadata = get_text(FORGE_MAVEN_METADATA_URL)?;
    Ok(parse_forge_loader_catalog_from_metadata(&metadata))
}

fn fetch_neoforge_versions() -> Result<HashSet<String>, InstallationError> {
    let primary = get_text(NEOFORGE_MAVEN_METADATA_URL)?;
    let mut versions = parse_neoforge_versions_from_metadata(&primary);

    if let Ok(legacy) = get_text(NEOFORGE_LEGACY_FORGE_METADATA_URL) {
        versions.extend(parse_minecraft_versions_from_maven_metadata(&legacy, true));
    }

    Ok(versions)
}

fn fetch_neoforge_loader_catalog() -> Result<LoaderVersionCatalog, InstallationError> {
    let primary = get_text(NEOFORGE_MAVEN_METADATA_URL)?;
    let mut catalog = parse_neoforge_loader_catalog_from_metadata(&primary);

    if let Ok(legacy) = get_text(NEOFORGE_LEGACY_FORGE_METADATA_URL) {
        let legacy_neoforge = parse_neoforge_loader_catalog_from_metadata(&legacy);
        merge_loader_catalog(&mut catalog, legacy_neoforge);
        let legacy_forge_style = parse_forge_loader_catalog_from_metadata(&legacy);
        merge_loader_catalog(&mut catalog, legacy_forge_style);
    }

    Ok(catalog)
}

fn fetch_fabric_loader_catalog_with_fallback() -> LoaderVersionCatalog {
    match fetch_fabric_loader_catalog() {
        Ok(catalog) if !catalog.supported_game_versions.is_empty() => catalog,
        _ => LoaderVersionCatalog {
            supported_game_versions: fetch_fabric_versions().unwrap_or_default(),
            ..LoaderVersionCatalog::default()
        },
    }
}

fn fetch_quilt_loader_catalog_with_fallback() -> LoaderVersionCatalog {
    match fetch_quilt_loader_catalog() {
        Ok(catalog) if !catalog.supported_game_versions.is_empty() => catalog,
        _ => LoaderVersionCatalog {
            supported_game_versions: fetch_quilt_versions().unwrap_or_default(),
            ..LoaderVersionCatalog::default()
        },
    }
}

fn fetch_forge_loader_catalog_with_fallback() -> LoaderVersionCatalog {
    match fetch_forge_loader_catalog() {
        Ok(catalog) if !catalog.supported_game_versions.is_empty() => catalog,
        _ => LoaderVersionCatalog {
            supported_game_versions: fetch_forge_versions().unwrap_or_default(),
            ..LoaderVersionCatalog::default()
        },
    }
}

fn fetch_neoforge_loader_catalog_with_fallback() -> LoaderVersionCatalog {
    match fetch_neoforge_loader_catalog() {
        Ok(catalog) if !catalog.supported_game_versions.is_empty() => catalog,
        _ => LoaderVersionCatalog {
            supported_game_versions: fetch_neoforge_versions().unwrap_or_default(),
            ..LoaderVersionCatalog::default()
        },
    }
}

fn parse_minecraft_versions_from_maven_metadata(
    metadata_xml: &str,
    read_prefix_before_dash: bool,
) -> HashSet<String> {
    parse_xml_versions(metadata_xml)
        .into_iter()
        .filter_map(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }

            let candidate = if read_prefix_before_dash {
                trimmed.split('-').next().unwrap_or(trimmed)
            } else {
                trimmed
            };

            if is_probable_minecraft_version(candidate) {
                Some(candidate.to_owned())
            } else {
                None
            }
        })
        .collect()
}

fn parse_loader_version_matrix(matrix: &serde_json::Value) -> LoaderVersionCatalog {
    let mut catalog = LoaderVersionCatalog::default();

    match matrix {
        serde_json::Value::Array(entries) => {
            collect_loader_versions_from_entries(entries, &mut catalog);
        }
        serde_json::Value::Object(object) => {
            // Support alternate wrappers some APIs use.
            for key in ["loader", "versions", "data"] {
                if let Some(entries) = object.get(key).and_then(serde_json::Value::as_array) {
                    collect_loader_versions_from_entries(entries, &mut catalog);
                }
            }
        }
        _ => {}
    }

    catalog.finalize()
}

fn collect_loader_versions_from_entries(
    entries: &[serde_json::Value],
    catalog: &mut LoaderVersionCatalog,
) {
    for entry in entries {
        let Some(entry) = entry.as_object() else {
            continue;
        };

        let Some(game_version) = extract_game_version_from_loader_entry(entry) else {
            continue;
        };
        let Some(loader_version) = extract_loader_version_from_loader_entry(entry) else {
            continue;
        };

        push_unique_loader_version(
            &mut catalog.versions_by_game_version,
            game_version.as_str(),
            loader_version,
        );
    }
}

fn parse_global_loader_versions(matrix: &serde_json::Value) -> Vec<String> {
    let mut versions = Vec::new();
    let mut seen = HashSet::new();
    let mut push_unique = |candidate: String| {
        if seen.insert(candidate.clone()) {
            versions.push(candidate);
        }
    };

    match matrix {
        serde_json::Value::Array(entries) => {
            collect_global_loader_versions_from_entries(entries, &mut push_unique);
        }
        serde_json::Value::Object(object) => {
            let mut found_wrapped_entries = false;
            for key in ["loader", "versions", "data"] {
                if let Some(entries) = object.get(key).and_then(serde_json::Value::as_array) {
                    found_wrapped_entries = true;
                    collect_global_loader_versions_from_entries(entries, &mut push_unique);
                }
            }
            if !found_wrapped_entries {
                if let Some(version) = extract_loader_version_from_loader_entry(object) {
                    push_unique(version);
                } else if let Some(version) = object
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                {
                    push_unique(version);
                }
            }
        }
        _ => {}
    }

    sort_loader_versions_desc(versions)
}

fn collect_global_loader_versions_from_entries<F>(
    entries: &[serde_json::Value],
    push_unique: &mut F,
) where
    F: FnMut(String),
{
    for entry in entries {
        let Some(object) = entry.as_object() else {
            continue;
        };
        if let Some(version) = extract_loader_version_from_loader_entry(object) {
            push_unique(version);
            continue;
        }
        if let Some(version) = object
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
        {
            push_unique(version);
        }
    }
}

fn url_encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        let is_unreserved =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if is_unreserved {
            out.push(byte as char);
        } else {
            use std::fmt::Write as _;

            out.push('%');
            let _ = write!(out, "{byte:02X}");
        }
    }
    out
}

fn fetch_loader_versions_for_game_uncached(
    loader_kind: LoaderKind,
    game_version: &str,
) -> Result<LoaderVersionFetchResult, InstallationError> {
    match loader_kind {
        LoaderKind::Fabric => {
            let url = format!(
                "{}/{}",
                FABRIC_VERSION_MATRIX_URL.trim_end_matches('/'),
                url_encode_component(game_version)
            );
            let payload: serde_json::Value = get_json(&url)?;
            let selected_versions = parse_global_loader_versions(&payload);
            let mut versions_by_game_version = BTreeMap::new();
            versions_by_game_version.insert(game_version.to_owned(), selected_versions.clone());
            Ok(LoaderVersionFetchResult {
                selected_versions,
                versions_by_game_version,
            })
        }
        LoaderKind::Quilt => {
            let url = format!(
                "{}/{}",
                QUILT_VERSION_MATRIX_URL.trim_end_matches('/'),
                url_encode_component(game_version)
            );
            let payload: serde_json::Value = get_json(&url)?;
            let selected_versions = parse_global_loader_versions(&payload);
            let mut versions_by_game_version = BTreeMap::new();
            versions_by_game_version.insert(game_version.to_owned(), selected_versions.clone());
            Ok(LoaderVersionFetchResult {
                selected_versions,
                versions_by_game_version,
            })
        }
        LoaderKind::Forge => {
            let metadata = get_text(FORGE_MAVEN_METADATA_URL)?;
            let catalog = parse_forge_loader_catalog_from_metadata(&metadata);
            let selected_versions = catalog
                .versions_by_game_version
                .get(game_version)
                .cloned()
                .unwrap_or_default();
            Ok(LoaderVersionFetchResult {
                selected_versions,
                versions_by_game_version: catalog.versions_by_game_version,
            })
        }
        LoaderKind::NeoForge => {
            let catalog = fetch_neoforge_loader_catalog()?;
            let selected_versions = catalog
                .versions_by_game_version
                .get(game_version)
                .cloned()
                .unwrap_or_default();
            Ok(LoaderVersionFetchResult {
                selected_versions,
                versions_by_game_version: catalog.versions_by_game_version,
            })
        }
        LoaderKind::Vanilla | LoaderKind::Custom => Ok(LoaderVersionFetchResult::default()),
    }
}

fn loader_versions_cache_file_path(loader_kind: LoaderKind) -> Option<PathBuf> {
    let file_name = match loader_kind {
        LoaderKind::Fabric => "fabric_loader_versions.json",
        LoaderKind::Forge => "forge_loader_versions.json",
        LoaderKind::NeoForge => "neoforge_loader_versions.json",
        LoaderKind::Quilt => "quilt_loader_versions.json",
        LoaderKind::Vanilla | LoaderKind::Custom => return None,
    };
    Some(
        cache_root_dir()
            .join(CACHE_LOADER_VERSIONS_DIR_NAME)
            .join(file_name),
    )
}

fn read_cached_loader_versions(
    loader_kind: LoaderKind,
) -> Result<CachedLoaderVersions, InstallationError> {
    let Some(path) = loader_versions_cache_file_path(loader_kind) else {
        return Ok(CachedLoaderVersions::default());
    };
    let raw = fs_read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn write_cached_loader_versions(
    loader_kind: LoaderKind,
    cached: &CachedLoaderVersions,
) -> Result<(), InstallationError> {
    let Some(path) = loader_versions_cache_file_path(loader_kind) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs_create_dir_all(parent)?;
    }
    let file = fs_file_create(path)?;
    serde_json::to_writer_pretty(file, cached)?;
    Ok(())
}

fn extract_game_version_from_loader_entry(
    entry: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    // Fabric/Quilt loader endpoints commonly encode Minecraft version in "intermediary.version".
    for key in [
        "game",
        "minecraft",
        "minecraft_version",
        "mcversion",
        "intermediary",
    ] {
        if let Some(version) = entry.get(key).and_then(extract_version_from_json_value)
            && is_probable_minecraft_version(version.as_str())
        {
            return Some(version);
        }
    }

    // Fallback: check all object fields for a probable MC version string.
    entry
        .values()
        .find_map(extract_version_from_json_value)
        .filter(|version| is_probable_minecraft_version(version.as_str()))
}

fn extract_loader_version_from_loader_entry(
    entry: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    for key in ["loader", "loader_version", "version"] {
        if let Some(version) = entry.get(key).and_then(extract_version_from_json_value) {
            return Some(version);
        }
    }
    None
}

fn parse_forge_loader_catalog_from_metadata(metadata_xml: &str) -> LoaderVersionCatalog {
    let mut catalog = LoaderVersionCatalog::default();
    for raw in parse_xml_versions(metadata_xml) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((game_version, loader_version)) = trimmed.split_once('-') else {
            continue;
        };
        let game_version = game_version.trim();
        let loader_version = loader_version.trim();
        if game_version.is_empty()
            || loader_version.is_empty()
            || !is_probable_minecraft_version(game_version)
        {
            continue;
        }
        push_unique_loader_version(
            &mut catalog.versions_by_game_version,
            game_version,
            loader_version.to_owned(),
        );
    }
    catalog.finalize()
}

fn parse_neoforge_loader_catalog_from_metadata(metadata_xml: &str) -> LoaderVersionCatalog {
    let mut catalog = LoaderVersionCatalog::default();
    for raw in parse_xml_versions(metadata_xml) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(game_version) = infer_neoforge_game_version(trimmed) else {
            continue;
        };
        push_unique_loader_version(
            &mut catalog.versions_by_game_version,
            game_version.as_str(),
            trimmed.to_owned(),
        );
    }
    catalog.finalize()
}

fn parse_neoforge_versions_from_metadata(metadata_xml: &str) -> HashSet<String> {
    parse_xml_versions(metadata_xml)
        .into_iter()
        .filter_map(|version| {
            let prefix = version.split('-').next().unwrap_or(version.as_str());
            let mut segments = prefix.split('.');
            let major = segments.next()?.parse::<u32>().ok()?;
            let minor = segments.next()?.parse::<u32>().ok()?;
            Some(format!("1.{major}.{minor}"))
        })
        .collect()
}

fn infer_neoforge_game_version(raw: &str) -> Option<String> {
    let prefix = raw.split('-').next().unwrap_or(raw).trim();
    if prefix.is_empty() {
        return None;
    }
    if is_probable_minecraft_version(prefix) {
        return Some(prefix.to_owned());
    }

    let mut segments = prefix.split('.');
    let major = segments.next()?.parse::<u32>().ok()?;
    let minor = segments.next()?.parse::<u32>().ok()?;
    Some(format!("1.{major}.{minor}"))
}

fn extract_version_from_json_value(value: &serde_json::Value) -> Option<String> {
    if let Some(raw) = value.as_str() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed.to_owned());
    }

    let object = value.as_object()?;
    for key in ["version", "id", "name"] {
        let Some(raw) = object.get(key).and_then(serde_json::Value::as_str) else {
            continue;
        };
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    None
}

fn push_unique_loader_version(
    versions_by_game_version: &mut BTreeMap<String, Vec<String>>,
    game_version: &str,
    loader_version: String,
) {
    let versions = versions_by_game_version
        .entry(game_version.to_owned())
        .or_default();
    if !versions.iter().any(|existing| existing == &loader_version) {
        versions.push(loader_version);
    }
}

fn sort_loader_version_map_desc(versions_by_game_version: &mut BTreeMap<String, Vec<String>>) {
    for versions in versions_by_game_version.values_mut() {
        sort_loader_versions_desc_in_place(versions);
    }
}

fn sort_loader_versions_desc(mut versions: Vec<String>) -> Vec<String> {
    sort_loader_versions_desc_in_place(&mut versions);
    versions
}

fn sort_loader_versions_desc_in_place(versions: &mut [String]) {
    versions.sort_by(|left, right| compare_version_like_desc(left.as_str(), right.as_str()));
}

fn compare_version_like_desc(left: &str, right: &str) -> std::cmp::Ordering {
    compare_version_like(left, right).reverse()
}

fn compare_version_like(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let left_tokens = version_like_tokens(left);
    let right_tokens = version_like_tokens(right);

    for (left_token, right_token) in left_tokens.iter().zip(right_tokens.iter()) {
        let ordering = match (left_token, right_token) {
            (VersionToken::Number(left), VersionToken::Number(right)) => left.cmp(right),
            (VersionToken::Text(left), VersionToken::Text(right)) => {
                left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
            }
            (VersionToken::Number(_), VersionToken::Text(_)) => Ordering::Greater,
            (VersionToken::Text(_), VersionToken::Number(_)) => Ordering::Less,
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    left_tokens
        .len()
        .cmp(&right_tokens.len())
        .then_with(|| left.cmp(right))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum VersionToken {
    Number(u64),
    Text(String),
}

fn version_like_tokens(raw: &str) -> Vec<VersionToken> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_is_digit = None;

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            let is_digit = ch.is_ascii_digit();
            match current_is_digit {
                Some(previous) if previous != is_digit => {
                    push_version_token(&mut tokens, &mut current, previous);
                    current_is_digit = Some(is_digit);
                }
                None => current_is_digit = Some(is_digit),
                _ => {}
            }
            current.push(ch);
        } else if let Some(previous) = current_is_digit.take() {
            push_version_token(&mut tokens, &mut current, previous);
        }
    }

    if let Some(previous) = current_is_digit {
        push_version_token(&mut tokens, &mut current, previous);
    }

    tokens
}

fn push_version_token(tokens: &mut Vec<VersionToken>, current: &mut String, was_digit: bool) {
    if current.is_empty() {
        return;
    }
    let token = if was_digit {
        VersionToken::Number(current.parse::<u64>().unwrap_or(0))
    } else {
        VersionToken::Text(current.clone())
    };
    tokens.push(token);
    current.clear();
}

fn merge_loader_catalog(target: &mut LoaderVersionCatalog, source: LoaderVersionCatalog) {
    for game_version in source.supported_game_versions {
        target.supported_game_versions.insert(game_version);
    }
    for (game_version, versions) in source.versions_by_game_version {
        for version in versions {
            push_unique_loader_version(
                &mut target.versions_by_game_version,
                &game_version,
                version,
            );
        }
    }
}

fn parse_xml_versions(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    const START: &str = "<version>";
    const END: &str = "</version>";

    while let Some(start_offset) = xml[cursor..].find(START) {
        let start_index = cursor + start_offset + START.len();
        let Some(end_offset) = xml[start_index..].find(END) else {
            break;
        };
        let end_index = start_index + end_offset;
        out.push(xml[start_index..end_index].to_owned());
        cursor = end_index + END.len();
    }

    out
}

fn map_version_type(raw: &str) -> MinecraftVersionType {
    match raw {
        "release" => MinecraftVersionType::Release,
        "snapshot" => MinecraftVersionType::Snapshot,
        "old_beta" => MinecraftVersionType::OldBeta,
        "old_alpha" => MinecraftVersionType::OldAlpha,
        _ => MinecraftVersionType::Unknown,
    }
}

fn is_probable_minecraft_version(value: &str) -> bool {
    let mut segments = value.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    let Some(second) = segments.next() else {
        return false;
    };
    if !first.chars().all(|ch| ch.is_ascii_digit()) || !second.chars().all(|ch| ch.is_ascii_digit())
    {
        return false;
    }

    if !first.starts_with('1') {
        return false;
    }

    segments.all(|segment| !segment.is_empty() && segment.chars().all(|ch| ch.is_ascii_digit()))
}

fn get_json<T: DeserializeOwned>(url: &str) -> Result<T, InstallationError> {
    let raw = get_text(url)?;
    Ok(serde_json::from_str(&raw)?)
}

fn get_json_with_user_agent<T: DeserializeOwned>(
    url: &str,
    user_agent: &str,
) -> Result<T, InstallationError> {
    let raw = call_get_with_retry(url, user_agent)?;
    Ok(serde_json::from_str(&raw)?)
}

fn http_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT_GLOBAL))
            .timeout_connect(Some(HTTP_TIMEOUT_CONNECT))
            .timeout_recv_response(Some(HTTP_TIMEOUT_RECV_RESPONSE))
            .timeout_recv_body(Some(HTTP_TIMEOUT_RECV_BODY))
            .build();
        ureq::Agent::new_with_config(config)
    })
}

fn get_text(url: &str) -> Result<String, InstallationError> {
    call_get_with_retry(url, DEFAULT_USER_AGENT)
}

fn call_get_response_with_retry(
    url: &str,
    user_agent: &str,
) -> Result<ureq::http::Response<ureq::Body>, InstallationError> {
    let mut last_err = None;
    for attempt in 1..=HTTP_RETRY_ATTEMPTS {
        tracing::trace!(
            target: "vertexlauncher/installation/http",
            url,
            user_agent,
            attempt,
            max_attempts = HTTP_RETRY_ATTEMPTS,
            "Sending HTTP GET request."
        );
        match http_agent()
            .get(url)
            .header("User-Agent", user_agent)
            .config()
            .http_status_as_error(false)
            .build()
            .call()
        {
            Ok(mut response) => {
                let status = response.status().as_u16();
                tracing::trace!(
                    target: "vertexlauncher/installation/http",
                    url,
                    attempt,
                    status,
                    "HTTP GET request completed."
                );
                if status < 400 {
                    return Ok(response);
                }
                let mut body = String::new();
                let _ = response.body_mut().as_reader().read_to_string(&mut body);
                let err = InstallationError::HttpStatus {
                    url: url.to_owned(),
                    status,
                    body,
                };
                let retryable = should_retry_http_status(status);
                if !retryable || attempt >= HTTP_RETRY_ATTEMPTS {
                    return Err(err);
                }
                last_err = Some(err);
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/installation/http",
                    url,
                    attempt,
                    error = %err,
                    "HTTP GET request failed before a valid response was received."
                );
                let mapped = InstallationError::Transport {
                    url: url.to_owned(),
                    message: err.to_string(),
                };
                if attempt >= HTTP_RETRY_ATTEMPTS {
                    return Err(mapped);
                }
                last_err = Some(mapped);
            }
        }

        let delay = retry_delay_for_attempt(attempt);
        tracing::warn!(
            target: "vertexlauncher/installation/downloads",
            "Request retry {}/{} for {} after {:?}: {}",
            attempt,
            HTTP_RETRY_ATTEMPTS,
            url,
            delay,
            last_err
                .as_ref()
                .map_or_else(|| "request failed".to_owned(), ToString::to_string)
        );
        thread::sleep(delay);
    }

    Err(last_err.unwrap_or_else(|| InstallationError::Transport {
        url: url.to_owned(),
        message: "request failed without detailed error".to_owned(),
    }))
}

fn call_get_with_retry(url: &str, user_agent: &str) -> Result<String, InstallationError> {
    let mut response = call_get_response_with_retry(url, user_agent)?;
    let mut raw = String::new();
    response
        .body_mut()
        .as_reader()
        .read_to_string(&mut raw)
        .map_err(InstallationError::Io)?;
    Ok(raw)
}

fn should_retry_http_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429) || (500..=599).contains(&status)
}

fn retry_delay_for_attempt(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(5);
    let multiplier = 1u64 << exponent;
    let millis = HTTP_RETRY_BASE_DELAY_MS
        .saturating_mul(multiplier)
        .min(5_000);
    Duration::from_millis(millis)
}

fn map_ureq_error(error: ureq::Error) -> InstallationError {
    match error {
        ureq::Error::StatusCode(status) => InstallationError::HttpStatus {
            url: "<unknown>".to_owned(),
            status,
            body: String::new(),
        },
        other => InstallationError::Transport {
            url: "<transport>".to_owned(),
            message: other.to_string(),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoaderKind {
    Vanilla,
    Fabric,
    Forge,
    NeoForge,
    Quilt,
    Custom,
}

fn normalized_loader_label(loader_label: &str) -> LoaderKind {
    match loader_label.trim().to_ascii_lowercase().as_str() {
        "vanilla" => LoaderKind::Vanilla,
        "fabric" => LoaderKind::Fabric,
        "forge" => LoaderKind::Forge,
        "neoforge" => LoaderKind::NeoForge,
        "quilt" => LoaderKind::Quilt,
        _ => LoaderKind::Custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_windows_cli_paths() {
        assert_eq!(
            normalize_windows_cli_path(r"\\?\C:\Users\clove\AppData\Local\vertexlauncher"),
            r"C:\Users\clove\AppData\Local\vertexlauncher"
        );
        assert_eq!(
            normalize_windows_cli_path(r"\\?\UNC\server\share\vertexlauncher"),
            r"\\server\share\vertexlauncher"
        );
        assert_eq!(
            normalize_windows_cli_path(r"C:\Users\clove\AppData\Local\vertexlauncher"),
            r"C:\Users\clove\AppData\Local\vertexlauncher"
        );
    }

    #[test]
    fn parses_loader_matrix_entries_from_array() {
        let payload = serde_json::json!([
            {
                "loader": { "version": "0.16.5" },
                "intermediary": { "version": "1.21.1" }
            },
            {
                "loader": { "version": "0.16.4" },
                "intermediary": { "version": "1.21.1" }
            }
        ]);

        let catalog = parse_loader_version_matrix(&payload);
        let versions = catalog
            .versions_by_game_version
            .get("1.21.1")
            .expect("expected versions for 1.21.1");
        assert!(versions.iter().any(|entry| entry == "0.16.5"));
        assert!(versions.iter().any(|entry| entry == "0.16.4"));
    }

    #[test]
    fn parses_loader_matrix_entries_from_loader_wrapped_object() {
        let payload = serde_json::json!({
            "loader": [
                {
                    "loader": { "version": "0.1.2" },
                    "intermediary": { "version": "1.20.6" }
                }
            ]
        });

        let catalog = parse_loader_version_matrix(&payload);
        let versions = catalog
            .versions_by_game_version
            .get("1.20.6")
            .expect("expected versions for 1.20.6");
        assert_eq!(versions, &vec!["0.1.2".to_owned()]);
    }

    #[test]
    fn parses_global_loader_versions_when_matrix_has_no_game_mapping() {
        let payload = serde_json::json!([
            {
                "loader": { "version": "0.16.9" }
            },
            {
                "loader": { "version": "0.16.10" }
            }
        ]);

        let versions = parse_global_loader_versions(&payload);
        assert_eq!(versions, vec!["0.16.10".to_owned(), "0.16.9".to_owned()]);
    }

    #[test]
    fn sorts_loader_versions_descending() {
        let versions = sort_loader_versions_desc(vec![
            "21.0.1-beta".to_owned(),
            "21.0.10".to_owned(),
            "21.0.2".to_owned(),
        ]);

        assert_eq!(
            versions,
            vec![
                "21.0.10".to_owned(),
                "21.0.2".to_owned(),
                "21.0.1-beta".to_owned(),
            ]
        );
    }

    #[test]
    fn url_encoding_covers_spaces_and_symbols() {
        assert_eq!(
            url_encode_component("1.14 Pre-Release 5"),
            "1.14%20Pre-Release%205"
        );
        assert_eq!(url_encode_component("a/b"), "a%2Fb");
    }

    #[test]
    fn eta_tracks_progress_fraction_deltas() {
        let mut state = ProgressEtaState::default();

        assert_eq!(
            state.observe(ProgressEtaPoint {
                fraction: 0.25,
                at_millis: 1_000,
            }),
            None
        );
        assert_eq!(
            state.observe(ProgressEtaPoint {
                fraction: 0.50,
                at_millis: 6_000,
            }),
            Some(10)
        );
    }

    #[test]
    fn eta_resets_when_progress_fraction_regresses() {
        let mut state = ProgressEtaState::default();
        let _ = state.observe(ProgressEtaPoint {
            fraction: 0.75,
            at_millis: 2_000,
        });
        let _ = state.observe(ProgressEtaPoint {
            fraction: 0.90,
            at_millis: 5_000,
        });

        assert_eq!(
            state.observe(ProgressEtaPoint {
                fraction: 0.40,
                at_millis: 6_000,
            }),
            None
        );
    }

    #[test]
    fn eta_reuses_last_estimate_when_fraction_does_not_move() {
        let mut state = ProgressEtaState::default();
        let _ = state.observe(ProgressEtaPoint {
            fraction: 0.10,
            at_millis: 1_000,
        });
        assert_eq!(
            state.observe(ProgressEtaPoint {
                fraction: 0.30,
                at_millis: 3_000,
            }),
            Some(7)
        );
        assert_eq!(
            state.observe(ProgressEtaPoint {
                fraction: 0.30,
                at_millis: 3_500,
            }),
            Some(7)
        );
    }

    #[test]
    fn quick_play_requested_singleplayer_is_appended_when_profile_has_no_flags() {
        let mut substitutions = HashMap::new();
        substitutions.insert("quickPlayPath".to_owned(), "/tmp/qp".to_owned());
        substitutions.insert("quickPlaySingleplayer".to_owned(), "MyWorld".to_owned());
        let context = LaunchContext {
            substitutions,
            features: HashMap::new(),
        };
        let args = vec!["--demo".to_owned()];
        let normalized = normalize_quick_play_arguments(args, &context);
        assert_eq!(
            normalized,
            vec![
                "--demo".to_owned(),
                "--quickPlayPath".to_owned(),
                "/tmp/qp".to_owned(),
                "--quickPlaySingleplayer".to_owned(),
                "MyWorld".to_owned(),
            ]
        );
    }

    #[test]
    fn quick_play_keeps_single_mode_and_filters_duplicates() {
        let mut substitutions = HashMap::new();
        substitutions.insert("quickPlayPath".to_owned(), "/tmp/qp".to_owned());
        substitutions.insert("quickPlayMultiplayer".to_owned(), "example.org".to_owned());
        let context = LaunchContext {
            substitutions,
            features: HashMap::new(),
        };
        let args = vec![
            "--quickPlayPath".to_owned(),
            "/tmp/qp".to_owned(),
            "--quickPlaySingleplayer".to_owned(),
            "WorldA".to_owned(),
            "--quickPlayMultiplayer".to_owned(),
            "example.org".to_owned(),
        ];
        let normalized = normalize_quick_play_arguments(args, &context);
        assert_eq!(
            normalized,
            vec![
                "--quickPlayPath".to_owned(),
                "/tmp/qp".to_owned(),
                "--quickPlaySingleplayer".to_owned(),
                "WorldA".to_owned(),
            ]
        );
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MojangVersionManifest {
    versions: Vec<MojangVersionEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MojangVersionEntry {
    id: String,
    #[serde(rename = "type")]
    version_type: String,
    release_time: String,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FabricGameVersion {
    version: String,
}

#[derive(Debug, Deserialize)]
struct QuiltGameVersion {
    version: Option<String>,
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MojangVersionMeta {
    downloads: Option<MojangDownloads>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MojangDownloads {
    client: Option<MojangDownloadArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MojangDownloadArtifact {
    url: String,
    size: Option<u64>,
}
