//! Resource pack sync: shares packs between opted-in instances where their metadata (or the
//! user's override) says they work, and keeps the enabled list in step for those packs.

use vertex_i18n::tr;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use instances::{InstanceStore, PackOverride};
use mc_options::packs::{Compat, PackSupport, compatibility, parse_pack_mcmeta};
use mc_options::{OptionsFile, parse_game_version, value};

use crate::fs_util::{copy_dir_recursive, modified, write_atomic_bytes};
use crate::{SyncInstance, SyncReport};

const PACKS_DIR: &str = "resourcepacks";

/// A resource pack in an instance's `resourcepacks` folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackInfo {
    /// File or folder name (the id the game stores is `file/<name>`).
    pub name: String,
    pub is_dir: bool,
    /// Declared support, normalized; `None` when the pack has no readable metadata.
    pub support: Option<PackSupport>,
    pub modified: Option<SystemTime>,
}

type SupportCache = HashMap<PathBuf, (u64, Option<SystemTime>, Option<PackSupport>)>;

fn support_cache() -> &'static Mutex<SupportCache> {
    static CACHE: OnceLock<Mutex<SupportCache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn read_mcmeta(path: &Path, is_dir: bool) -> Option<String> {
    if is_dir {
        return fs::read_to_string(path.join("pack.mcmeta")).ok();
    }
    let file = fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name("pack.mcmeta").ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    Some(text)
}

/// Reads a pack's support range, cached by file size and modification time so repeated scans
/// don't reopen unchanged archives.
pub fn read_support(path: &Path) -> Option<PackSupport> {
    let is_dir = path.is_dir();
    let meta = fs::metadata(path).ok()?;
    let key = (meta.len(), meta.modified().ok());
    if !is_dir
        && let Ok(cache) = support_cache().lock()
        && let Some((len, when, support)) = cache.get(path)
        && (*len, *when) == key
    {
        return *support;
    }
    let support = read_mcmeta(path, is_dir).and_then(|text| parse_pack_mcmeta(&text));
    if !is_dir && let Ok(mut cache) = support_cache().lock() {
        cache.insert(path.to_path_buf(), (key.0, key.1, support));
    }
    support
}

/// Lists the packs in an instance root, sorted by name.
pub fn scan_packs(instance_root: &Path) -> Vec<PackInfo> {
    let mut packs: Vec<PackInfo> = fs::read_dir(instance_root.join(PACKS_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = path.is_dir();
            if !is_dir && !name.to_ascii_lowercase().ends_with(".zip") {
                return None;
            }
            Some(PackInfo {
                support: read_support(&path),
                modified: modified(&path),
                name,
                is_dir,
            })
        })
        .collect();
    packs.sort_by_key(|pack| pack.name.to_lowercase());
    packs
}

/// How a pack stands for one instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackStatus {
    Compatible,
    Incompatible,
    Unknown,
    /// The user says it works here, regardless of metadata.
    ForcedAllow,
    /// The user says it doesn't work (or isn't wanted) here.
    ForcedDeny,
}

impl PackStatus {
    /// Whether sync should put the pack into the instance. Unknown metadata isn't enough on
    /// its own; the user can vouch for it with an override.
    pub fn allows_sync(self) -> bool {
        matches!(self, Self::Compatible | Self::ForcedAllow)
    }
}

pub fn pack_status(
    store: &InstanceStore,
    instance_id: &str,
    game_version: &str,
    pack_name: &str,
    support: Option<PackSupport>,
) -> PackStatus {
    match store.pack_override(pack_name, instance_id) {
        Some(PackOverride::Allow) => PackStatus::ForcedAllow,
        Some(PackOverride::Deny) => PackStatus::ForcedDeny,
        None => match compatibility(support, parse_game_version(game_version)) {
            Compat::Compatible => PackStatus::Compatible,
            Compat::Incompatible { .. } => PackStatus::Incompatible,
            Compat::Unknown => PackStatus::Unknown,
        },
    }
}

fn place_pack(source: &Path, target: &Path, is_dir: bool) -> std::io::Result<()> {
    if is_dir {
        return copy_dir_recursive(source, target);
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    // A hard link costs no extra disk space; fall back to a copy across volumes.
    fs::hard_link(source, target).or_else(|_| fs::copy(source, target).map(|_| ()))
}

/// Copies packs into instances where they work.
fn share_packs(instances: &[&SyncInstance], store: &InstanceStore, report: &mut SyncReport) {
    let scans: Vec<Vec<PackInfo>> = instances.iter().map(|i| scan_packs(&i.root)).collect();
    // Newest copy of each pack is the source.
    let mut sources: BTreeMap<&str, (usize, &PackInfo)> = BTreeMap::new();
    for (index, packs) in scans.iter().enumerate() {
        for pack in packs {
            let newer = sources
                .get(pack.name.as_str())
                .is_none_or(|(_, existing)| pack.modified > existing.modified);
            if newer {
                sources.insert(pack.name.as_str(), (index, pack));
            }
        }
    }
    for (index, instance) in instances.iter().enumerate() {
        let present: HashSet<&str> = scans[index].iter().map(|p| p.name.as_str()).collect();
        for (name, (source_index, pack)) in &sources {
            if present.contains(name) {
                continue;
            }
            let status = pack_status(
                store,
                &instance.id,
                &instance.game_version,
                name,
                pack.support,
            );
            if !status.allows_sync() {
                continue;
            }
            let from = instances[*source_index].root.join(PACKS_DIR).join(name);
            let to = instance.root.join(PACKS_DIR).join(name);
            match place_pack(&from, &to, pack.is_dir) {
                Ok(()) => report.packs_shared += 1,
                Err(err) => report
                    .errors
                    .push(tr!("{id}: could not add pack {name}: {err}", id = instance.id, name = name, err = err).into_owned()),
            }
        }
    }
}

const OPTIONS_FILE: &str = "options.txt";
const PACKS_KEY: &str = "resourcePacks";

/// Makes the enabled state of shared packs follow the most recently saved instance.
/// Only `file/<pack>` entries for packs the master has are touched; built-in and mod packs,
/// packs unknown to the master, and pack order elsewhere in the list are left alone.
fn sync_enabled(
    instances: &[&SyncInstance],
    store: &InstanceStore,
    skip: &HashSet<String>,
    report: &mut SyncReport,
) {
    let mut loaded: Vec<(&SyncInstance, OptionsFile, SystemTime)> = Vec::new();
    for instance in instances {
        let path = instance.root.join(OPTIONS_FILE);
        if let (Ok(file), Some(when)) = (OptionsFile::read(&path), modified(&path)) {
            loaded.push((instance, file, when));
        }
    }
    let Some((master, master_file, master_modified)) = loaded
        .iter()
        .max_by_key(|(_, _, when)| *when)
        .map(|(i, f, w)| (*i, f.clone(), *w))
    else {
        return;
    };
    let master_list = value::parse_list(master_file.get(PACKS_KEY).unwrap_or("[]"));
    let master_packs: Vec<PackInfo> = scan_packs(&master.root);

    for (instance, file, when) in &mut loaded {
        if instance.id == master.id || *when >= master_modified || skip.contains(&instance.id) {
            continue;
        }
        let here: HashSet<String> = scan_packs(&instance.root)
            .into_iter()
            .map(|p| p.name)
            .collect();
        let mut list = value::parse_list(file.get(PACKS_KEY).unwrap_or("[]"));
        let original = list.clone();
        for pack in &master_packs {
            let entry = format!("file/{}", pack.name);
            let status = pack_status(
                store,
                &instance.id,
                &instance.game_version,
                &pack.name,
                pack.support,
            );
            if !here.contains(&pack.name) || !status.allows_sync() {
                continue;
            }
            let enabled_in_master = master_list.contains(&entry);
            let enabled_here = list.contains(&entry);
            if enabled_in_master && !enabled_here {
                list.push(entry);
            } else if !enabled_in_master && enabled_here {
                list.retain(|existing| *existing != entry);
            }
        }
        if list == original {
            continue;
        }
        file.set(PACKS_KEY, &value::format_list(&list));
        let path = instance.root.join(OPTIONS_FILE);
        match write_atomic_bytes(&path, file.serialize().as_bytes(), Some(master_modified)) {
            Ok(()) => report.settings_files += 1,
            Err(err) => report.errors.push(format!(
                "{}: failed to write {OPTIONS_FILE}: {err}",
                instance.id
            )),
        }
    }
}

pub(crate) fn sync(
    instances: &[SyncInstance],
    store: &InstanceStore,
    skip: &HashSet<String>,
    report: &mut SyncReport,
) {
    let sync = &store.game_settings_sync;
    if !sync.resource_packs {
        return;
    }
    let participants: Vec<&SyncInstance> = instances
        .iter()
        .filter(|instance| sync.instances.contains(&instance.id))
        .collect();
    if participants.len() < 2 {
        return;
    }
    share_packs(&participants, store, report);
    sync_enabled(&participants, store, skip, report);
}
