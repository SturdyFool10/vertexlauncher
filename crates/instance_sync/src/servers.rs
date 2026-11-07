use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::time::SystemTime;

use instances::{InstanceStore, normalize_server_key};

use crate::fs_util::{modified, write_atomic};
use crate::nbt::{self, Compound, NbtFile, Tag, TagList};
use crate::{SyncInstance, SyncReport};

pub(crate) const SERVERS_FILE: &str = "servers.dat";
const NBT_COMPOUND_ID: u8 = 10;

struct LoadedServers {
    file: NbtFile,
    modified: Option<SystemTime>,
    dirty: bool,
}

pub(crate) fn server_list(file: &NbtFile) -> &[Tag] {
    match nbt::get(&file.root, "servers") {
        Some(Tag::List(list)) => &list.items,
        _ => &[],
    }
}

pub(crate) fn entry_key(entry: &Tag) -> Option<String> {
    let Tag::Compound(compound) = entry else {
        return None;
    };
    let key = normalize_server_key(nbt::get(compound, "ip")?.as_str()?.as_str());
    (!key.is_empty()).then_some(key)
}

pub(crate) fn push_entry(file: &mut NbtFile, entry: Compound) {
    if nbt::get(&file.root, "servers").is_none() {
        file.root.push((
            "servers".to_owned(),
            Tag::List(TagList {
                element_id: NBT_COMPOUND_ID,
                items: Vec::new(),
            }),
        ));
    }
    if let Some(Tag::List(list)) = nbt::get_mut(&mut file.root, "servers") {
        list.element_id = NBT_COMPOUND_ID;
        list.items.push(Tag::Compound(entry));
    }
}

/// Copies each server into every instance its scope includes. Existing entries are never
/// changed or removed, so nothing a user set up per instance is lost.
pub(crate) fn sync(
    instances: &[SyncInstance],
    store: &InstanceStore,
    default_is_all: bool,
    skip: &HashSet<String>,
    report: &mut SyncReport,
) {
    let mut loaded: Vec<Option<LoadedServers>> = Vec::with_capacity(instances.len());
    for instance in instances {
        let path = instance.root.join(SERVERS_FILE);
        if !path.is_file() {
            loaded.push(Some(LoadedServers {
                file: NbtFile {
                    name: String::new(),
                    root: Vec::new(),
                },
                modified: None,
                dirty: false,
            }));
            continue;
        }
        let parsed = fs::read(&path)
            .map_err(|err| err.to_string())
            .and_then(|bytes| nbt::parse(&bytes));
        match parsed {
            Ok(file) => loaded.push(Some(LoadedServers {
                file,
                modified: modified(&path),
                dirty: false,
            })),
            Err(err) => {
                report
                    .errors
                    .push(format!("{}: unreadable {SERVERS_FILE}: {err}", instance.id));
                loaded.push(None);
            }
        }
    }

    // Newest file wins as the source of each server's entry (fresher icon and name).
    let mut sources: BTreeMap<String, (Option<SystemTime>, Compound)> = BTreeMap::new();
    for state in loaded.iter().flatten() {
        for entry in server_list(&state.file) {
            let (Some(key), Tag::Compound(compound)) = (entry_key(entry), entry) else {
                continue;
            };
            let newer = sources
                .get(&key)
                .is_none_or(|(existing, _)| state.modified > *existing);
            if newer {
                sources.insert(key, (state.modified, compound.clone()));
            }
        }
    }

    for (instance, state) in instances.iter().zip(loaded.iter_mut()) {
        let Some(state) = state else { continue };
        if skip.contains(&instance.id) {
            continue;
        }
        let present: HashSet<String> = server_list(&state.file)
            .iter()
            .filter_map(entry_key)
            .collect();
        for (key, (_, entry)) in &sources {
            if present.contains(key) {
                continue;
            }
            let address = nbt::get(entry, "ip")
                .and_then(Tag::as_str)
                .unwrap_or_default();
            if store
                .server_sync_scope(&address)
                .includes(&instance.id, default_is_all)
            {
                push_entry(&mut state.file, entry.clone());
                state.dirty = true;
                report.servers_added += 1;
            }
        }
    }

    for (instance, state) in instances.iter().zip(loaded) {
        let Some(state) = state.filter(|state| state.dirty) else {
            continue;
        };
        let path = instance.root.join(SERVERS_FILE);
        if path.is_file() {
            // The game keeps a `_old` copy too; ours is a one-run safety net.
            let _ = fs::copy(&path, instance.root.join("servers.dat_vertex_backup"));
        }
        if let Err(err) = write_atomic(&path, &nbt::write(&state.file), None) {
            report.errors.push(format!(
                "{}: failed to write {SERVERS_FILE}: {err}",
                instance.id
            ));
        }
    }
}
