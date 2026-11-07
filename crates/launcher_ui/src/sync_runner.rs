//! Runs instance sync (servers, command history, hotbars) with the launcher's settings.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

use config::Config;
use instance_sync::{
    ManageOutcome, ManageRequest, SyncOptions, SyncReport, WorldPaths, WorldSyncOutcome, disable_world_sync, enable_world_sync,
    set_world_members, sync_all,
};
use instances::{InstanceStore, ServerSyncScope, SyncedWorld, instance_root_path};

/// Settings needed to run a sync pass, captured so it can move to a worker thread.
#[derive(Clone, Debug)]
pub struct SyncRunConfig {
    installations_root: PathBuf,
    shared_worlds_root: PathBuf,
    options: SyncOptions,
}

impl SyncRunConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            installations_root: config.minecraft_installations_root_path().to_path_buf(),
            shared_worlds_root: app_paths::synced_worlds_root(),
            options: SyncOptions {
                servers: config.sync_servers_enabled(),
                command_history: config.sync_command_history_enabled(),
                hotbars: config.sync_hotbars_enabled(),
                servers_default_all_instances: config.sync_servers_to_all_instances_by_default(),
            },
        }
    }

    /// Always true: mirrored worlds are refreshed even with every toggle off.
    pub fn is_enabled(&self) -> bool {
        true
    }
}

/// Syncs `store`'s instances now, leaving running instances untouched because the game
/// rewrites these files on exit.
pub fn run_blocking(store: &InstanceStore, run: &SyncRunConfig) -> SyncReport {
    if !run.is_enabled() {
        return SyncReport::default();
    }
    let running = running_instances(store, run);
    let report = sync_all(
        store,
        &run.installations_root,
        &run.shared_worlds_root,
        run.options,
        &running,
    );
    for error in &report.errors {
        tracing::warn!(target: "vertexlauncher/instance_sync", "{error}");
    }
    report
}

fn running_instances(store: &InstanceStore, run: &SyncRunConfig) -> HashSet<String> {
    store
        .instances
        .iter()
        .filter(|instance| {
            installation::is_instance_running(
                instance_root_path(&run.installations_root, instance).as_path(),
            )
        })
        .map(|instance| instance.id.clone())
        .collect()
}

/// A change to which instances share a world.
#[derive(Clone, Debug)]
pub enum WorldSyncRequest {
    Enable {
        source_instance_id: String,
        folder: String,
        members: BTreeSet<String>,
    },
    SetMembers {
        world_id: String,
        members: BTreeSet<String>,
    },
    Disable {
        world_id: String,
    },
}

/// Result of a [`WorldSyncRequest`]: the registry to store, and what happened.
pub struct WorldSyncResult {
    pub synced_worlds: Vec<SyncedWorld>,
    pub outcome: Result<WorldSyncOutcome, String>,
}

pub type WorldSyncReceiver = Arc<Mutex<mpsc::Receiver<WorldSyncResult>>>;

/// Runs a world sync change on a worker thread, since moving or copying a world can take
/// a while. The caller stores `synced_worlds` from the result.
pub fn spawn_world_sync(
    mut store: InstanceStore,
    run: &SyncRunConfig,
    request: WorldSyncRequest,
) -> WorldSyncReceiver {
    let (tx, rx) = mpsc::channel();
    let run = run.clone();
    let _ = std::thread::Builder::new()
        .name("world-sync".to_owned())
        .spawn(move || {
            let running = running_instances(&store, &run);
            let paths = WorldPaths {
                installations_root: &run.installations_root,
                shared_root: &run.shared_worlds_root,
            };
            let outcome = match request {
                WorldSyncRequest::Enable {
                    source_instance_id,
                    folder,
                    members,
                } => enable_world_sync(
                    &mut store,
                    &paths,
                    &source_instance_id,
                    &folder,
                    &members,
                    &running,
                ),
                WorldSyncRequest::SetMembers { world_id, members } => {
                    set_world_members(&mut store, &paths, &world_id, &members, &running)
                }
                WorldSyncRequest::Disable { world_id } => {
                    disable_world_sync(&mut store, &paths, &world_id, &running)
                }
            };
            let _ = tx.send(WorldSyncResult {
                synced_worlds: store.synced_worlds,
                outcome,
            });
        });
    Arc::new(Mutex::new(rx))
}

/// Like [`run_blocking`], reading the saved store from disk. Used by launch paths that
/// don't hold the in-memory store.
pub fn run_from_saved_store_blocking(run: &SyncRunConfig) -> SyncReport {
    if !run.is_enabled() {
        return SyncReport::default();
    }
    match instances::load_store() {
        Ok(store) => run_blocking(&store, run),
        Err(err) => {
            tracing::warn!(
                target: "vertexlauncher/instance_sync",
                error = %err,
                "Skipping instance sync: could not load the instance store."
            );
            SyncReport::default()
        }
    }
}

/// Runs a sync pass on a background thread.
pub fn spawn(store: InstanceStore, run: SyncRunConfig) {
    if !run.is_enabled() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("instance-sync".to_owned())
        .spawn(move || {
            run_blocking(&store, &run);
        });
}

/// Result of a batch of [`ManageRequest`]s: the sync rules to store (a deleted or moved server changes them), and what
/// happened.
pub struct ManageResult {
    pub server_sync_scopes: std::collections::BTreeMap<String, ServerSyncScope>,
    pub outcome: Result<ManageOutcome, String>,
}

pub type ManageReceiver = Arc<Mutex<mpsc::Receiver<ManageResult>>>;

/// Runs `requests` in order on a worker thread (copying a world can take a while), stopping at the first that fails.
pub fn spawn_manage(mut store: InstanceStore, run: &SyncRunConfig, requests: Vec<ManageRequest>) -> ManageReceiver {
    let (tx, rx) = mpsc::channel();
    let run = run.clone();
    let _ = std::thread::Builder::new()
        .name("entry-manage".to_owned())
        .spawn(move || {
            let running = running_instances(&store, &run);
            let mut messages = Vec::new();
            let mut outcome = Ok(ManageOutcome::default());
            for request in requests {
                match instance_sync::manage(
                    &mut store,
                    &run.installations_root,
                    &running,
                    run.options.servers_default_all_instances,
                    request,
                ) {
                    Ok(done) => messages.push(done.message),
                    Err(err) => {
                        outcome = Err(err);
                        break;
                    }
                }
            }
            if outcome.is_ok() {
                messages.dedup();
                outcome = Ok(ManageOutcome { message: messages.join(" ") });
            }
            let _ = tx.send(ManageResult { server_sync_scopes: store.server_sync_scopes, outcome });
        });
    Arc::new(Mutex::new(rx))
}
