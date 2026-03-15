use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const INSTANCES_FILENAME: &str = "instances.json";
const DEFAULT_INSTANCE_NAME: &str = "Instance";
const DEFAULT_MODLOADER: &str = "Vanilla";
const DEFAULT_GAME_VERSION: &str = "latest";

static NEXT_INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Wrapper around `fs::read_to_string` with structured IO tracing.
#[track_caller]
fn fs_read_to_string(path: impl AsRef<Path>) -> std::io::Result<String> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    fs::read_to_string(path)
}

/// Wrapper around `fs::create_dir_all` with structured IO tracing.
#[track_caller]
fn fs_create_dir_all(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display());
    fs::create_dir_all(path)
}

/// Wrapper around `File::create` with structured IO tracing.
#[track_caller]
fn fs_file_create(path: impl AsRef<Path>) -> std::io::Result<fs::File> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "file_create", path = %path.display());
    fs::File::create(path)
}

/// Wrapper around `fs::copy` with structured IO tracing.
#[track_caller]
fn fs_copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> std::io::Result<u64> {
    let from = from.as_ref();
    let to = to.as_ref();
    tracing::debug!(
        target: "vertexlauncher/io",
        op = "copy",
        from = %from.display(),
        to = %to.display()
    );
    fs::copy(from, to)
}

/// Wrapper around `fs::remove_dir_all` with structured IO tracing.
#[track_caller]
fn fs_remove_dir_all(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(
        target: "vertexlauncher/io",
        op = "remove_dir_all",
        path = %path.display()
    );
    fs::remove_dir_all(path)
}

/// Persisted record describing a launcher instance.
///
/// Field expectations:
/// - `id`: unique, non-empty identifier.
/// - `name`: human-readable display name.
/// - `minecraft_root`: relative directory name for this instance under the
///   launcher installations root.
/// - `description`: optional short user description shown in library views.
/// - `modloader`, `game_version`: non-empty normalized values.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct InstanceRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub minecraft_root: String,
    pub thumbnail_path: Option<String>,
    pub modloader: String,
    pub game_version: String,
    pub modloader_version: String,
    pub max_memory_mib: Option<u128>,
    pub cli_args: Option<String>,
    pub java_override_enabled: bool,
    pub java_override_runtime_major: Option<u8>,
    pub linux_set_opengl_driver: Option<bool>,
    pub linux_use_zink_driver: Option<bool>,
    pub launch_count: u64,
    pub last_launched_at_ms: Option<u64>,
    pub favorite_world_ids: Vec<String>,
    pub favorite_server_ids: Vec<String>,
}

impl Default for InstanceRecord {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: DEFAULT_INSTANCE_NAME.to_owned(),
            description: None,
            minecraft_root: "instance".to_owned(),
            thumbnail_path: None,
            modloader: DEFAULT_MODLOADER.to_owned(),
            game_version: DEFAULT_GAME_VERSION.to_owned(),
            modloader_version: String::new(),
            max_memory_mib: None,
            cli_args: None,
            java_override_enabled: false,
            java_override_runtime_major: None,
            linux_set_opengl_driver: None,
            linux_use_zink_driver: None,
            launch_count: 0,
            last_launched_at_ms: None,
            favorite_world_ids: Vec::new(),
            favorite_server_ids: Vec::new(),
        }
    }
}

/// On-disk collection of launcher instances.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct InstanceStore {
    pub instances: Vec<InstanceRecord>,
}

/// Inputs required to create a new instance record and directory layout.
#[derive(Clone, Debug)]
pub struct NewInstanceSpec {
    pub name: String,
    pub description: Option<String>,
    pub thumbnail_path: Option<String>,
    pub modloader: String,
    pub game_version: String,
    pub modloader_version: String,
}

/// Errors raised by instance store and filesystem operations.
#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Instance name cannot be empty")]
    EmptyName,
    #[error("Minecraft game version cannot be empty")]
    EmptyGameVersion,
    #[error("Modloader cannot be empty")]
    EmptyModloader,
    #[error("Could not find instance '{0}'")]
    MissingInstance(String),
    #[error("Mod file path cannot be empty")]
    EmptyModFilePath,
    #[error("Mod file does not exist: {0}")]
    MissingModFile(String),
    #[error("Mod file has no file name component")]
    InvalidModFileName,
}

impl InstanceStore {
    /// Normalizes all records into valid launcher defaults.
    ///
    /// This is safe to call repeatedly and is used when loading/saving store
    /// content from disk.
    pub fn normalize(&mut self) {
        for instance in &mut self.instances {
            normalize_instance(instance);
        }
    }

    /// Returns the first instance matching `id`, if present.
    pub fn find(&self, id: &str) -> Option<&InstanceRecord> {
        self.instances.iter().find(|instance| instance.id == id)
    }

    /// Returns a mutable instance matching `id`, if present.
    pub fn find_mut(&mut self, id: &str) -> Option<&mut InstanceRecord> {
        self.instances
            .iter_mut()
            .find(|instance| instance.id.as_str() == id)
    }
}

/// Loads the instance store from disk.
///
/// Missing files are treated as an empty store. Any present records are
/// normalized to enforce current defaults.
pub fn load_store() -> Result<InstanceStore, InstanceError> {
    let path = store_path();
    tracing::debug!(
        target: "vertexlauncher/instances",
        path = %path.display(),
        "loading instance store"
    );

    match fs_read_to_string(&path) {
        Ok(raw) => {
            let mut store: InstanceStore = serde_json::from_str(&raw)?;
            store.normalize();
            tracing::debug!(
                target: "vertexlauncher/instances",
                path = %path.display(),
                count = store.instances.len(),
                "loaded instance store"
            );
            Ok(store)
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            tracing::debug!(
                target: "vertexlauncher/instances",
                path = %path.display(),
                "instance store file missing; using empty store"
            );
            Ok(InstanceStore::default())
        }
        Err(err) => {
            tracing::warn!(
                target: "vertexlauncher/instances",
                path = %path.display(),
                error = %err,
                "failed to read instance store"
            );
            Err(InstanceError::Io(err))
        }
    }
}

/// Saves the instance store to disk after normalizing records.
pub fn save_store(store: &InstanceStore) -> Result<(), InstanceError> {
    let mut normalized = store.clone();
    normalized.normalize();

    let path = store_path();
    tracing::debug!(
        target: "vertexlauncher/instances",
        path = %path.display(),
        count = normalized.instances.len(),
        "saving instance store"
    );
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs_create_dir_all(parent)?;
    }

    let file = fs_file_create(path)?;
    serde_json::to_writer_pretty(file, &normalized)?;
    tracing::debug!(
        target: "vertexlauncher/instances",
        count = normalized.instances.len(),
        "instance store saved"
    );
    Ok(())
}

/// Creates a new instance, allocating a unique root directory and `mods/` folder.
///
/// Fails when required values are blank or filesystem operations fail.
pub fn create_instance(
    store: &mut InstanceStore,
    installations_root: &Path,
    spec: NewInstanceSpec,
) -> Result<InstanceRecord, InstanceError> {
    let name = required(spec.name, InstanceError::EmptyName)?;
    let modloader = required(spec.modloader, InstanceError::EmptyModloader)?;
    let game_version = required(spec.game_version, InstanceError::EmptyGameVersion)?;
    let modloader_version = spec.modloader_version.trim().to_owned();
    let description = normalize_optional_string(spec.description.as_deref());
    let thumbnail_path = normalize_optional_string(spec.thumbnail_path.as_deref());

    fs_create_dir_all(installations_root)?;
    let minecraft_root = unique_minecraft_root(store, installations_root, &name);
    let instance_root_path = installations_root.join(&minecraft_root);
    fs_create_dir_all(&instance_root_path)?;
    fs_create_dir_all(instance_root_path.join("mods"))?;

    let instance = InstanceRecord {
        id: next_instance_id(),
        name,
        description,
        minecraft_root,
        thumbnail_path,
        modloader,
        game_version,
        modloader_version,
        max_memory_mib: None,
        cli_args: None,
        java_override_enabled: false,
        java_override_runtime_major: None,
        linux_set_opengl_driver: None,
        linux_use_zink_driver: None,
        launch_count: 0,
        last_launched_at_ms: None,
        favorite_world_ids: Vec::new(),
        favorite_server_ids: Vec::new(),
    };

    store.instances.push(instance.clone());
    tracing::info!(
        target: "vertexlauncher/instances",
        id = instance.id.as_str(),
        name = instance.name.as_str(),
        minecraft_root = instance.minecraft_root.as_str(),
        modloader = instance.modloader.as_str(),
        game_version = instance.game_version.as_str(),
        has_description = instance.description.is_some(),
        has_modloader_version = !instance.modloader_version.is_empty(),
        "created instance"
    );
    Ok(instance)
}

/// Updates modloader and version settings for an existing instance.
///
/// All string inputs except `modloader_version` must be non-empty after trim.
pub fn set_instance_versions(
    store: &mut InstanceStore,
    id: &str,
    modloader: String,
    game_version: String,
    modloader_version: String,
) -> Result<(), InstanceError> {
    let modloader = required(modloader, InstanceError::EmptyModloader)?;
    let game_version = required(game_version, InstanceError::EmptyGameVersion)?;
    let modloader_version = modloader_version.trim().to_owned();

    let instance = store
        .find_mut(id)
        .ok_or_else(|| InstanceError::MissingInstance(id.to_owned()))?;
    instance.modloader = modloader;
    instance.game_version = game_version;
    instance.modloader_version = modloader_version;
    tracing::info!(
        target: "vertexlauncher/instances",
        id,
        modloader = instance.modloader.as_str(),
        game_version = instance.game_version.as_str(),
        has_modloader_version = !instance.modloader_version.is_empty(),
        "updated instance versions"
    );
    Ok(())
}

/// Updates runtime settings for an existing instance.
///
/// `max_memory_mib` accepts `None` (launcher default) or a positive integer in
/// MiB. `cli_args` is trimmed and stored only when non-empty.
pub fn set_instance_settings(
    store: &mut InstanceStore,
    id: &str,
    max_memory_mib: Option<u128>,
    cli_args: Option<String>,
    java_override_enabled: bool,
    java_override_runtime_major: Option<u8>,
    linux_set_opengl_driver: Option<bool>,
    linux_use_zink_driver: Option<bool>,
) -> Result<(), InstanceError> {
    let instance = store
        .find_mut(id)
        .ok_or_else(|| InstanceError::MissingInstance(id.to_owned()))?;
    instance.max_memory_mib = max_memory_mib;
    instance.cli_args = normalize_optional_string(cli_args.as_deref());
    instance.java_override_enabled = java_override_enabled;
    instance.java_override_runtime_major =
        normalize_java_override(java_override_enabled, java_override_runtime_major);
    instance.linux_set_opengl_driver = linux_set_opengl_driver;
    instance.linux_use_zink_driver = linux_use_zink_driver;
    tracing::debug!(
        target: "vertexlauncher/instances",
        id,
        max_memory_mib = ?instance.max_memory_mib,
        has_cli_args = instance.cli_args.is_some(),
        java_override_enabled = instance.java_override_enabled,
        java_override_runtime_major = ?instance.java_override_runtime_major,
        linux_set_opengl_driver = ?instance.linux_set_opengl_driver,
        linux_use_zink_driver = ?instance.linux_use_zink_driver,
        "updated instance runtime settings"
    );
    Ok(())
}

/// Records successful instance usage (launch count and last-used timestamp).
pub fn record_instance_launch_usage(
    store: &mut InstanceStore,
    id: &str,
) -> Result<(), InstanceError> {
    let instance = store
        .find_mut(id)
        .ok_or_else(|| InstanceError::MissingInstance(id.to_owned()))?;
    instance.launch_count = instance.launch_count.saturating_add(1);
    instance.last_launched_at_ms = Some(current_time_millis());
    tracing::debug!(
        target: "vertexlauncher/instances",
        id,
        launch_count = instance.launch_count,
        last_launched_at_ms = ?instance.last_launched_at_ms,
        "recorded instance launch usage"
    );
    Ok(())
}

/// Toggles a world favorite for an instance by world identifier.
pub fn set_world_favorite(
    store: &mut InstanceStore,
    id: &str,
    world_id: &str,
    favorite: bool,
) -> Result<(), InstanceError> {
    let normalized_world_id = world_id.trim();
    if normalized_world_id.is_empty() {
        return Ok(());
    }
    let instance = store
        .find_mut(id)
        .ok_or_else(|| InstanceError::MissingInstance(id.to_owned()))?;
    if favorite {
        if !instance
            .favorite_world_ids
            .iter()
            .any(|entry| entry == normalized_world_id)
        {
            instance
                .favorite_world_ids
                .push(normalized_world_id.to_owned());
        }
    } else {
        instance
            .favorite_world_ids
            .retain(|entry| entry != normalized_world_id);
    }
    Ok(())
}

/// Toggles a server favorite for an instance by normalized server identifier.
pub fn set_server_favorite(
    store: &mut InstanceStore,
    id: &str,
    server_id: &str,
    favorite: bool,
) -> Result<(), InstanceError> {
    let normalized_server_id = server_id.trim();
    if normalized_server_id.is_empty() {
        return Ok(());
    }
    let instance = store
        .find_mut(id)
        .ok_or_else(|| InstanceError::MissingInstance(id.to_owned()))?;
    if favorite {
        if !instance
            .favorite_server_ids
            .iter()
            .any(|entry| entry == normalized_server_id)
        {
            instance
                .favorite_server_ids
                .push(normalized_server_id.to_owned());
        }
    } else {
        instance
            .favorite_server_ids
            .retain(|entry| entry != normalized_server_id);
    }
    Ok(())
}

/// Copies a local mod file into the selected instance's `mods` directory.
///
/// `source_mod_path` must reference an existing file path with a file name.
pub fn add_mod_file_to_instance(
    store: &InstanceStore,
    id: &str,
    installations_root: &Path,
    source_mod_path: &str,
) -> Result<PathBuf, InstanceError> {
    let source_mod_path = source_mod_path.trim();
    if source_mod_path.is_empty() {
        return Err(InstanceError::EmptyModFilePath);
    }

    let source = PathBuf::from(source_mod_path);
    if !source.exists() {
        tracing::warn!(
            target: "vertexlauncher/instances",
            id,
            path = source_mod_path,
            "mod file path does not exist"
        );
        return Err(InstanceError::MissingModFile(source_mod_path.to_owned()));
    }

    let file_name = source
        .file_name()
        .ok_or(InstanceError::InvalidModFileName)?
        .to_owned();

    let instance = store
        .find(id)
        .ok_or_else(|| InstanceError::MissingInstance(id.to_owned()))?;
    let mods_dir = instance_root_path(installations_root, instance).join("mods");
    fs_create_dir_all(&mods_dir)?;

    let destination = mods_dir.join(file_name);
    fs_copy(source, &destination)?;
    tracing::info!(
        target: "vertexlauncher/instances",
        id,
        destination = %destination.display(),
        "copied mod file into instance"
    );
    Ok(destination)
}

/// Deletes an instance record and its root directory.
///
/// Missing instance roots on disk are tolerated so stale metadata can still be
/// cleaned up. The instance record is removed only after filesystem deletion
/// succeeds or the directory is already absent.
pub fn delete_instance(
    store: &mut InstanceStore,
    id: &str,
    installations_root: &Path,
) -> Result<InstanceRecord, InstanceError> {
    let Some(index) = store
        .instances
        .iter()
        .position(|instance| instance.id == id)
    else {
        return Err(InstanceError::MissingInstance(id.to_owned()));
    };

    let instance = store.instances[index].clone();
    let instance_root = instance_root_path(installations_root, &instance);
    match fs_remove_dir_all(&instance_root) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => {
            tracing::warn!(
                target: "vertexlauncher/instances",
                id,
                path = %instance_root.display(),
                error = %err,
                "failed to delete instance root"
            );
            return Err(InstanceError::Io(err));
        }
    }

    let removed = store.instances.remove(index);
    tracing::info!(
        target: "vertexlauncher/instances",
        id = removed.id.as_str(),
        name = removed.name.as_str(),
        root = %instance_root.display(),
        "deleted instance"
    );
    Ok(removed)
}

/// Resolves the absolute filesystem path to the given instance root directory.
#[must_use]
pub fn instance_root_path(installations_root: &Path, instance: &InstanceRecord) -> PathBuf {
    installations_root.join(&instance.minecraft_root)
}

/// Returns whether this instance has an explicit Linux graphics override.
#[must_use]
pub fn linux_graphics_override_enabled(instance: &InstanceRecord) -> bool {
    instance.linux_set_opengl_driver == Some(true)
}

/// Resolves the effective Linux graphics settings for an instance.
///
/// Instance `linux_set_opengl_driver` acts as an override toggle: only
/// `Some(true)` enables per-instance control. Any other stored value inherits
/// the launcher-wide settings.
#[must_use]
pub fn effective_linux_graphics_settings(
    instance: &InstanceRecord,
    global_set_opengl_driver: bool,
    global_use_zink_driver: bool,
) -> (bool, bool) {
    if linux_graphics_override_enabled(instance) {
        (
            true,
            instance
                .linux_use_zink_driver
                .unwrap_or(global_use_zink_driver),
        )
    } else {
        (global_set_opengl_driver, global_use_zink_driver)
    }
}

/// Returns the path used for persistent instance metadata (`instances.json`).
///
/// Uses `VERTEX_CONFIG_LOCATION/instances.json` when set, otherwise
/// `./instances.json` relative to the current process directory.
#[must_use]
pub fn store_path() -> PathBuf {
    match std::env::var("VERTEX_CONFIG_LOCATION") {
        Ok(dir) => PathBuf::from(dir).join(INSTANCES_FILENAME),
        Err(_) => PathBuf::from(INSTANCES_FILENAME),
    }
}

/// Normalizes a record into launcher defaults and sanitized persisted values.
fn normalize_instance(instance: &mut InstanceRecord) {
    instance.name = required(std::mem::take(&mut instance.name), InstanceError::EmptyName)
        .unwrap_or_else(|_| DEFAULT_INSTANCE_NAME.to_owned());

    if instance.minecraft_root.trim().is_empty() {
        instance.minecraft_root = slugify_name(&instance.name);
    } else {
        instance.minecraft_root = sanitize_root_name(&instance.minecraft_root);
    }

    instance.thumbnail_path = normalize_optional_string(instance.thumbnail_path.as_deref());
    instance.description = normalize_optional_string(instance.description.as_deref());
    instance.modloader = required(
        std::mem::take(&mut instance.modloader),
        InstanceError::EmptyModloader,
    )
    .unwrap_or_else(|_| DEFAULT_MODLOADER.to_owned());
    instance.game_version = required(
        std::mem::take(&mut instance.game_version),
        InstanceError::EmptyGameVersion,
    )
    .unwrap_or_else(|_| DEFAULT_GAME_VERSION.to_owned());
    instance.modloader_version = instance.modloader_version.trim().to_owned();
    instance.cli_args = normalize_optional_string(instance.cli_args.as_deref());
    instance.java_override_runtime_major = normalize_java_override(
        instance.java_override_enabled,
        instance.java_override_runtime_major,
    );
    if !linux_graphics_override_enabled(instance) {
        instance.linux_set_opengl_driver = None;
        instance.linux_use_zink_driver = None;
    } else {
        instance.linux_set_opengl_driver = Some(true);
    }
    instance.favorite_world_ids = normalize_world_favorites(&instance.favorite_world_ids);
    instance.favorite_server_ids = normalize_world_favorites(&instance.favorite_server_ids);

    if instance.id.trim().is_empty() {
        instance.id = next_instance_id();
    } else {
        instance.id = instance.id.trim().to_owned();
    }
}

fn normalize_world_favorites(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let key = value.to_owned();
        if seen.insert(key.clone()) {
            normalized.push(key);
        }
    }
    normalized
}

/// Generates a unique, filesystem-safe root name for a new instance.
fn unique_minecraft_root(
    store: &InstanceStore,
    installations_root: &Path,
    instance_name: &str,
) -> String {
    let base = slugify_name(instance_name);
    let existing: HashSet<&str> = store
        .instances
        .iter()
        .map(|instance| instance.minecraft_root.as_str())
        .collect();

    if !existing.contains(base.as_str()) && !installations_root.join(&base).exists() {
        return base;
    }

    let mut suffix: u32 = 2;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !existing.contains(candidate.as_str()) && !installations_root.join(&candidate).exists() {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}

/// Converts a display name into a lowercase ASCII slug.
///
/// Empty or non-alphanumeric-only values normalize to `"instance"`.
fn slugify_name(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }

    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "instance".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Sanitizes persisted root names into a safe directory slug.
fn sanitize_root_name(value: &str) -> String {
    slugify_name(value)
}

/// Trims optional user input and drops it when empty.
fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_java_override(enabled: bool, runtime_major: Option<u8>) -> Option<u8> {
    if !enabled {
        return None;
    }
    runtime_major.filter(|major| matches!(major, 8 | 16 | 17 | 21))
}

/// Validates a required string input by trimming and rejecting empties.
fn required(value: String, err: InstanceError) -> Result<String, InstanceError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(err)
    } else {
        Ok(trimmed.to_owned())
    }
}

/// Generates a mostly monotonic instance id composed of epoch millis + counter.
fn next_instance_id() -> String {
    let epoch_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let counter = NEXT_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("instance-{epoch_millis}-{counter}")
}

fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        InstanceRecord, effective_linux_graphics_settings, linux_graphics_override_enabled,
        normalize_instance,
    };

    #[test]
    fn linux_graphics_inherit_global_when_override_is_disabled() {
        let instance = InstanceRecord {
            linux_set_opengl_driver: None,
            linux_use_zink_driver: Some(false),
            ..InstanceRecord::default()
        };
        assert!(!linux_graphics_override_enabled(&instance));
        assert_eq!(
            effective_linux_graphics_settings(&instance, true, true),
            (true, true)
        );

        let legacy_false = InstanceRecord {
            linux_set_opengl_driver: Some(false),
            linux_use_zink_driver: Some(false),
            ..InstanceRecord::default()
        };
        assert!(!linux_graphics_override_enabled(&legacy_false));
        assert_eq!(
            effective_linux_graphics_settings(&legacy_false, true, true),
            (true, true)
        );
    }

    #[test]
    fn linux_graphics_use_instance_zink_when_override_is_enabled() {
        let instance = InstanceRecord {
            linux_set_opengl_driver: Some(true),
            linux_use_zink_driver: Some(false),
            ..InstanceRecord::default()
        };
        assert!(linux_graphics_override_enabled(&instance));
        assert_eq!(
            effective_linux_graphics_settings(&instance, false, true),
            (true, false)
        );
    }

    #[test]
    fn normalize_clears_legacy_linux_graphics_values_when_override_is_disabled() {
        let mut instance = InstanceRecord {
            linux_set_opengl_driver: Some(false),
            linux_use_zink_driver: Some(true),
            ..InstanceRecord::default()
        };
        normalize_instance(&mut instance);
        assert_eq!(instance.linux_set_opengl_driver, None);
        assert_eq!(instance.linux_use_zink_driver, None);
    }
}
