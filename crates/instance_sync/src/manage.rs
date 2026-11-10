//! Managing single worlds and servers in an instance's game files: renaming, deleting, and moving or copying to another
//! instance. Sharing one between several instances is `worlds` and `servers`; this keeps those in step when an entry is
//! changed, so a deleted server is not put back by the next sync.

use vertex_i18n::tr;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use instances::{InstanceStore, ServerSyncScope, instance_root_path, normalize_server_key};

use crate::fs_util::{copy_dir_recursive, is_link, write_atomic};
use crate::nbt::{self, Compound, NbtFile, Tag};
use crate::servers::{SERVERS_FILE, entry_key, push_entry, server_list};

/// A change to one world or server in one or more instances.
#[derive(Clone, Debug)]
pub enum ManageRequest {
    /// Sets the name the game lists a world under (`level.dat`); the folder keeps its name.
    RenameWorld { instance_ids: BTreeSet<String>, folder: String, new_name: String },
    DeleteWorld { instance_ids: BTreeSet<String>, folder: String },
    /// Copies a world's folder into another instance, and removes the original when `remove_source` is set.
    TransferWorld { source_id: String, folder: String, dest_id: String, remove_source: bool },
    /// Changes a server's name and/or address in the given instances.
    EditServer { instance_ids: BTreeSet<String>, address: String, new_name: String, new_address: String },
    DeleteServer { instance_ids: BTreeSet<String>, address: String },
    /// Copies a server into another instance's list, and removes the original when `remove_source` is set.
    TransferServer { source_id: String, address: String, dest_id: String, remove_source: bool },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManageOutcome {
    /// What happened, for the user.
    pub message: String,
}

fn label(store: &InstanceStore, id: &str) -> String {
    store.find(id).map_or_else(|| id.to_owned(), |instance| instance.name.clone())
}

fn ensure_stopped<'a>(
    store: &InstanceStore,
    ids: impl IntoIterator<Item = &'a String>,
    running: &HashSet<String>,
) -> Result<(), String> {
    for id in ids {
        if running.contains(id) {
            return Err(tr!("Close {instance} first; the game rewrites these files when it exits.", instance = label(store, id)).into_owned());
        }
    }
    Ok(())
}

fn root_of(store: &InstanceStore, installations_root: &Path, id: &str) -> Result<PathBuf, String> {
    store
        .find(id)
        .map(|instance| instance_root_path(installations_root, instance))
        .ok_or_else(|| tr!("Instance not found.").into_owned())
}

/// A folder name that stays inside `saves`.
fn checked_folder(folder: &str) -> Result<&str, String> {
    if folder.is_empty() || folder == "." || folder == ".." || folder.contains(['/', '\\']) {
        return Err(vertex_i18n::tr!("{folder} is not a world folder.", folder = format!("{folder:?}")).into_owned());
    }
    Ok(folder)
}

fn world_path(store: &InstanceStore, installations_root: &Path, id: &str, folder: &str) -> Result<PathBuf, String> {
    Ok(root_of(store, installations_root, id)?.join("saves").join(checked_folder(folder)?))
}

fn refuse_synced(store: &InstanceStore, id: &str, folder: &str) -> Result<(), String> {
    if store.synced_world_for(id, folder).is_some() {
        return Err(tr!("{folder} is synced between instances. Use \"Sync world…\" to take {instance} out of the group first.", folder = folder, instance = label(store, id)).into_owned());
    }
    Ok(())
}

/// `base`, or `base-2`, `base-3`… when that name is taken in `dir`.
fn free_name(dir: &Path, base: &str) -> String {
    if !dir.join(base).exists() {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|name| !dir.join(name).exists())
        .unwrap_or_else(|| base.to_owned())
}

fn rename_world(path: &Path, new_name: &str) -> Result<(), String> {
    let level = path.join("level.dat");
    let bytes = fs::read(&level).map_err(|err| tr!("could not read level.dat: {err}", err = err).into_owned())?;
    let mut file = nbt::parse(&bytes)?;
    let Some(Tag::Compound(data)) = nbt::get_mut(&mut file.root, "Data") else {
        return Err(tr!("level.dat has no Data section.").into_owned());
    };
    let name = Tag::String(new_name.as_bytes().to_vec());
    match nbt::get_mut(data, "LevelName") {
        Some(tag) => *tag = name,
        None => data.push((tr!("LevelName").into_owned(), name)),
    }
    let _ = fs::copy(&level, path.join("level.dat_vertex_backup"));
    write_atomic(&level, &nbt::write_gzip(&file)?, None).map_err(|err| tr!("could not write level.dat: {err}", err = err).into_owned())
}

fn load_servers(root: &Path) -> Result<NbtFile, String> {
    let path = root.join(SERVERS_FILE);
    if !path.is_file() {
        return Ok(NbtFile { name: String::new(), root: Vec::new() });
    }
    nbt::parse(&fs::read(&path).map_err(|err| err.to_string())?)
}

fn save_servers(root: &Path, file: &NbtFile) -> Result<(), String> {
    let path = root.join(SERVERS_FILE);
    if path.is_file() {
        let _ = fs::copy(&path, root.join("servers.dat_vertex_backup"));
    }
    write_atomic(&path, &nbt::write(file), None).map_err(|err| format!("could not write {SERVERS_FILE}: {err}"))
}

fn find_server<'a>(file: &'a mut NbtFile, key: &str) -> Option<&'a mut Compound> {
    let Some(Tag::List(list)) = nbt::get_mut(&mut file.root, "servers") else {
        return None;
    };
    list.items.iter_mut().find_map(|entry| match entry {
        Tag::Compound(compound) if entry_key_of(compound).as_deref() == Some(key) => Some(compound),
        _ => None,
    })
}

fn entry_key_of(compound: &Compound) -> Option<String> {
    entry_key(&Tag::Compound(compound.clone()))
}

fn set_string(compound: &mut Compound, key: &str, value: &str) {
    let tag = Tag::String(value.as_bytes().to_vec());
    match nbt::get_mut(compound, key) {
        Some(existing) => *existing = tag,
        None => compound.push((key.to_owned(), tag)),
    }
}

fn remove_server(file: &mut NbtFile, key: &str) -> bool {
    let Some(Tag::List(list)) = nbt::get_mut(&mut file.root, "servers") else {
        return false;
    };
    let before = list.items.len();
    list.items.retain(|entry| entry_key(entry).as_deref() != Some(key));
    list.items.len() != before
}

/// Keeps `address`'s sync scope from putting a server back into `removed` instances (where the next sync would add it
/// again) and, for `added`, from leaving a deliberate copy outside its own scope.
fn adjust_scope(
    store: &mut InstanceStore,
    address: &str,
    removed: &BTreeSet<String>,
    added: Option<&str>,
    default_is_all: bool,
) {
    let scope = store.server_sync_scope(address);
    let includes = |id: &str| scope.includes(id, default_is_all);
    let touches_removed = removed.iter().any(|id| includes(id));
    if !touches_removed && !matches!(scope, ServerSyncScope::Selected(_)) {
        return;
    }
    let mut keep: BTreeSet<String> = store
        .instances
        .iter()
        .filter(|instance| includes(&instance.id) && !removed.contains(&instance.id))
        .map(|instance| instance.id.clone())
        .collect();
    if let Some(added) = added {
        keep.insert(added.to_owned());
    }
    store.set_server_sync_scope(address, ServerSyncScope::Selected(keep));
}

/// Runs `request`. Blocking: copying a world can take a while, so call it from a worker. `store` is updated where the
/// request changes how a server is synced; the caller keeps that part of it. Instances in `running` are refused.
pub fn manage(
    store: &mut InstanceStore,
    installations_root: &Path,
    running: &HashSet<String>,
    servers_default_all: bool,
    request: ManageRequest,
) -> Result<ManageOutcome, String> {
    match request {
        ManageRequest::RenameWorld { instance_ids, folder, new_name } => {
            let new_name = new_name.trim();
            if new_name.is_empty() {
                return Err(tr!("A world needs a name.").into_owned());
            }
            ensure_stopped(store, &instance_ids, running)?;
            for id in &instance_ids {
                rename_world(&world_path(store, installations_root, id, &folder)?, new_name)
                    .map_err(|err| tr!("{instance}: {err}", instance = label(store, id), err = err).into_owned())?;
            }
            Ok(ManageOutcome { message: tr!("Renamed to {new_name}.", new_name = new_name).into_owned() })
        }
        ManageRequest::DeleteWorld { instance_ids, folder } => {
            ensure_stopped(store, &instance_ids, running)?;
            for id in &instance_ids {
                refuse_synced(store, id, &folder)?;
                let path = world_path(store, installations_root, id, &folder)?;
                if is_link(&path) {
                    return Err(tr!("{folder} is a link; remove it from the sync group instead.", folder = folder).into_owned());
                }
            }
            for id in &instance_ids {
                let path = world_path(store, installations_root, id, &folder)?;
                fs::remove_dir_all(&path).map_err(|err| tr!("{instance}: could not delete {folder}: {err}", instance = label(store, id), folder = folder, err = err).into_owned())?;
            }
            Ok(ManageOutcome { message: tr!("Deleted.").into_owned() })
        }
        ManageRequest::TransferWorld { source_id, folder, dest_id, remove_source } => {
            if source_id == dest_id {
                return Err(tr!("Pick a different instance.").into_owned());
            }
            ensure_stopped(store, [&source_id, &dest_id], running)?;
            refuse_synced(store, &source_id, &folder)?;
            let source = world_path(store, installations_root, &source_id, &folder)?;
            if is_link(&source) || !source.is_dir() {
                return Err(tr!("{folder} is not a regular world folder.", folder = folder).into_owned());
            }
            let saves = root_of(store, installations_root, &dest_id)?.join("saves");
            let name = free_name(&saves, &folder);
            let dest = saves.join(&name);
            if let Err(err) = copy_dir_recursive(&source, &dest) {
                let _ = fs::remove_dir_all(&dest);
                return Err(tr!("Could not copy the world: {err}", err = err).into_owned());
            }
            if remove_source {
                fs::remove_dir_all(&source)
                    .map_err(|err| tr!("Copied to {instance}, but could not remove the original: {err}", instance = label(store, &dest_id), err = err).into_owned())?;
            }
            Ok(ManageOutcome {
                message: if remove_source {
                    tr!("Moved {name} to {destination}.", name = name, destination = label(store, &dest_id)).into_owned()
                } else {
                    tr!("Copied {name} to {destination}.", name = name, destination = label(store, &dest_id)).into_owned()
                },
            })
        }
        ManageRequest::EditServer { instance_ids, address, new_name, new_address } => {
            let (new_name, new_address) = (new_name.trim(), new_address.trim());
            if new_address.is_empty() {
                return Err(tr!("A server needs an address.").into_owned());
            }
            ensure_stopped(store, &instance_ids, running)?;
            let (old_key, new_key) = (normalize_server_key(&address), normalize_server_key(new_address));
            for id in &instance_ids {
                let root = root_of(store, installations_root, id)?;
                let mut file = load_servers(&root).map_err(|err| tr!("{instance}: {err}", instance = label(store, id), err = err).into_owned())?;
                if old_key != new_key && server_list(&file).iter().any(|entry| entry_key(entry).as_deref() == Some(new_key.as_str())) {
                    return Err(tr!("{instance} already has a server at {new_address}.", instance = label(store, id), new_address = new_address).into_owned());
                }
                let Some(entry) = find_server(&mut file, &old_key) else { continue };
                set_string(entry, "name", new_name);
                set_string(entry, "ip", new_address);
                save_servers(&root, &file).map_err(|err| tr!("{instance}: {err}", instance = label(store, id), err = err).into_owned())?;
            }
            if old_key != new_key {
                // The sync rule follows the server to its new address.
                let scope = store.server_sync_scope(&address);
                store.set_server_sync_scope(&address, ServerSyncScope::FollowDefault);
                store.set_server_sync_scope(new_address, scope);
            }
            Ok(ManageOutcome { message: tr!("Saved.").into_owned() })
        }
        ManageRequest::DeleteServer { instance_ids, address } => {
            ensure_stopped(store, &instance_ids, running)?;
            let key = normalize_server_key(&address);
            for id in &instance_ids {
                let root = root_of(store, installations_root, id)?;
                let mut file = load_servers(&root).map_err(|err| tr!("{instance}: {err}", instance = label(store, id), err = err).into_owned())?;
                if remove_server(&mut file, &key) {
                    save_servers(&root, &file).map_err(|err| tr!("{instance}: {err}", instance = label(store, id), err = err).into_owned())?;
                }
            }
            adjust_scope(store, &address, &instance_ids, None, servers_default_all);
            Ok(ManageOutcome { message: tr!("Deleted.").into_owned() })
        }
        ManageRequest::TransferServer { source_id, address, dest_id, remove_source } => {
            if source_id == dest_id {
                return Err(tr!("Pick a different instance.").into_owned());
            }
            ensure_stopped(store, [&source_id, &dest_id], running)?;
            let key = normalize_server_key(&address);
            let source_root = root_of(store, installations_root, &source_id)?;
            let mut source_file = load_servers(&source_root)?;
            let entry = find_server(&mut source_file, &key).map(|entry| entry.clone()).ok_or_else(|| vertex_i18n::tr!("The server is no longer in that instance.").into_owned())?;
            let dest_root = root_of(store, installations_root, &dest_id)?;
            let mut dest_file = load_servers(&dest_root)?;
            if server_list(&dest_file).iter().any(|existing| entry_key(existing).as_deref() == Some(key.as_str())) {
                return Err(tr!("{instance} already has this server.", instance = label(store, &dest_id)).into_owned());
            }
            push_entry(&mut dest_file, entry);
            save_servers(&dest_root, &dest_file).map_err(|err| tr!("{instance}: {err}", instance = label(store, &dest_id), err = err).into_owned())?;
            let removed: BTreeSet<String> = if remove_source { BTreeSet::from([source_id.clone()]) } else { BTreeSet::new() };
            if remove_source {
                remove_server(&mut source_file, &key);
                save_servers(&source_root, &source_file).map_err(|err| tr!("{instance}: {err}", instance = label(store, &source_id), err = err).into_owned())?;
            }
            adjust_scope(store, &address, &removed, Some(&dest_id), servers_default_all);
            Ok(ManageOutcome {
                message: if remove_source {
                    tr!("Moved to {destination}.", destination = label(store, &dest_id)).into_owned()
                } else {
                    tr!("Copied to {destination}.", destination = label(store, &dest_id)).into_owned()
                },
            })
        }
    }
}
