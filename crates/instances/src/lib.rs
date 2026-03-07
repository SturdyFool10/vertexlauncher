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

#[track_caller]
fn fs_read_to_string(path: impl AsRef<Path>) -> std::io::Result<String> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    fs::read_to_string(path)
}

#[track_caller]
fn fs_create_dir_all(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display());
    fs::create_dir_all(path)
}

#[track_caller]
fn fs_file_create(path: impl AsRef<Path>) -> std::io::Result<fs::File> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "file_create", path = %path.display());
    fs::File::create(path)
}

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct InstanceRecord {
    pub id: String,
    pub name: String,
    pub minecraft_root: String,
    pub thumbnail_path: Option<String>,
    pub modloader: String,
    pub game_version: String,
    pub modloader_version: String,
    pub max_memory_mib: Option<u128>,
    pub cli_args: Option<String>,
}

impl Default for InstanceRecord {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: DEFAULT_INSTANCE_NAME.to_owned(),
            minecraft_root: "instance".to_owned(),
            thumbnail_path: None,
            modloader: DEFAULT_MODLOADER.to_owned(),
            game_version: DEFAULT_GAME_VERSION.to_owned(),
            modloader_version: String::new(),
            max_memory_mib: None,
            cli_args: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct InstanceStore {
    pub instances: Vec<InstanceRecord>,
}

#[derive(Clone, Debug)]
pub struct NewInstanceSpec {
    pub name: String,
    pub thumbnail_path: Option<String>,
    pub modloader: String,
    pub game_version: String,
    pub modloader_version: String,
}

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
    pub fn normalize(&mut self) {
        for instance in &mut self.instances {
            normalize_instance(instance);
        }
    }

    pub fn find(&self, id: &str) -> Option<&InstanceRecord> {
        self.instances.iter().find(|instance| instance.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut InstanceRecord> {
        self.instances
            .iter_mut()
            .find(|instance| instance.id.as_str() == id)
    }
}

pub fn load_store() -> Result<InstanceStore, InstanceError> {
    let path = store_path();
    match fs_read_to_string(path) {
        Ok(raw) => {
            let mut store: InstanceStore = serde_json::from_str(&raw)?;
            store.normalize();
            Ok(store)
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(InstanceStore::default()),
        Err(err) => Err(InstanceError::Io(err)),
    }
}

pub fn save_store(store: &InstanceStore) -> Result<(), InstanceError> {
    let mut normalized = store.clone();
    normalized.normalize();

    let path = store_path();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs_create_dir_all(parent)?;
    }

    let file = fs_file_create(path)?;
    serde_json::to_writer_pretty(file, &normalized)?;
    Ok(())
}

pub fn create_instance(
    store: &mut InstanceStore,
    installations_root: &Path,
    spec: NewInstanceSpec,
) -> Result<InstanceRecord, InstanceError> {
    let name = required(spec.name, InstanceError::EmptyName)?;
    let modloader = required(spec.modloader, InstanceError::EmptyModloader)?;
    let game_version = required(spec.game_version, InstanceError::EmptyGameVersion)?;
    let modloader_version = spec.modloader_version.trim().to_owned();
    let thumbnail_path = normalize_optional_string(spec.thumbnail_path.as_deref());

    fs_create_dir_all(installations_root)?;
    let minecraft_root = unique_minecraft_root(store, installations_root, &name);
    let instance_root_path = installations_root.join(&minecraft_root);
    fs_create_dir_all(&instance_root_path)?;
    fs_create_dir_all(instance_root_path.join("mods"))?;

    let instance = InstanceRecord {
        id: next_instance_id(),
        name,
        minecraft_root,
        thumbnail_path,
        modloader,
        game_version,
        modloader_version,
        max_memory_mib: None,
        cli_args: None,
    };

    store.instances.push(instance.clone());
    Ok(instance)
}

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
    Ok(())
}

pub fn set_instance_settings(
    store: &mut InstanceStore,
    id: &str,
    max_memory_mib: Option<u128>,
    cli_args: Option<String>,
) -> Result<(), InstanceError> {
    let instance = store
        .find_mut(id)
        .ok_or_else(|| InstanceError::MissingInstance(id.to_owned()))?;
    instance.max_memory_mib = max_memory_mib;
    instance.cli_args = normalize_optional_string(cli_args.as_deref());
    Ok(())
}

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
    Ok(destination)
}

pub fn instance_root_path(installations_root: &Path, instance: &InstanceRecord) -> PathBuf {
    installations_root.join(&instance.minecraft_root)
}

pub fn store_path() -> PathBuf {
    match std::env::var("VERTEX_CONFIG_LOCATION") {
        Ok(dir) => PathBuf::from(dir).join(INSTANCES_FILENAME),
        Err(_) => PathBuf::from(INSTANCES_FILENAME),
    }
}

fn normalize_instance(instance: &mut InstanceRecord) {
    instance.name = required(std::mem::take(&mut instance.name), InstanceError::EmptyName)
        .unwrap_or_else(|_| DEFAULT_INSTANCE_NAME.to_owned());

    if instance.minecraft_root.trim().is_empty() {
        instance.minecraft_root = slugify_name(&instance.name);
    } else {
        instance.minecraft_root = sanitize_root_name(&instance.minecraft_root);
    }

    instance.thumbnail_path = normalize_optional_string(instance.thumbnail_path.as_deref());
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

    if instance.id.trim().is_empty() {
        instance.id = next_instance_id();
    } else {
        instance.id = instance.id.trim().to_owned();
    }
}

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

fn sanitize_root_name(value: &str) -> String {
    slugify_name(value)
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn required(value: String, err: InstanceError) -> Result<String, InstanceError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(err)
    } else {
        Ok(trimmed.to_owned())
    }
}

fn next_instance_id() -> String {
    let epoch_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let counter = NEXT_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("instance-{epoch_millis}-{counter}")
}
