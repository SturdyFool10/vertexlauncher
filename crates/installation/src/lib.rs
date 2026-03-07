use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_USER_AGENT: &str =
    "VertexLauncher/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";
const MOJANG_VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const FABRIC_VERSION_MATRIX_URL: &str = "https://meta.fabricmc.net/v2/versions/loader";
const FABRIC_GAME_VERSIONS_URL: &str = "https://meta.fabricmc.net/v2/versions/game";
const QUILT_VERSION_MATRIX_URL: &str = "https://meta.quiltmc.org/v3/versions/loader";
const QUILT_GAME_VERSIONS_URL: &str = "https://meta.quiltmc.org/v3/versions/game";
const FORGE_MAVEN_METADATA_URL: &str =
    "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml";
const NEOFORGE_MAVEN_METADATA_URL: &str =
    "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml";
const NEOFORGE_LEGACY_FORGE_METADATA_URL: &str =
    "https://maven.neoforged.net/releases/net/neoforged/forge/maven-metadata.xml";
const CACHE_VERSION_CATALOG_RELEASES_FILE: &str = "version_catalog_release_only.json";
const CACHE_VERSION_CATALOG_ALL_FILE: &str = "version_catalog_with_snapshots.json";
const CACHE_LOADER_VERSIONS_DIR_NAME: &str = "loader_versions";
const CACHE_DIR_NAME: &str = "cache";
const VERSION_CATALOG_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const HTTP_RETRY_ATTEMPTS: u32 = 4;
const HTTP_RETRY_BASE_DELAY_MS: u64 = 350;
const OPENJDK_USER_AGENT: &str =
    "VertexLauncher-JavaProvisioner/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";

#[track_caller]
fn fs_create_dir_all(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display());
    fs::create_dir_all(path)
}

#[track_caller]
fn fs_remove_dir_all(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "remove_dir_all", path = %path.display());
    fs::remove_dir_all(path)
}

#[track_caller]
fn fs_read_to_string(path: impl AsRef<Path>) -> std::io::Result<String> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    fs::read_to_string(path)
}

#[track_caller]
fn fs_read_dir(path: impl AsRef<Path>) -> std::io::Result<fs::ReadDir> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read_dir", path = %path.display());
    fs::read_dir(path)
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
    fs::rename(from, to)
}

#[track_caller]
fn fs_canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "canonicalize", path = %path.display());
    fs::canonicalize(path)
}

#[track_caller]
fn fs_write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "write", path = %path.display());
    fs::write(path, contents)
}

#[track_caller]
fn fs_file_create(path: impl AsRef<Path>) -> std::io::Result<fs::File> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "file_create", path = %path.display());
    fs::File::create(path)
}

#[track_caller]
fn fs_file_open(path: impl AsRef<Path>) -> std::io::Result<fs::File> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "file_open", path = %path.display());
    fs::File::open(path)
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchResult {
    pub pid: u32,
    pub profile_id: String,
    pub launch_log_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadPolicy {
    pub max_concurrent_downloads: u32,
    pub max_download_bps: Option<u64>,
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
        "{loader} installer did not produce a usable version profile for Minecraft {game_version} ({loader_version}) in {versions_dir}"
    )]
    ModloaderInstallOutputMissing {
        loader: String,
        game_version: String,
        loader_version: String,
        versions_dir: String,
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
    #[error("Launch profile {profile_id} is missing required file: {path}")]
    LaunchFileMissing { profile_id: String, path: String },
    #[error("Minecraft exited immediately (status: {status}). See launch log: {log_path}")]
    LaunchExitedImmediately { status: String, log_path: String },
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
        return Ok(cached.catalog.clone());
    }

    match fetch_version_catalog_uncached(include_snapshots_and_betas) {
        Ok(catalog) => {
            let _ = write_cached_version_catalog(include_snapshots_and_betas, &catalog);
            Ok(catalog)
        }
        Err(err) => {
            if let Some(cached) = cached {
                Ok(cached.catalog)
            } else {
                Err(err)
            }
        }
    }
}

pub fn ensure_openjdk_runtime(runtime_major: u8) -> Result<PathBuf, InstallationError> {
    let (os, arch) = platform_for_adoptium()?;
    let install_root = cache_root_dir()
        .join("java")
        .join(format!("openjdk-{runtime_major}"));
    if let Some(existing) = find_java_executable_under(install_root.as_path())? {
        return Ok(existing);
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

    find_java_executable_under(install_root.as_path())?
        .ok_or(InstallationError::OpenJdkMetadataMissing { runtime_major })
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
        return Ok(versions.clone());
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
            let _ = write_cached_loader_versions(loader_kind, &updated_cache);
            Ok(selected_versions)
        }
        Err(err) => {
            if let Some(cached) = cached
                && let Some(versions) = cached.versions_by_game_version.get(game_version)
            {
                Ok(versions.clone())
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
        let fabric = fabric_task.join().unwrap_or_default();
        let forge = forge_task.join().unwrap_or_default();
        let neoforge = neoforge_task.join().unwrap_or_default();
        let quilt = quilt_task.join().unwrap_or_default();
        Ok::<_, InstallationError>((manifest, fabric, forge, neoforge, quilt))
    })?;

    let game_versions: Vec<MinecraftVersionEntry> = manifest
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
                Some(MinecraftVersionEntry {
                    id: entry.id,
                    version_type,
                })
            } else {
                None
            }
        })
        .collect();

    let loader_support = LoaderSupportIndex {
        fabric: fabric.supported_game_versions,
        forge: forge.supported_game_versions,
        neoforge: neoforge.supported_game_versions,
        quilt: quilt.supported_game_versions,
    };
    let loader_versions = LoaderVersionIndex {
        fabric: fabric.versions_by_game_version,
        forge: forge.versions_by_game_version,
        neoforge: neoforge.versions_by_game_version,
        quilt: quilt.versions_by_game_version,
    };

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
        )?;
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
    )?;
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
    )?;
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
            )?;
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

static RUNNING_INSTANCE_PROCESSES: OnceLock<Mutex<HashMap<String, RunningInstanceProcess>>> =
    OnceLock::new();

fn process_registry() -> &'static Mutex<HashMap<String, RunningInstanceProcess>> {
    RUNNING_INSTANCE_PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn launch_instance(request: &LaunchRequest) -> Result<LaunchResult, InstallationError> {
    let instance_root = fs_canonicalize(request.instance_root.as_path())
        .unwrap_or_else(|_| request.instance_root.clone());
    let instance_key = instance_root.display().to_string();
    let requested_account = normalize_account_key(request.account_key.as_deref());
    if let Ok(mut processes) = process_registry().lock() {
        prune_finished_processes(&mut processes);
        if let Some(process) = processes.get_mut(instance_key.as_str()) {
            if let Ok(None) = process.child.try_wait() {
                return Err(InstallationError::InstanceAlreadyRunning {
                    instance_root: instance_key,
                    pid: process.child.id(),
                });
            }
        }
        if let Some(account) = requested_account.as_deref() {
            for (running_instance_root, process) in processes.iter_mut() {
                if running_instance_root == &instance_key {
                    continue;
                }
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
    let classpath = build_classpath(
        instance_root.as_path(),
        profile_id.as_str(),
        request.game_version.as_str(),
        &profile_chain,
    )?;
    let (mut launch_log_file, launch_log_path) = prepare_launch_log_file(instance_root.as_path())?;
    let launch_log_for_error = launch_log_path.display().to_string();
    let _ = writeln!(
        launch_log_file,
        "[vertexlauncher] Launching Minecraft {} with profile {} in {}",
        request.game_version,
        profile_id,
        instance_root.display()
    );
    let stderr_log = launch_log_file.try_clone()?;

    let mut command = Command::new(java.as_str());
    command
        .current_dir(instance_root.as_path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(launch_log_file))
        .stderr(Stdio::from(stderr_log));

    command.arg(format!("-Xmx{}M", request.max_memory_mib.max(512)));
    for arg in parse_user_args(request.extra_jvm_args.as_deref()) {
        command.arg(arg);
    }

    let launch_context = build_launch_context(
        instance_root.as_path(),
        request.game_version.as_str(),
        profile_id.as_str(),
        resolve_assets_index_name(&profile_chain, request.game_version.as_str()),
        classpath.as_str(),
        natives_dir.as_path(),
        request.player_name.as_deref(),
        request.player_uuid.as_deref(),
        request.auth_access_token.as_deref(),
        request.auth_xuid.as_deref(),
        request.auth_user_type.as_deref(),
    );

    let mut jvm_args = collect_jvm_arguments(&profile_chain, &launch_context);
    if !jvm_args.iter().any(|arg| arg.contains("${classpath}")) {
        jvm_args.push("-cp".to_owned());
        jvm_args.push(classpath);
    }
    for arg in jvm_args {
        command.arg(arg);
    }

    command.arg(main_class);

    let game_args = collect_game_arguments(&profile_chain, &launch_context);
    for arg in game_args {
        command.arg(arg);
    }

    let mut child = spawn_command_child(&mut command, java.as_str())?;
    thread::sleep(Duration::from_millis(1200));
    if let Some(status) = child.try_wait()? {
        return Err(InstallationError::LaunchExitedImmediately {
            status: status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated by signal".to_owned()),
            log_path: launch_log_for_error,
        });
    }
    let pid = child.id();
    if let Ok(mut processes) = process_registry().lock() {
        processes.insert(
            instance_root.display().to_string(),
            RunningInstanceProcess {
                child,
                account_key: requested_account,
            },
        );
    }
    Ok(LaunchResult {
        pid,
        profile_id,
        launch_log_path,
    })
}

pub fn stop_running_instance(instance_root: &Path) -> bool {
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return false;
    };
    let Some(mut process) = processes.remove(key.as_str()) else {
        return false;
    };
    let _ = process.child.kill();
    let _ = process.child.wait();
    true
}

pub fn is_instance_running(instance_root: &Path) -> bool {
    let key = instance_process_key(instance_root);
    let Ok(mut processes) = process_registry().lock() else {
        return false;
    };
    prune_finished_processes(&mut processes);
    if let Some(process) = processes.get_mut(key.as_str())
        && let Ok(None) = process.child.try_wait()
    {
        return true;
    }
    false
}

pub fn running_instance_for_account(account_key: &str) -> Option<String> {
    let account = normalize_account_key(Some(account_key))?;
    let Ok(mut processes) = process_registry().lock() else {
        return None;
    };
    prune_finished_processes(&mut processes);
    processes.iter_mut().find_map(|(instance_root, process)| {
        if process
            .account_key
            .as_deref()
            .is_some_and(|value| value == account)
            && matches!(process.child.try_wait(), Ok(None))
        {
            Some(instance_root.clone())
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

fn prune_finished_processes(processes: &mut HashMap<String, RunningInstanceProcess>) {
    processes.retain(|_, process| matches!(process.child.try_wait(), Ok(None)));
}

fn instance_process_key(instance_root: &Path) -> String {
    fs_canonicalize(instance_root)
        .unwrap_or_else(|_| instance_root.to_path_buf())
        .display()
        .to_string()
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
                java = canonical.display().to_string();
            } else if let Ok(cwd) = std::env::current_dir() {
                java = cwd.join(java_path).display().to_string();
            }
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

fn build_classpath(
    instance_root: &Path,
    profile_id: &str,
    game_version: &str,
    chain: &[serde_json::Value],
) -> Result<String, InstallationError> {
    let mut classpath = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();

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
                let key = full.display().to_string();
                if seen.insert(key.clone()) {
                    classpath.push(key);
                }
            }
        }
    }

    let launch_jar = instance_root
        .join("versions")
        .join(profile_id)
        .join(format!("{profile_id}.jar"));
    let fallback_jar = instance_root
        .join("versions")
        .join(game_version)
        .join(format!("{game_version}.jar"));
    let selected_jar = if launch_jar.exists() {
        launch_jar
    } else if fallback_jar.exists() {
        fallback_jar
    } else {
        return Err(InstallationError::LaunchFileMissing {
            profile_id: profile_id.to_owned(),
            path: launch_jar.display().to_string(),
        });
    };
    classpath.push(selected_jar.display().to_string());
    Ok(classpath.join(classpath_separator()))
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
        instance_root.display().to_string(),
    );
    substitutions.insert(
        "assets_root".to_owned(),
        instance_root.join("assets").display().to_string(),
    );
    substitutions.insert("assets_index_name".to_owned(), assets_index_name.to_owned());
    substitutions.insert("auth_uuid".to_owned(), uuid.to_owned());
    substitutions.insert("auth_access_token".to_owned(), access_token.to_owned());
    substitutions.insert("clientid".to_owned(), "0".to_owned());
    substitutions.insert("auth_xuid".to_owned(), xuid.to_owned());
    substitutions.insert("user_type".to_owned(), user_type.to_owned());
    substitutions.insert("version_type".to_owned(), "release".to_owned());
    substitutions.insert("user_properties".to_owned(), "{}".to_owned());
    substitutions.insert("classpath".to_owned(), classpath.to_owned());
    substitutions.insert(
        "library_directory".to_owned(),
        instance_root.join("libraries").display().to_string(),
    );
    substitutions.insert(
        "natives_directory".to_owned(),
        natives_dir.display().to_string(),
    );
    substitutions.insert("launcher_name".to_owned(), "vertexlauncher".to_owned());
    substitutions.insert("launcher_version".to_owned(), "0.1".to_owned());
    let mut features = HashMap::new();
    features.insert("is_demo_user".to_owned(), false);
    features.insert("has_custom_resolution".to_owned(), false);
    features.insert("has_quick_plays_support".to_owned(), false);
    features.insert("is_quick_play_singleplayer".to_owned(), false);
    features.insert("is_quick_play_multiplayer".to_owned(), false);
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
    normalize_quick_play_arguments(resolved)
}

fn normalize_quick_play_arguments(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
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
        if value.trim().is_empty() || unresolved_placeholder || quick_play_mode_selected {
            cursor = cursor.saturating_add(2);
            continue;
        }

        out.push(args[cursor].clone());
        out.push(args[cursor + 1].clone());
        quick_play_mode_selected = true;
        cursor += 2;
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
    let Some(name) = os_object.get("name").and_then(serde_json::Value::as_str) else {
        return true;
    };
    name == current_os_natives_key()
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
    let raw = fs_read_to_string(version_json_path)?;
    let version_meta: serde_json::Value = serde_json::from_str(&raw)?;
    let mut downloaded = 0u32;

    let mut library_tasks = Vec::new();
    collect_library_download_tasks(instance_root, &version_meta, &mut library_tasks);
    downloaded += download_files_concurrent(
        InstallStage::DownloadingCore,
        library_tasks,
        policy,
        downloaded_files_offset.saturating_add(downloaded),
        progress,
    )?;

    let mut asset_index_task = Vec::new();
    let asset_index_path =
        collect_asset_index_download_task(instance_root, &version_meta, &mut asset_index_task);
    downloaded += download_files_concurrent(
        InstallStage::DownloadingCore,
        asset_index_task,
        policy,
        downloaded_files_offset.saturating_add(downloaded),
        progress,
    )?;

    if let Some(asset_index_path) = asset_index_path {
        let mut object_tasks = Vec::new();
        collect_asset_object_download_tasks(
            instance_root,
            asset_index_path.as_path(),
            &mut object_tasks,
        )?;
        downloaded += download_files_concurrent(
            InstallStage::DownloadingCore,
            object_tasks,
            policy,
            downloaded_files_offset.saturating_add(downloaded),
            progress,
        )?;
    }

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
    let raw = fs_read_to_string(asset_index_path)?;
    let index: serde_json::Value = serde_json::from_str(&raw)?;
    let Some(objects) = index.get("objects").and_then(serde_json::Value::as_object) else {
        return Ok(());
    };
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
        tasks.push(FileDownloadTask {
            url: format!("https://resources.download.minecraft.net/{prefix}/{hash}"),
            destination,
            expected_size: entry
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .filter(|size| *size > 0),
        });
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
        }
    }
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
    let eta_seconds = if known_total_bytes > downloaded_bytes && bytes_per_second > 1.0 {
        Some(((known_total_bytes - downloaded_bytes) as f64 / bytes_per_second).ceil() as u64)
    } else {
        None
    };
    let total_bytes = (known_total_bytes > 0).then_some(known_total_bytes);

    progress(InstallProgress {
        stage,
        message: format!(
            "Downloading files ({}/{})...",
            downloaded_files_offset.saturating_add(completed_files),
            downloaded_files_offset.saturating_add(telemetry.total_files)
        ),
        downloaded_files: downloaded_files_offset.saturating_add(completed_files),
        total_files: downloaded_files_offset.saturating_add(telemetry.total_files),
        downloaded_bytes,
        total_bytes,
        bytes_per_second,
        eta_seconds,
    });
}

fn prefetch_batch_total_bytes(tasks: &mut [FileDownloadTask], probe_workers: usize) -> u64 {
    let mut total_known_bytes = 0u64;
    let mut unknown = std::collections::VecDeque::new();
    for (index, task) in tasks.iter().enumerate() {
        if let Some(size) = task.expected_size {
            total_known_bytes = total_known_bytes.saturating_add(size);
        } else {
            unknown.push_back((index, task.url.clone()));
        }
    }
    if unknown.is_empty() {
        return total_known_bytes;
    }

    let queue = Arc::new(Mutex::new(unknown));
    let discovered = Arc::new(Mutex::new(Vec::<(usize, u64)>::new()));
    thread::scope(|scope| {
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
            let _ = worker.join();
        }
    });

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

    total_known_bytes
}

fn probe_content_length(url: &str) -> Option<u64> {
    let response = match http_agent()
        .request("HEAD", url)
        .set("User-Agent", DEFAULT_USER_AGENT)
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::Status(status, _)) => {
            tracing::debug!(
                target: "vertexlauncher/installation/downloads",
                "Size prefetch HEAD failed for {} with status {}",
                url,
                status
            );
            return None;
        }
        Err(ureq::Error::Transport(transport)) => {
            tracing::debug!(
                target: "vertexlauncher/installation/downloads",
                "Size prefetch HEAD transport error for {}: {}",
                url,
                transport
            );
            return None;
        }
    };
    response
        .header("Content-Length")
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
    let prefetched_total_bytes = prefetch_batch_total_bytes(&mut tasks, size_probe_workers);
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

    let response = call_get_with_retry(task.url.as_str(), DEFAULT_USER_AGENT)?;
    if task.expected_size.is_none()
        && let Some(content_length) = response
            .header("Content-Length")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|size| *size > 0)
    {
        telemetry
            .known_total_bytes
            .fetch_add(content_length, Ordering::Relaxed);
    }

    let temp_path = task.destination.with_extension("downloading");
    let mut reader = response.into_reader();
    let mut file = fs_file_create(&temp_path)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
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
        file.write_all(&buffer[..read])?;
    }
    file.flush()?;
    fs_rename(temp_path, &task.destination)?;
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
                versions_dir: instance_root.join("versions").display().to_string(),
            },
        )?;
    let installer_path = match fs_canonicalize(installer_path.as_path()) {
        Ok(path) => path,
        Err(_) => installer_path,
    };
    ensure_launcher_profiles(instance_root)?;
    let installer_target =
        fs_canonicalize(instance_root).unwrap_or_else(|_| instance_root.to_path_buf());

    // Try both flag variants used by Forge/NeoForge installers.
    let mut last_failure = None;
    for flag in ["--installClient", "--install-client"] {
        let mut cmd = Command::new(java.as_str());
        cmd.arg("-jar")
            .arg(installer_path.as_path())
            .arg(flag)
            .arg(installer_target.as_path())
            .current_dir(installer_target.as_path());
        let command_line = format!(
            "{} -jar {} {} {}",
            java,
            installer_path.display(),
            flag,
            installer_target.display()
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
                versions_dir: instance_root.join("versions").display().to_string(),
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
    match std::env::var("VERTEX_CONFIG_LOCATION") {
        Ok(dir) => PathBuf::from(dir).join(CACHE_DIR_NAME),
        Err(_) => PathBuf::from(CACHE_DIR_NAME),
    }
}

fn platform_for_adoptium() -> Result<(&'static str, &'static str), InstallationError> {
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
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        return Err(InstallationError::UnsupportedPlatform(
            std::env::consts::ARCH.to_owned(),
        ));
    };
    Ok((os, arch))
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
        .set("User-Agent", OPENJDK_USER_AGENT)
        .call()
        .map_err(map_ureq_error)?;
    let mut reader = response.into_reader();
    let temp = destination.with_extension("downloading");
    let mut file = fs_file_create(&temp)?;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
    }
    file.flush()?;
    fs_rename(temp, destination)?;
    Ok(())
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

    catalog.supported_game_versions = catalog.versions_by_game_version.keys().cloned().collect();
    catalog
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
    let mut push_unique = |candidate: String| {
        if !versions.iter().any(|existing| existing == &candidate) {
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

    versions
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
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
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
    catalog.supported_game_versions = catalog.versions_by_game_version.keys().cloned().collect();
    catalog
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
    catalog.supported_game_versions = catalog.versions_by_game_version.keys().cloned().collect();
    catalog
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
    let response = call_get_with_retry(url, user_agent)?;
    let raw = response.into_string().map_err(InstallationError::Io)?;
    Ok(serde_json::from_str(&raw)?)
}

fn http_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(30))
            .timeout_write(Duration::from_secs(30))
            .build()
    })
}

fn get_text(url: &str) -> Result<String, InstallationError> {
    let response = call_get_with_retry(url, DEFAULT_USER_AGENT)?;

    response.into_string().map_err(InstallationError::Io)
}

fn call_get_with_retry(url: &str, user_agent: &str) -> Result<ureq::Response, InstallationError> {
    let mut last_err = None;
    for attempt in 1..=HTTP_RETRY_ATTEMPTS {
        match http_agent().get(url).set("User-Agent", user_agent).call() {
            Ok(response) => return Ok(response),
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
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
            Err(ureq::Error::Transport(transport)) => {
                let err = InstallationError::Transport {
                    url: url.to_owned(),
                    message: transport.to_string(),
                };
                if attempt >= HTTP_RETRY_ATTEMPTS {
                    return Err(err);
                }
                last_err = Some(err);
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
                .map(ToString::to_string)
                .unwrap_or_else(|| "request failed".to_owned())
        );
        thread::sleep(delay);
    }
    Err(last_err.unwrap_or_else(|| InstallationError::Transport {
        url: url.to_owned(),
        message: "request failed without detailed error".to_owned(),
    }))
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
        ureq::Error::Status(status, response) => {
            let url = response.get_url().to_owned();
            let body = response.into_string().unwrap_or_default();
            InstallationError::HttpStatus { url, status, body }
        }
        ureq::Error::Transport(transport) => InstallationError::Transport {
            url: "<transport>".to_owned(),
            message: transport.to_string(),
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
                "loader": { "version": "0.16.10" }
            },
            {
                "loader": { "version": "0.16.9" }
            }
        ]);

        let versions = parse_global_loader_versions(&payload);
        assert!(versions.iter().any(|entry| entry == "0.16.10"));
        assert!(versions.iter().any(|entry| entry == "0.16.9"));
    }

    #[test]
    fn url_encoding_covers_spaces_and_symbols() {
        assert_eq!(
            url_encode_component("1.14 Pre-Release 5"),
            "1.14%20Pre-Release%205"
        );
        assert_eq!(url_encode_component("a/b"), "a%2Fb");
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
