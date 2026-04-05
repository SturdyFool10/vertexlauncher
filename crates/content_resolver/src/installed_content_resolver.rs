use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use curseforge::Client as CurseForgeClient;
use managed_content::InstalledContentIdentity;
use modprovider::{ContentSource, UnifiedContentEntry, search_minecraft_content};
use modrinth::{Client as ModrinthClient, ModrinthError};
use vertex_constants::content_resolver::{
    HASH_CACHE_DIR_NAME, HASH_CACHE_FILE_NAME, HEURISTIC_WARNING_MESSAGE, LOOKUP_CACHE_KEY_PREFIX,
};

use crate::{
    InstalledContentFile, InstalledContentHashCache, InstalledContentHashCacheUpdate,
    InstalledContentKind, InstalledContentResolutionKind, InstalledContentUpdate,
    ResolveInstalledContentRequest, ResolveInstalledContentResult, ResolvedInstalledContent,
};

#[track_caller]
fn fs_read_dir(path: &Path) -> std::io::Result<std::fs::ReadDir> {
    tracing::debug!(target: "vertexlauncher/io", op = "read_dir", path = %path.display());
    let result = std::fs::read_dir(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_dir", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_read_to_string(path: &Path) -> std::io::Result<String> {
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    let result = std::fs::read_to_string(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_create_dir_all(path: &Path) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display());
    let result = std::fs::create_dir_all(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_write(path: &Path, raw: String) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "write", path = %path.display());
    let result = std::fs::write(path, raw);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "write", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_remove_file(path: &Path) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "remove_file", path = %path.display());
    let result = std::fs::remove_file(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "remove_file", path = %path.display(), error = %err);
    }
    result
}

/// Maximum number of entries kept in each session-scoped lookup cache.
/// Prevents unbounded growth when the user browses a very large mod library.
const LOOKUP_CACHE_MAX_ENTRIES: usize = 4096;

/// Insert `value` under `key` only when the map is below the entry cap.
/// Existing entries are always overwritten regardless of size.
fn bounded_cache_insert<K, V>(map: &mut HashMap<K, V>, key: K, value: V)
where
    K: Eq + std::hash::Hash,
{
    if map.len() < LOOKUP_CACHE_MAX_ENTRIES || map.contains_key(&key) {
        map.insert(key, value);
    }
}

macro_rules! static_cache {
    ($name:ident, $k:ty, $v:ty) => {
        fn $name() -> &'static Mutex<HashMap<$k, Option<Arc<$v>>>> {
            static CACHE: OnceLock<Mutex<HashMap<$k, Option<Arc<$v>>>>> = OnceLock::new();
            CACHE.get_or_init(|| Mutex::new(HashMap::new()))
        }
    };
}

static_cache!(modrinth_entry_cache, String, UnifiedContentEntry);
static_cache!(modrinth_version_cache, String, modrinth::ProjectVersion);
static_cache!(
    modrinth_latest_version_cache,
    String,
    modrinth::ProjectVersion
);
static_cache!(curseforge_project_cache, u64, curseforge::Project);
static_cache!(curseforge_file_cache, u64, curseforge::File);

fn should_cache_modrinth_absence(err: &ModrinthError) -> bool {
    matches!(err, ModrinthError::HttpStatus { status: 404, .. })
}

fn should_cache_curseforge_absence(err: &curseforge::CurseForgeError) -> bool {
    matches!(
        err,
        curseforge::CurseForgeError::HttpStatus { status: 404, .. }
    )
}

pub struct InstalledContentResolver;

impl InstalledContentResolver {
    pub fn scan_installed_content_files(
        instance_root: &Path,
        kind: InstalledContentKind,
        managed_identities: &HashMap<String, InstalledContentIdentity>,
    ) -> Vec<InstalledContentFile> {
        let dir = instance_root.join(kind.folder_name());
        let mut files = Vec::new();
        let Ok(read_dir) = fs_read_dir(dir.as_path()) else {
            return files;
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with('.') {
                continue;
            }

            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            let allowed = match kind {
                InstalledContentKind::Mods => file_type.is_file() && extension == "jar",
                InstalledContentKind::ResourcePacks
                | InstalledContentKind::ShaderPacks
                | InstalledContentKind::DataPacks => file_type.is_dir() || extension == "zip",
            };
            if !allowed {
                continue;
            }

            let relative_path_key = normalize_installed_content_path_key(
                path.strip_prefix(instance_root)
                    .unwrap_or(path.as_path())
                    .to_string_lossy()
                    .as_ref(),
            );
            let managed_identity = managed_identities.get(relative_path_key.as_str()).cloned();
            let lookup_query = managed_identity
                .as_ref()
                .map(|identity| identity.name.clone())
                .unwrap_or_else(|| {
                    derive_installed_lookup_query(path.as_path(), file_name.as_str())
                });
            let (fallback_lookup_query, fallback_lookup_key) = if managed_identity.is_some() {
                (None, None)
            } else {
                let fallback_query = derive_raw_lookup_query(path.as_path(), file_name.as_str());
                let fallback_key_suffix = normalize_lookup_key(fallback_query.as_str());
                if fallback_key_suffix.is_empty()
                    || fallback_key_suffix == normalize_lookup_key(lookup_query.as_str())
                {
                    (None, None)
                } else {
                    (
                        Some(fallback_query),
                        Some(format!("{}::{fallback_key_suffix}", kind.folder_name())),
                    )
                }
            };
            let lookup_key = format!(
                "{}::{}",
                kind.folder_name(),
                managed_lookup_key_suffix(managed_identity.as_ref(), lookup_query.as_str())
            );
            files.push(InstalledContentFile {
                file_name,
                file_path: path,
                lookup_query,
                lookup_key,
                fallback_lookup_query,
                fallback_lookup_key,
                managed_identity,
            });
        }

        files.sort_by(|left, right| {
            left.file_name
                .to_ascii_lowercase()
                .cmp(&right.file_name.to_ascii_lowercase())
        });
        files
    }

    pub fn load_hash_cache(instance_root: &Path) -> InstalledContentHashCache {
        let cache_path = content_hash_cache_path(instance_root);
        let Ok(raw) = fs_read_to_string(cache_path.as_path()) else {
            return InstalledContentHashCache::default();
        };
        let Ok(cache) = serde_json::from_str::<InstalledContentHashCache>(raw.as_str()) else {
            return InstalledContentHashCache::default();
        };
        cache.into_current().unwrap_or_default()
    }

    pub fn save_hash_cache(
        instance_root: &Path,
        cache: &InstalledContentHashCache,
    ) -> Result<(), std::io::Error> {
        let cache_path = content_hash_cache_path(instance_root);
        if let Some(parent) = cache_path.parent() {
            fs_create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(cache)
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        fs_write(cache_path.as_path(), raw)
    }

    pub fn clear_hash_cache(instance_root: &Path) -> Result<(), std::io::Error> {
        let cache_path = content_hash_cache_path(instance_root);
        if cache_path.exists() {
            fs_remove_file(cache_path.as_path())?;
        }
        Ok(())
    }

    pub fn resolve(
        request: &ResolveInstalledContentRequest,
        hash_cache: &InstalledContentHashCache,
    ) -> ResolveInstalledContentResult {
        let mut hash_cache_updates = Vec::new();

        let exact_hash_resolution =
            if supports_modrinth_hash_resolution(request.kind, request.file_path.as_path()) {
                let (resolution, updates) = resolve_modrinth_hash_metadata(
                    request.file_path.as_path(),
                    request.kind,
                    request.game_version.as_str(),
                    request.loader.as_str(),
                    hash_cache,
                );
                hash_cache_updates = updates;
                resolution
            } else {
                None
            };

        if let Some(resolution) = exact_hash_resolution.as_ref() {
            hash_cache_updates.extend(lookup_cache_updates_for_request(request, resolution));
        }

        let managed_resolution = if exact_hash_resolution.is_none() {
            managed_content_metadata(
                request.file_path.as_path(),
                request.disk_file_name.as_str(),
                request.managed_identity.as_ref(),
                request.kind,
                request.game_version.as_str(),
                request.loader.as_str(),
            )
        } else {
            None
        };
        if let Some(resolution) = managed_resolution.as_ref() {
            hash_cache_updates.extend(lookup_cache_updates_for_request(request, resolution));
        }

        let lookup_cached_resolution =
            if exact_hash_resolution.is_none() && managed_resolution.is_none() {
                resolve_lookup_cache_metadata(request, hash_cache)
            } else {
                None
            };

        let heuristic_resolution = if exact_hash_resolution.is_none()
            && managed_resolution.is_none()
            && lookup_cached_resolution.is_none()
        {
            heuristic_content_metadata(request)
        } else {
            None
        };

        let resolution = exact_hash_resolution
            .or(managed_resolution)
            .or(lookup_cached_resolution)
            .or(heuristic_resolution);

        ResolveInstalledContentResult {
            resolution,
            hash_cache_updates,
        }
    }
}

fn resolve_modrinth_hash_metadata(
    file_path: &Path,
    kind: InstalledContentKind,
    game_version: &str,
    loader: &str,
    hash_cache: &InstalledContentHashCache,
) -> (
    Option<ResolvedInstalledContent>,
    Vec<InstalledContentHashCacheUpdate>,
) {
    let Ok((sha1, sha512)) = modrinth::hash_file_sha1_and_sha512_hex(file_path) else {
        return (None, Vec::new());
    };

    let modrinth = ModrinthClient::default();
    let loaders = if kind == InstalledContentKind::Mods {
        modrinth_loader_slugs(loader)
    } else {
        Vec::new()
    };
    let game_versions = normalized_game_versions(game_version);
    let mut saw_transient_error = false;

    for (algorithm, hash) in [("sha512", sha512.as_str()), ("sha1", sha1.as_str())] {
        let hash_key = format!("{algorithm}:{hash}");
        if let Some(cached) = hash_cache.entries.get(hash_key.as_str()) {
            if let Some(mut cached_resolution) = cached.clone() {
                cached_resolution.update = resolve_modrinth_hash_update(
                    &modrinth,
                    hash,
                    algorithm,
                    loaders.as_slice(),
                    game_versions.as_slice(),
                    cached_resolution.installed_version_id.as_deref(),
                );
                return (Some(cached_resolution), Vec::new());
            }
            continue;
        }

        let version = match modrinth.get_version_from_hash(hash, algorithm) {
            Ok(version) => version,
            Err(err) => {
                if should_skip_hash_cache_update(&err) {
                    saw_transient_error = true;
                }
                continue;
            }
        };
        let Some(version) = version else {
            continue;
        };
        let Some(entry) = modrinth_entry_from_project_id(&modrinth, version.project_id.as_str())
        else {
            continue;
        };

        let resolution = ResolvedInstalledContent {
            entry: Arc::unwrap_or_clone(entry),
            installed_version_id: non_empty_owned(version.id.as_str()),
            installed_version_label: non_empty_owned(version.version_number.as_str()),
            resolution_kind: InstalledContentResolutionKind::ExactHash,
            warning_message: None,
            update: resolve_modrinth_hash_update(
                &modrinth,
                hash,
                algorithm,
                loaders.as_slice(),
                game_versions.as_slice(),
                Some(version.id.as_str()),
            ),
        };
        let mut cached_resolution = resolution.clone();
        cached_resolution.update = None;
        let updates = vec![
            InstalledContentHashCacheUpdate {
                hash_key: format!("sha512:{sha512}"),
                resolution: Some(cached_resolution.clone()),
            },
            InstalledContentHashCacheUpdate {
                hash_key: format!("sha1:{sha1}"),
                resolution: Some(cached_resolution),
            },
        ];
        return (Some(resolution), updates);
    }

    if saw_transient_error {
        return (None, Vec::new());
    }

    (
        None,
        vec![
            InstalledContentHashCacheUpdate {
                hash_key: format!("sha512:{sha512}"),
                resolution: None,
            },
            InstalledContentHashCacheUpdate {
                hash_key: format!("sha1:{sha1}"),
                resolution: None,
            },
        ],
    )
}

fn resolve_modrinth_hash_update(
    modrinth: &ModrinthClient,
    hash: &str,
    algorithm: &str,
    loaders: &[String],
    game_versions: &[String],
    installed_version_id: Option<&str>,
) -> Option<InstalledContentUpdate> {
    let latest = modrinth
        .get_latest_version_from_hash(hash, algorithm, loaders, game_versions)
        .ok()
        .flatten()?;
    if installed_version_id.is_some_and(|value| value == latest.id) {
        return None;
    }

    Some(InstalledContentUpdate {
        latest_version_id: latest.id,
        latest_version_label: non_empty_owned(latest.version_number.as_str())
            .unwrap_or_else(|| "Unknown update".to_owned()),
    })
}

fn modrinth_entry_from_project_id(
    modrinth: &ModrinthClient,
    project_id: &str,
) -> Option<Arc<UnifiedContentEntry>> {
    if let Ok(cache) = modrinth_entry_cache().lock()
        && let Some(cached) = cache.get(project_id)
    {
        return cached.clone();
    }

    match modrinth.get_project(project_id) {
        Ok(project) => {
            let entry = Arc::new(UnifiedContentEntry {
                id: format!("modrinth:{}", project.project_id),
                name: project.title,
                summary: project.description.trim().to_owned(),
                content_type: project.project_type,
                source: ContentSource::Modrinth,
                project_url: Some(project.project_url),
                icon_url: project.icon_url,
            });
            if let Ok(mut cache) = modrinth_entry_cache().lock() {
                bounded_cache_insert(&mut cache, project_id.to_owned(), Some(Arc::clone(&entry)));
            }
            Some(entry)
        }
        Err(err) if should_cache_modrinth_absence(&err) => {
            if let Ok(mut cache) = modrinth_entry_cache().lock() {
                bounded_cache_insert(&mut cache, project_id.to_owned(), None);
            }
            None
        }
        Err(_) => None,
    }
}

fn managed_content_metadata(
    file_path: &Path,
    disk_file_name: &str,
    managed_identity: Option<&InstalledContentIdentity>,
    kind: InstalledContentKind,
    game_version: &str,
    loader: &str,
) -> Option<ResolvedInstalledContent> {
    let identity = managed_identity?;
    let suppress_updates = identity.pack_managed;

    match identity.source {
        ContentSource::Modrinth => {
            let project_id = identity.modrinth_project_id.as_deref()?;
            let version_id = identity.selected_version_id.trim();
            if version_id.is_empty()
                || !managed_identity_matches_file_name(identity, disk_file_name)
            {
                return None;
            }

            let modrinth = ModrinthClient::default();
            let version = cached_modrinth_version(&modrinth, version_id)?;
            if version.project_id != project_id
                || !version_contains_file_name(version.files.as_slice(), disk_file_name)
            {
                return None;
            }
            let entry = modrinth_entry_from_project_id(&modrinth, project_id)?;
            Some(ResolvedInstalledContent {
                entry: Arc::unwrap_or_clone(entry),
                installed_version_id: Some(version.id.clone()),
                installed_version_label: non_empty_owned(version.version_number.as_str()),
                resolution_kind: InstalledContentResolutionKind::Managed,
                warning_message: None,
                update: if suppress_updates {
                    None
                } else {
                    resolve_managed_modrinth_update(
                        &modrinth,
                        project_id,
                        kind,
                        game_version,
                        loader,
                        Some(version_id),
                    )
                },
            })
        }
        ContentSource::CurseForge => {
            let project_id = identity.curseforge_project_id?;
            let version_id = identity.selected_version_id.trim().parse::<u64>().ok()?;
            let curseforge = CurseForgeClient::from_env()?;
            let file = cached_curseforge_file(&curseforge, version_id)?;
            if !file_name_matches(file.file_name.as_str(), disk_file_name)
                || !file_name_matches(file_path.file_name()?.to_str()?, disk_file_name)
            {
                return None;
            }

            let project = cached_curseforge_project(&curseforge, project_id)?;
            Some(ResolvedInstalledContent {
                entry: UnifiedContentEntry {
                    id: format!("curseforge:{}", project.id),
                    name: project.name.clone(),
                    summary: project.summary.trim().to_owned(),
                    content_type: kind.content_type_key().to_owned(),
                    source: ContentSource::CurseForge,
                    project_url: project.website_url.clone(),
                    icon_url: project.icon_url.clone(),
                },
                installed_version_id: Some(file.id.to_string()),
                installed_version_label: non_empty_owned(file.display_name.as_str()),
                resolution_kind: InstalledContentResolutionKind::Managed,
                warning_message: None,
                update: if suppress_updates {
                    None
                } else {
                    resolve_managed_curseforge_update(
                        &curseforge,
                        &project,
                        version_id,
                        kind,
                        game_version,
                        loader,
                    )
                },
            })
        }
    }
}

fn heuristic_content_metadata(
    request: &ResolveInstalledContentRequest,
) -> Option<ResolvedInstalledContent> {
    let mut candidates = Vec::new();
    if !request.lookup_query.trim().is_empty() {
        candidates.push((request.lookup_query.as_str(), None::<&str>));
    }
    if let (Some(fallback_key), Some(fallback_query)) = (
        request.fallback_lookup_key.as_deref(),
        request.fallback_lookup_query.as_deref(),
    ) && !fallback_key.trim().is_empty()
        && !fallback_query.trim().is_empty()
    {
        candidates.push((fallback_query, Some(fallback_key)));
    }

    for (query, override_key) in candidates {
        let lookup_key = override_key.unwrap_or(request.lookup_query.as_str());
        let mut entries = search_modrinth_heuristic_content(
            query,
            request.kind,
            request.game_version.as_str(),
            request.loader.as_str(),
        );
        if entries.is_empty() {
            entries = search_minecraft_content(query, 10).ok()?.entries;
        }
        if let Some(entry) = choose_preferred_content_entry(entries, lookup_key, request.kind) {
            return Some(ResolvedInstalledContent {
                entry,
                installed_version_id: None,
                installed_version_label: None,
                resolution_kind: InstalledContentResolutionKind::HeuristicSearch,
                warning_message: Some(HEURISTIC_WARNING_MESSAGE.to_owned()),
                update: None,
            });
        }
    }

    None
}

fn resolve_lookup_cache_metadata(
    request: &ResolveInstalledContentRequest,
    hash_cache: &InstalledContentHashCache,
) -> Option<ResolvedInstalledContent> {
    for cache_key in lookup_cache_keys_for_request(request) {
        if let Some(Some(resolution)) = hash_cache.entries.get(cache_key.as_str()) {
            return Some(resolution.clone());
        }
    }

    None
}

fn lookup_cache_updates_for_request(
    request: &ResolveInstalledContentRequest,
    resolution: &ResolvedInstalledContent,
) -> Vec<InstalledContentHashCacheUpdate> {
    let cached_resolution = lookup_cache_resolution(resolution);
    lookup_cache_keys_for_request_and_resolution(request, resolution)
        .into_iter()
        .map(|cache_key| InstalledContentHashCacheUpdate {
            hash_key: cache_key,
            resolution: Some(cached_resolution.clone()),
        })
        .collect()
}

fn lookup_cache_resolution(resolution: &ResolvedInstalledContent) -> ResolvedInstalledContent {
    ResolvedInstalledContent {
        entry: resolution.entry.clone(),
        installed_version_id: None,
        installed_version_label: None,
        resolution_kind: InstalledContentResolutionKind::HeuristicSearch,
        warning_message: Some(HEURISTIC_WARNING_MESSAGE.to_owned()),
        update: None,
    }
}

fn lookup_cache_keys_for_request(request: &ResolveInstalledContentRequest) -> Vec<String> {
    let mut cache_keys = Vec::new();
    push_lookup_cache_key(&mut cache_keys, request.kind, request.lookup_query.as_str());
    if let Some(fallback_lookup_query) = request.fallback_lookup_query.as_deref() {
        push_lookup_cache_key(&mut cache_keys, request.kind, fallback_lookup_query);
    }
    cache_keys
}

fn lookup_cache_keys_for_request_and_resolution(
    request: &ResolveInstalledContentRequest,
    resolution: &ResolvedInstalledContent,
) -> Vec<String> {
    let mut cache_keys = lookup_cache_keys_for_request(request);
    push_lookup_cache_key(
        &mut cache_keys,
        request.kind,
        resolution.entry.name.as_str(),
    );
    cache_keys
}

fn push_lookup_cache_key(cache_keys: &mut Vec<String>, kind: InstalledContentKind, query: &str) {
    let Some(cache_key) = lookup_cache_key(kind, query) else {
        return;
    };
    if !cache_keys.iter().any(|existing| existing == &cache_key) {
        cache_keys.push(cache_key);
    }
}

fn lookup_cache_key(kind: InstalledContentKind, query: &str) -> Option<String> {
    let normalized_query = normalize_lookup_cache_query(query)?;
    Some(format!(
        "{LOOKUP_CACHE_KEY_PREFIX}{}::{normalized_query}",
        kind.folder_name()
    ))
}

fn normalize_lookup_cache_query(query: &str) -> Option<String> {
    let normalized_query = normalize_lookup_key(query);
    if normalized_query.is_empty() {
        return None;
    }

    let tokens = split_lookup_tokens(normalized_query.as_str());
    let canonical_tokens = trim_ignorable_lookup_suffix(tokens.as_slice());
    if canonical_tokens.is_empty() {
        Some(normalized_query)
    } else {
        Some(canonical_tokens.join(" "))
    }
}

fn search_modrinth_heuristic_content(
    query: &str,
    kind: InstalledContentKind,
    game_version: &str,
    loader: &str,
) -> Vec<UnifiedContentEntry> {
    let modrinth = ModrinthClient::default();
    let loader_filter = if kind == InstalledContentKind::Mods {
        modrinth_loader_slug(loader)
    } else {
        None
    };
    let game_version = normalize_optional(game_version);
    let Ok(entries) = modrinth.search_projects_with_filters(
        query,
        10,
        0,
        Some(kind.modrinth_project_type()),
        game_version.as_deref(),
        loader_filter,
        None,
    ) else {
        return Vec::new();
    };

    entries
        .into_iter()
        .map(|entry| UnifiedContentEntry {
            id: format!("modrinth:{}", entry.project_id),
            name: entry.title,
            summary: entry.description.trim().to_owned(),
            content_type: entry.project_type,
            source: ContentSource::Modrinth,
            project_url: Some(entry.project_url),
            icon_url: entry.icon_url,
        })
        .collect()
}

fn managed_identity_matches_file_name(
    identity: &InstalledContentIdentity,
    disk_file_name: &str,
) -> bool {
    let expected = identity
        .file_path
        .as_path()
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    file_name_matches(expected, disk_file_name)
}

fn version_contains_file_name(
    files: &[modrinth::ProjectVersionFile],
    disk_file_name: &str,
) -> bool {
    files
        .iter()
        .any(|file| file_name_matches(file.filename.as_str(), disk_file_name))
}

fn cached_modrinth_version(
    modrinth: &ModrinthClient,
    version_id: &str,
) -> Option<Arc<modrinth::ProjectVersion>> {
    if let Ok(cache) = modrinth_version_cache().lock()
        && let Some(cached) = cache.get(version_id)
    {
        return cached.clone();
    }

    match modrinth.get_version(version_id) {
        Ok(version) => {
            let arc = Arc::new(version);
            if let Ok(mut cache) = modrinth_version_cache().lock() {
                bounded_cache_insert(&mut cache, version_id.to_owned(), Some(Arc::clone(&arc)));
            }
            Some(arc)
        }
        Err(err) if should_cache_modrinth_absence(&err) => {
            if let Ok(mut cache) = modrinth_version_cache().lock() {
                bounded_cache_insert(&mut cache, version_id.to_owned(), None);
            }
            None
        }
        Err(_) => None,
    }
}

fn cached_latest_modrinth_project_version(
    modrinth: &ModrinthClient,
    project_id: &str,
    loaders: &[String],
    game_versions: &[String],
) -> Option<Arc<modrinth::ProjectVersion>> {
    let cache_key = format!(
        "{project_id}|{}|{}",
        loaders.join(","),
        game_versions.join(",")
    );
    if let Ok(cache) = modrinth_latest_version_cache().lock()
        && let Some(cached) = cache.get(cache_key.as_str())
    {
        return cached.clone();
    }

    match modrinth.list_project_versions(project_id, loaders, game_versions) {
        Ok(versions) => {
            let latest = versions
                .into_iter()
                .filter(|version| !version.files.is_empty())
                .max_by(|left, right| left.date_published.cmp(&right.date_published))
                .map(Arc::new);
            if let Ok(mut cache) = modrinth_latest_version_cache().lock() {
                bounded_cache_insert(&mut cache, cache_key, latest.clone());
            }
            latest
        }
        Err(err) if should_cache_modrinth_absence(&err) => {
            if let Ok(mut cache) = modrinth_latest_version_cache().lock() {
                bounded_cache_insert(&mut cache, cache_key, None);
            }
            None
        }
        Err(_) => None,
    }
}

fn cached_curseforge_project(
    client: &CurseForgeClient,
    project_id: u64,
) -> Option<Arc<curseforge::Project>> {
    if let Ok(cache) = curseforge_project_cache().lock()
        && let Some(cached) = cache.get(&project_id)
    {
        return cached.clone();
    }

    match client.get_mod(project_id) {
        Ok(project) => {
            let arc = Arc::new(project);
            if let Ok(mut cache) = curseforge_project_cache().lock() {
                bounded_cache_insert(&mut cache, project_id, Some(Arc::clone(&arc)));
            }
            Some(arc)
        }
        Err(err) if should_cache_curseforge_absence(&err) => {
            if let Ok(mut cache) = curseforge_project_cache().lock() {
                bounded_cache_insert(&mut cache, project_id, None);
            }
            None
        }
        Err(_) => None,
    }
}

fn cached_curseforge_file(
    client: &CurseForgeClient,
    file_id: u64,
) -> Option<Arc<curseforge::File>> {
    if let Ok(cache) = curseforge_file_cache().lock()
        && let Some(cached) = cache.get(&file_id)
    {
        return cached.clone();
    }

    match client.get_files(&[file_id]) {
        Ok(files) => {
            let file = files.into_iter().next().map(Arc::new);
            if let Ok(mut cache) = curseforge_file_cache().lock() {
                bounded_cache_insert(&mut cache, file_id, file.clone());
            }
            file
        }
        Err(err) if should_cache_curseforge_absence(&err) => {
            if let Ok(mut cache) = curseforge_file_cache().lock() {
                bounded_cache_insert(&mut cache, file_id, None);
            }
            None
        }
        Err(_) => None,
    }
}

fn resolve_managed_modrinth_update(
    modrinth: &ModrinthClient,
    project_id: &str,
    kind: InstalledContentKind,
    game_version: &str,
    loader: &str,
    installed_version_id: Option<&str>,
) -> Option<InstalledContentUpdate> {
    let loaders = if kind == InstalledContentKind::Mods {
        modrinth_loader_slugs(loader)
    } else {
        Vec::new()
    };
    let game_versions = normalized_game_versions(game_version);
    let latest = cached_latest_modrinth_project_version(
        modrinth,
        project_id,
        loaders.as_slice(),
        game_versions.as_slice(),
    )?;
    if installed_version_id.is_some_and(|value| value == latest.id) {
        return None;
    }

    Some(InstalledContentUpdate {
        latest_version_id: latest.id.clone(),
        latest_version_label: non_empty_owned(latest.version_number.as_str())
            .unwrap_or_else(|| "Unknown update".to_owned()),
    })
}

fn resolve_managed_curseforge_update(
    curseforge: &CurseForgeClient,
    project: &curseforge::Project,
    installed_version_id: u64,
    kind: InstalledContentKind,
    game_version: &str,
    loader: &str,
) -> Option<InstalledContentUpdate> {
    let latest_file_id = project
        .latest_files_indexes
        .iter()
        .filter(|index| {
            normalize_optional(game_version)
                .as_deref()
                .is_none_or(|value| index.game_version.trim() == value)
        })
        .filter(|index| {
            if kind == InstalledContentKind::Mods {
                curseforge_mod_loader_type(loader)
                    .is_none_or(|value| index.mod_loader == Some(value))
            } else {
                true
            }
        })
        .map(|index| index.file_id)
        .max()?;
    let latest = cached_curseforge_file(curseforge, latest_file_id)?;
    if latest.download_url.is_none() {
        return None;
    }
    if latest.id == installed_version_id {
        return None;
    }

    Some(InstalledContentUpdate {
        latest_version_id: latest.id.to_string(),
        latest_version_label: non_empty_owned(latest.display_name.as_str())
            .unwrap_or_else(|| "Unknown update".to_owned()),
    })
}

fn supports_modrinth_hash_resolution(kind: InstalledContentKind, path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };

    match kind {
        InstalledContentKind::Mods => extension.eq_ignore_ascii_case("jar"),
        InstalledContentKind::ResourcePacks
        | InstalledContentKind::ShaderPacks
        | InstalledContentKind::DataPacks => extension.eq_ignore_ascii_case("zip"),
    }
}

fn modrinth_loader_slugs(loader: &str) -> Vec<String> {
    modrinth_loader_slug(loader)
        .map(|value| vec![value.to_owned()])
        .unwrap_or_default()
}

fn modrinth_loader_slug(loader: &str) -> Option<&'static str> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "fabric" => Some("fabric"),
        "forge" => Some("forge"),
        "neoforge" => Some("neoforge"),
        "quilt" => Some("quilt"),
        _ => None,
    }
}

fn curseforge_mod_loader_type(loader: &str) -> Option<u32> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

fn normalized_game_versions(game_version: &str) -> Vec<String> {
    normalize_optional(game_version).into_iter().collect()
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn should_skip_hash_cache_update(err: &ModrinthError) -> bool {
    err.is_rate_limited()
        || matches!(
            err,
            ModrinthError::HttpStatus { .. }
                | ModrinthError::Transport(_)
                | ModrinthError::Read(_)
                | ModrinthError::Json(_)
        )
}

fn non_empty_owned(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn file_name_matches(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    !left.is_empty() && left.eq_ignore_ascii_case(right)
}

fn content_hash_cache_path(instance_root: &Path) -> PathBuf {
    instance_root
        .join(HASH_CACHE_DIR_NAME)
        .join(HASH_CACHE_FILE_NAME)
}

fn normalize_lookup_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_installed_content_path_key(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn derive_installed_lookup_query(path: &Path, fallback_file_name: &str) -> String {
    let raw = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(fallback_file_name)
        .trim();
    if raw.is_empty() {
        return fallback_file_name.to_owned();
    }

    let pieces: Vec<String> = raw
        .split(['-', '_'])
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .map(split_camel_case_words)
        .collect();
    if pieces.is_empty() {
        return split_camel_case_words(raw);
    }

    let mut kept = Vec::new();
    for piece in pieces {
        if looks_like_version_segment(piece.as_str()) {
            break;
        }
        kept.push(piece);
    }

    if kept.is_empty() {
        split_camel_case_words(raw)
    } else {
        kept.join(" ")
    }
}

fn derive_raw_lookup_query(path: &Path, fallback_file_name: &str) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(split_camel_case_words)
        .unwrap_or_else(|| split_camel_case_words(fallback_file_name))
}

fn looks_like_version_segment(value: &str) -> bool {
    let normalized = value
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.' && ch != '+')
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    if normalized.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }
    if normalized.starts_with('v')
        && normalized
            .chars()
            .skip(1)
            .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == '+')
    {
        return true;
    }
    if normalized.starts_with("mc")
        && normalized
            .chars()
            .skip(2)
            .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == '+')
    {
        return true;
    }
    if normalized.len() >= 8 && normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return true;
    }
    normalized.chars().any(|ch| ch.is_ascii_digit())
        && normalized.chars().any(|ch| ch == '.' || ch == '+')
}

fn managed_lookup_key_suffix(
    managed_identity: Option<&InstalledContentIdentity>,
    lookup_query: &str,
) -> String {
    if let Some(identity) = managed_identity {
        if let Some(project_id) = identity.modrinth_project_id.as_deref() {
            return format!("modrinth:{project_id}");
        }
        if let Some(project_id) = identity.curseforge_project_id {
            return format!("curseforge:{project_id}");
        }
    }
    normalize_lookup_key(lookup_query)
}

fn choose_preferred_content_entry(
    entries: Vec<UnifiedContentEntry>,
    lookup_key: &str,
    kind: InstalledContentKind,
) -> Option<UnifiedContentEntry> {
    let target_key = lookup_key
        .split_once("::")
        .map(|(_, value)| value)
        .unwrap_or(lookup_key);
    if target_key.trim().is_empty() {
        return None;
    }

    let lookup_tokens = split_lookup_tokens(target_key);
    let canonical_lookup_tokens = trim_ignorable_lookup_suffix(lookup_tokens.as_slice());
    let mut best: Option<(i32, UnifiedContentEntry)> = None;

    for entry in entries {
        let mut score = 0i32;
        if kind_accepts_content_type(kind, entry.content_type.as_str()) {
            score += 80;
        } else {
            continue;
        }

        let normalized_name = normalize_lookup_key(entry.name.as_str());
        let entry_tokens = split_lookup_tokens(normalized_name.as_str());
        let mut overlap = 0i32;
        for token in canonical_lookup_tokens {
            if token.len() < 2 {
                continue;
            }
            if entry_tokens.iter().any(|entry_token| entry_token == token) {
                overlap += 1;
            }
        }
        let candidate_covers_lookup = !canonical_lookup_tokens.is_empty()
            && canonical_lookup_tokens
                .iter()
                .all(|token| entry_tokens.iter().any(|entry_token| entry_token == token));
        let lookup_covers_candidate = query_has_only_ignorable_suffix_tokens(
            lookup_tokens.as_slice(),
            entry_tokens.as_slice(),
        );
        if normalized_name != target_key
            && entry_tokens.as_slice() != canonical_lookup_tokens
            && !candidate_covers_lookup
            && !lookup_covers_candidate
        {
            continue;
        }

        if normalized_name == target_key || entry_tokens.as_slice() == canonical_lookup_tokens {
            score += 600;
        } else {
            if candidate_covers_lookup {
                score += 300;
                if normalized_name.contains(target_key) {
                    score += 40;
                }
            }
            score += overlap * 60;
            if lookup_covers_candidate {
                score += 140;
            }
            score -= (entry_tokens.len() as i32 - canonical_lookup_tokens.len() as i32).abs() * 8;
        }

        let distance = levenshtein_distance(normalized_name.as_str(), target_key);
        score -= distance.min(64);

        if !entry.summary.trim().is_empty() {
            score += 8;
        }
        if entry.icon_url.is_some() {
            score += 10;
        }
        score += match entry.source {
            ContentSource::Modrinth => 20,
            ContentSource::CurseForge => 10,
        };

        let should_replace = best.as_ref().is_none_or(|(best_score, best_entry)| {
            score > *best_score
                || (score == *best_score
                    && content_source_priority(entry.source)
                        > content_source_priority(best_entry.source))
        });
        if should_replace {
            best = Some((score, entry));
        }
    }

    best.map(|(_, entry)| entry)
}

fn split_lookup_tokens(value: &str) -> Vec<&str> {
    value
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .collect()
}

fn trim_ignorable_lookup_suffix<'a>(tokens: &'a [&'a str]) -> &'a [&'a str] {
    let trimmed_len = tokens
        .iter()
        .rposition(|token| !is_ignorable_lookup_suffix_token(token))
        .map(|index| index + 1)
        .unwrap_or(tokens.len());
    &tokens[..trimmed_len]
}

fn is_ignorable_lookup_suffix_token(token: &str) -> bool {
    matches!(
        token,
        "fabric"
            | "forge"
            | "neoforge"
            | "quilt"
            | "rift"
            | "liteloader"
            | "loader"
            | "mod"
            | "mods"
            | "shader"
            | "shaders"
            | "shaderpack"
            | "shaderpacks"
            | "resourcepack"
            | "resourcepacks"
            | "texturepack"
            | "texturepacks"
            | "datapack"
            | "datapacks"
            | "minecraft"
            | "mc"
            | "client"
            | "server"
    )
}

fn split_camel_case_words(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 8);
    let mut chars = value.chars().peekable();
    let mut previous: Option<char> = None;

    while let Some(ch) = chars.next() {
        let next = chars.peek().copied();
        if let Some(prev) = previous
            && should_insert_camel_case_boundary(prev, ch, next)
            && !result.ends_with(' ')
        {
            result.push(' ');
        }
        result.push(ch);
        previous = Some(ch);
    }

    result
}

fn should_insert_camel_case_boundary(previous: char, current: char, next: Option<char>) -> bool {
    (previous.is_ascii_lowercase() && current.is_ascii_uppercase())
        || (previous.is_ascii_uppercase()
            && current.is_ascii_uppercase()
            && next.is_some_and(|next| next.is_ascii_lowercase()))
}

fn query_has_only_ignorable_suffix_tokens(
    query_tokens: &[&str],
    candidate_tokens: &[&str],
) -> bool {
    query_tokens.starts_with(candidate_tokens)
        && query_tokens[candidate_tokens.len()..]
            .iter()
            .all(|token| is_ignorable_lookup_suffix_token(token))
}

fn content_source_priority(source: ContentSource) -> i32 {
    match source {
        ContentSource::Modrinth => 2,
        ContentSource::CurseForge => 1,
    }
}

fn kind_accepts_content_type(kind: InstalledContentKind, content_type: &str) -> bool {
    let normalized_type = normalize_lookup_key(content_type);
    match kind {
        InstalledContentKind::Mods => normalized_type.contains("mod"),
        InstalledContentKind::ResourcePacks => {
            normalized_type.contains("resource pack")
                || normalized_type.contains("resourcepack")
                || normalized_type.contains("texture pack")
        }
        InstalledContentKind::ShaderPacks => normalized_type.contains("shader"),
        InstalledContentKind::DataPacks => {
            normalized_type.contains("data pack") || normalized_type.contains("datapack")
        }
    }
}

fn levenshtein_distance(left: &str, right: &str) -> i32 {
    if left == right {
        return 0;
    }
    if left.is_empty() {
        return right.chars().count() as i32;
    }
    if right.is_empty() {
        return left.chars().count() as i32;
    }

    let right_chars: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right_chars.len()).collect();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution_cost = usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution_cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right_chars.len()] as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, source: ContentSource) -> UnifiedContentEntry {
        UnifiedContentEntry {
            id: format!("{}:{name}", source.label().to_ascii_lowercase()),
            name: name.to_owned(),
            summary: String::new(),
            content_type: "mod".to_owned(),
            source,
            project_url: None,
            icon_url: None,
        }
    }

    #[test]
    fn autodetect_prefers_exact_multi_token_name_over_short_prefix() {
        let selected = choose_preferred_content_entry(
            vec![
                entry("Voxy", ContentSource::Modrinth),
                entry("Voxy Worldgen", ContentSource::CurseForge),
            ],
            "mods::voxy worldgen",
            InstalledContentKind::Mods,
        )
        .expect("expected a matching entry");

        assert_eq!(selected.name, "Voxy Worldgen");
    }

    #[test]
    fn autodetect_allows_trailing_loader_noise_in_filename_queries() {
        let selected = choose_preferred_content_entry(
            vec![entry("Sodium", ContentSource::Modrinth)],
            "mods::sodium fabric",
            InstalledContentKind::Mods,
        )
        .expect("expected a matching entry");

        assert_eq!(selected.name, "Sodium");
    }

    #[test]
    fn autodetect_keeps_short_name_when_lookup_is_short_name() {
        let selected = choose_preferred_content_entry(
            vec![
                entry("Voxy", ContentSource::Modrinth),
                entry("Voxy Worldgen", ContentSource::CurseForge),
            ],
            "mods::voxy",
            InstalledContentKind::Mods,
        )
        .expect("expected a matching entry");

        assert_eq!(selected.name, "Voxy");
    }

    #[test]
    fn raw_lookup_query_preserves_full_jar_stem_for_fallback_search() {
        let path = Path::new("mods/foomod-neoforge-mc1.21.1-v1.3.0.jar");

        assert_eq!(
            derive_installed_lookup_query(path, "foomod-neoforge-mc1.21.1-v1.3.0.jar"),
            "foomod neoforge"
        );
        assert_eq!(
            derive_raw_lookup_query(path, "foomod-neoforge-mc1.21.1-v1.3.0.jar"),
            "foomod-neoforge-mc1.21.1-v1.3.0"
        );
    }

    #[test]
    fn shader_lookup_query_splits_camel_case_file_names() {
        let path = Path::new("shaderpacks/ComplementaryUnbound_r5.4.1.zip");

        assert_eq!(
            derive_installed_lookup_query(path, "ComplementaryUnbound_r5.4.1.zip"),
            "Complementary Unbound"
        );
        assert_eq!(
            derive_raw_lookup_query(path, "ComplementaryUnbound_r5.4.1.zip"),
            "Complementary Unbound_r5.4.1"
        );
    }

    #[test]
    fn levenshtein_distance_prefers_nearest_match() {
        assert!(levenshtein_distance("sodium", "sodium") < levenshtein_distance("sodium", "sod"));
        assert!(
            levenshtein_distance("iris shaders", "iris")
                < levenshtein_distance("iris shaders", "indium")
        );
    }

    #[test]
    fn autodetect_ignores_shader_suffix_tokens() {
        let selected = choose_preferred_content_entry(
            vec![UnifiedContentEntry {
                id: "modrinth:complementary-unbound".to_owned(),
                name: "Complementary Unbound".to_owned(),
                summary: String::new(),
                content_type: "shader".to_owned(),
                source: ContentSource::Modrinth,
                project_url: None,
                icon_url: None,
            }],
            "shaderpacks::complementary unbound shaders",
            InstalledContentKind::ShaderPacks,
        )
        .expect("expected a matching shader entry");

        assert_eq!(selected.name, "Complementary Unbound");
    }

    #[test]
    fn resolve_uses_lookup_cache_before_falling_back_to_heuristic_search() {
        let request = ResolveInstalledContentRequest {
            file_path: PathBuf::from("resourcepacks/fresh-animations.zip"),
            disk_file_name: "fresh-animations.zip".to_owned(),
            lookup_query: "fresh animations".to_owned(),
            fallback_lookup_key: None,
            fallback_lookup_query: None,
            managed_identity: None,
            kind: InstalledContentKind::ResourcePacks,
            game_version: "1.21.1".to_owned(),
            loader: "fabric".to_owned(),
        };
        let mut hash_cache = InstalledContentHashCache::default();
        hash_cache.entries.insert(
            "lookup::resourcepacks::fresh animations".to_owned(),
            Some(ResolvedInstalledContent {
                entry: UnifiedContentEntry {
                    id: "modrinth:freshanimations".to_owned(),
                    name: "Fresh Animations".to_owned(),
                    summary: String::new(),
                    content_type: "resourcepack".to_owned(),
                    source: ContentSource::Modrinth,
                    project_url: None,
                    icon_url: None,
                },
                installed_version_id: None,
                installed_version_label: None,
                resolution_kind: InstalledContentResolutionKind::HeuristicSearch,
                warning_message: Some(HEURISTIC_WARNING_MESSAGE.to_owned()),
                update: None,
            }),
        );

        let result = InstalledContentResolver::resolve(&request, &hash_cache);
        let resolution = result
            .resolution
            .expect("expected lookup cache to resolve content");

        assert_eq!(resolution.entry.name, "Fresh Animations");
        assert!(result.hash_cache_updates.is_empty());
    }

    #[test]
    fn lookup_cache_updates_seed_canonical_query_aliases() {
        let request = ResolveInstalledContentRequest {
            file_path: PathBuf::from("mods/sodium-fabric-0.6.0.jar"),
            disk_file_name: "sodium-fabric-0.6.0.jar".to_owned(),
            lookup_query: "sodium fabric".to_owned(),
            fallback_lookup_key: None,
            fallback_lookup_query: None,
            managed_identity: None,
            kind: InstalledContentKind::Mods,
            game_version: "1.21.1".to_owned(),
            loader: "fabric".to_owned(),
        };
        let resolution = ResolvedInstalledContent {
            entry: entry("Sodium", ContentSource::Modrinth),
            installed_version_id: Some("version-1".to_owned()),
            installed_version_label: Some("0.6.0".to_owned()),
            resolution_kind: InstalledContentResolutionKind::ExactHash,
            warning_message: None,
            update: None,
        };

        let updates = lookup_cache_updates_for_request(&request, &resolution);

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].hash_key, "lookup::mods::sodium");
        assert_eq!(
            updates[0]
                .resolution
                .as_ref()
                .and_then(|value| value.warning_message.as_deref()),
            Some(HEURISTIC_WARNING_MESSAGE)
        );
        assert_eq!(
            updates[0]
                .resolution
                .as_ref()
                .and_then(|value| value.installed_version_id.as_deref()),
            None
        );
    }

    #[test]
    fn modrinth_absence_cache_only_treats_not_found_as_stable() {
        assert!(should_cache_modrinth_absence(&ModrinthError::HttpStatus {
            status: 404,
            body: String::new(),
        }));
        assert!(!should_cache_modrinth_absence(&ModrinthError::HttpStatus {
            status: 429,
            body: String::new(),
        }));
        assert!(!should_cache_modrinth_absence(&ModrinthError::Transport(
            "timeout".to_owned(),
        )));
    }

    #[test]
    fn curseforge_absence_cache_only_treats_not_found_as_stable() {
        assert!(should_cache_curseforge_absence(
            &curseforge::CurseForgeError::HttpStatus {
                status: 404,
                body: String::new(),
            }
        ));
        assert!(!should_cache_curseforge_absence(
            &curseforge::CurseForgeError::HttpStatus {
                status: 429,
                body: String::new(),
            }
        ));
        assert!(!should_cache_curseforge_absence(
            &curseforge::CurseForgeError::Transport("timeout".to_owned())
        ));
    }
}
