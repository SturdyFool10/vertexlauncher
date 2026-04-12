use super::*;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionCatalogFilter {
    pub include_snapshots_and_betas: bool,
    pub include_alpha: bool,
    pub include_experimental: bool,
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

    pub(crate) fn sort_desc(&mut self) {
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
pub(crate) type InstallProgressSink = dyn Fn(InstallProgress) + Send + Sync + 'static;

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
pub(crate) struct CachedVersionCatalog {
    pub(crate) fetched_at_unix_secs: u64,
    #[serde(default)]
    pub(crate) filter: VersionCatalogFilter,
    pub(crate) catalog: VersionCatalog,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct CachedLoaderVersions {
    pub(crate) fetched_at_unix_secs: u64,
    pub(crate) loader_label: String,
    pub(crate) versions_by_game_version: BTreeMap<String, Vec<String>>,
}

pub fn fetch_version_catalog(filter: VersionCatalogFilter) -> Result<VersionCatalog, InstallationError> {
    fetch_version_catalog_with_refresh(filter, false)
}

pub fn fetch_version_catalog_with_refresh(
    filter: VersionCatalogFilter,
    force_refresh: bool,
) -> Result<VersionCatalog, InstallationError> {
    let cached = read_cached_version_catalog(filter).ok();
    if !force_refresh
        && let Some(cached) = cached.as_ref()
        && cached.filter == filter
        && !is_cache_expired(cached.fetched_at_unix_secs)
        && catalog_has_loader_version_data(&cached.catalog)
    {
        let mut catalog = cached.catalog.clone();
        normalize_version_catalog_ordering(&mut catalog);
        return Ok(catalog);
    }

    match fetch_version_catalog_uncached(filter) {
        Ok(catalog) => {
            let _ = write_cached_version_catalog(filter, &catalog);
            Ok(catalog)
        }
        Err(err) => {
            if let Some(cached) = cached
                && cached.filter == filter
            {
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
    filter: VersionCatalogFilter,
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
                | MinecraftVersionType::OldBeta => filter.include_snapshots_and_betas,
                MinecraftVersionType::OldAlpha => filter.include_alpha,
                MinecraftVersionType::Unknown => filter.include_experimental,
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
