use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_USER_AGENT: &str =
    "VertexLauncher/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";
const MOJANG_VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const FABRIC_GAME_VERSIONS_URL: &str = "https://meta.fabricmc.net/v2/versions/game";
const QUILT_GAME_VERSIONS_URL: &str = "https://meta.quiltmc.org/v3/versions/game";
const FORGE_MAVEN_METADATA_URL: &str =
    "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml";
const NEOFORGE_MAVEN_METADATA_URL: &str =
    "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml";
const NEOFORGE_LEGACY_FORGE_METADATA_URL: &str =
    "https://maven.neoforged.net/releases/net/neoforged/forge/maven-metadata.xml";
const CACHE_VERSION_CATALOG_RELEASES_FILE: &str = "version_catalog_release_only.json";
const CACHE_VERSION_CATALOG_ALL_FILE: &str = "version_catalog_with_snapshots.json";
const CACHE_DIR_NAME: &str = "cache";
const VERSION_CATALOG_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

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
pub struct VersionCatalog {
    pub game_versions: Vec<MinecraftVersionEntry>,
    pub loader_support: LoaderSupportIndex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameSetupResult {
    pub version_json_path: PathBuf,
    pub client_jar_path: PathBuf,
    pub downloaded_files: u32,
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
    #[error("Minecraft version '{0}' was not found in Mojang manifest")]
    UnknownMinecraftVersion(String),
    #[error("Version metadata for '{0}' is missing client download information")]
    MissingClientDownload(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedVersionCatalog {
    fetched_at_unix_secs: u64,
    include_snapshots_and_betas: bool,
    catalog: VersionCatalog,
}

pub fn fetch_version_catalog(
    include_snapshots_and_betas: bool,
) -> Result<VersionCatalog, InstallationError> {
    let cached = read_cached_version_catalog(include_snapshots_and_betas).ok();
    if let Some(cached) = cached.as_ref()
        && !is_cache_expired(cached.fetched_at_unix_secs)
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

pub fn purge_cache() -> Result<(), InstallationError> {
    let cache_root = cache_root_dir();
    match fs::remove_dir_all(&cache_root) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(InstallationError::Io(err)),
    }
}

fn fetch_version_catalog_uncached(
    include_snapshots_and_betas: bool,
) -> Result<VersionCatalog, InstallationError> {
    let manifest: MojangVersionManifest = get_json(MOJANG_VERSION_MANIFEST_URL)?;
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
        fabric: fetch_fabric_versions().unwrap_or_default(),
        forge: fetch_forge_versions().unwrap_or_default(),
        neoforge: fetch_neoforge_versions().unwrap_or_default(),
        quilt: fetch_quilt_versions().unwrap_or_default(),
    };

    Ok(VersionCatalog {
        game_versions,
        loader_support,
    })
}

pub fn ensure_game_files(
    instance_root: &Path,
    game_version: &str,
) -> Result<GameSetupResult, InstallationError> {
    let game_version = game_version.trim();
    if game_version.is_empty() {
        return Err(InstallationError::UnknownMinecraftVersion(String::new()));
    }

    let versions_dir = instance_root.join("versions").join(game_version);
    fs::create_dir_all(&versions_dir)?;
    let version_json_path = versions_dir.join(format!("{game_version}.json"));
    let client_jar_path = versions_dir.join(format!("{game_version}.jar"));

    if version_json_path.exists() && client_jar_path.exists() {
        fs::create_dir_all(instance_root.join("mods"))?;
        fs::create_dir_all(instance_root.join("assets"))?;
        fs::create_dir_all(instance_root.join("libraries"))?;

        return Ok(GameSetupResult {
            version_json_path,
            client_jar_path,
            downloaded_files: 0,
        });
    }

    let manifest: MojangVersionManifest = get_json(MOJANG_VERSION_MANIFEST_URL)?;
    let version_entry = manifest
        .versions
        .into_iter()
        .find(|entry| entry.id == game_version)
        .ok_or_else(|| InstallationError::UnknownMinecraftVersion(game_version.to_owned()))?;

    let version_json_raw = get_text(&version_entry.url)?;
    let version_meta: MojangVersionMeta = serde_json::from_str(&version_json_raw)?;
    let client_download = version_meta
        .downloads
        .and_then(|downloads| downloads.client)
        .ok_or_else(|| InstallationError::MissingClientDownload(game_version.to_owned()))?;

    let mut downloaded_files = 0;

    if !version_json_path.exists() {
        fs::write(&version_json_path, version_json_raw.as_bytes())?;
        downloaded_files += 1;
    }

    if !client_jar_path.exists() {
        let jar_bytes = get_bytes(&client_download.url)?;
        fs::write(&client_jar_path, jar_bytes)?;
        downloaded_files += 1;
    }

    fs::create_dir_all(instance_root.join("mods"))?;
    fs::create_dir_all(instance_root.join("assets"))?;
    fs::create_dir_all(instance_root.join("libraries"))?;

    Ok(GameSetupResult {
        version_json_path,
        client_jar_path,
        downloaded_files,
    })
}

fn cache_root_dir() -> PathBuf {
    match std::env::var("VERTEX_CONFIG_LOCATION") {
        Ok(dir) => PathBuf::from(dir).join(CACHE_DIR_NAME),
        Err(_) => PathBuf::from(CACHE_DIR_NAME),
    }
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
    let raw = fs::read_to_string(cache_file_path(include_snapshots_and_betas))?;
    Ok(serde_json::from_str(&raw)?)
}

fn write_cached_version_catalog(
    include_snapshots_and_betas: bool,
    catalog: &VersionCatalog,
) -> Result<(), InstallationError> {
    let path = cache_file_path(include_snapshots_and_betas);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let payload = CachedVersionCatalog {
        fetched_at_unix_secs: now_unix_secs(),
        include_snapshots_and_betas,
        catalog: catalog.clone(),
    };
    let file = fs::File::create(path)?;
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

fn fetch_fabric_versions() -> Result<HashSet<String>, InstallationError> {
    let versions: Vec<FabricGameVersion> = get_json(FABRIC_GAME_VERSIONS_URL)?;
    Ok(versions
        .into_iter()
        .map(|version| version.version.trim().to_owned())
        .filter(|version| !version.is_empty())
        .collect())
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

fn fetch_forge_versions() -> Result<HashSet<String>, InstallationError> {
    let metadata = get_text(FORGE_MAVEN_METADATA_URL)?;
    Ok(parse_minecraft_versions_from_maven_metadata(
        &metadata, true,
    ))
}

fn fetch_neoforge_versions() -> Result<HashSet<String>, InstallationError> {
    let primary = get_text(NEOFORGE_MAVEN_METADATA_URL)?;
    let mut versions = parse_neoforge_versions_from_metadata(&primary);

    if let Ok(legacy) = get_text(NEOFORGE_LEGACY_FORGE_METADATA_URL) {
        versions.extend(parse_minecraft_versions_from_maven_metadata(&legacy, true));
    }

    Ok(versions)
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

fn get_text(url: &str) -> Result<String, InstallationError> {
    let response = match ureq::get(url).set("User-Agent", DEFAULT_USER_AGENT).call() {
        Ok(ok) => ok,
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(InstallationError::HttpStatus {
                url: url.to_owned(),
                status,
                body,
            });
        }
        Err(ureq::Error::Transport(transport)) => {
            return Err(InstallationError::Transport {
                url: url.to_owned(),
                message: transport.to_string(),
            });
        }
    };

    response.into_string().map_err(InstallationError::Io)
}

fn get_bytes(url: &str) -> Result<Vec<u8>, InstallationError> {
    let response = match ureq::get(url).set("User-Agent", DEFAULT_USER_AGENT).call() {
        Ok(ok) => ok,
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(InstallationError::HttpStatus {
                url: url.to_owned(),
                status,
                body,
            });
        }
        Err(ureq::Error::Transport(transport)) => {
            return Err(InstallationError::Transport {
                url: url.to_owned(),
                message: transport.to_string(),
            });
        }
    };

    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    std::io::copy(&mut reader, &mut bytes)?;
    Ok(bytes)
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
}
