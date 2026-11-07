//! Keeps multiplayer servers, command history and creative hotbars in step across
//! instances by reconciling the game files in each instance root.

mod command_history;
mod fs_util;
mod hotbars;
mod manage;
mod mod_configs;
pub mod nbt;
mod resource_packs;
mod servers;
mod settings;
mod versions;
mod worlds;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use instances::{InstanceStore, instance_root_path};

pub use resource_packs::{PackInfo, PackStatus, pack_status, read_support, scan_packs};
pub use versions::compare_game_versions;

/// `(id, display name)` of each mod config that can be synced.
pub fn available_mod_configs() -> Vec<(&'static str, &'static str)> {
    mod_configs::MOD_CONFIGS
        .iter()
        .map(|config| (config.id, config.name))
        .collect()
}
pub use manage::{ManageOutcome, ManageRequest, manage};
pub use worlds::{
    WorldPaths, WorldSyncOutcome, disable_world_sync, enable_world_sync, set_world_members,
};

/// What to sync, mirroring the launcher settings.
#[derive(Clone, Copy, Debug, Default)]
pub struct SyncOptions {
    pub servers: bool,
    pub command_history: bool,
    pub hotbars: bool,
    /// Servers with no explicit scope go to every instance when true.
    pub servers_default_all_instances: bool,
}

impl SyncOptions {
    pub fn any_enabled(&self) -> bool {
        self.servers || self.command_history || self.hotbars
    }
}

/// What a sync pass changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub servers_added: usize,
    pub command_history_files: usize,
    pub hotbars_copied: usize,
    pub worlds_updated: usize,
    pub settings_files: usize,
    pub mod_config_files: usize,
    pub packs_shared: usize,
    pub errors: Vec<String>,
}

impl SyncReport {
    pub fn changed_anything(&self) -> bool {
        self.servers_added
            + self.command_history_files
            + self.hotbars_copied
            + self.worlds_updated
            + self.settings_files
            + self.mod_config_files
            + self.packs_shared
            > 0
    }
}

pub(crate) struct SyncInstance {
    pub(crate) id: String,
    pub(crate) root: PathBuf,
    pub(crate) game_version: String,
}

/// Reconciles all instances. Instances in `skip` (for example ones whose game is running,
/// since the game rewrites these files on exit) are read but never written.
pub fn sync_all(
    store: &InstanceStore,
    installations_root: &Path,
    shared_worlds_root: &Path,
    options: SyncOptions,
    skip: &HashSet<String>,
) -> SyncReport {
    let mut report = SyncReport::default();
    // Worlds shared by mirroring are refreshed regardless of the other toggles: the user
    // opted into them per world.
    worlds::mirror_pass(
        store,
        installations_root,
        shared_worlds_root,
        skip,
        &mut report,
    );
    // Settings sync is configured per instance in the store rather than through options.
    let settings_instances = instances_for_settings(store, installations_root);
    settings::sync(&settings_instances, store, skip, &mut report);
    mod_configs::sync(&settings_instances, store, skip, &mut report);
    resource_packs::sync(&settings_instances, store, skip, &mut report);
    if !options.any_enabled() {
        return report;
    }
    let instances: Vec<SyncInstance> = store
        .instances
        .iter()
        .map(|instance| SyncInstance {
            id: instance.id.clone(),
            root: instance_root_path(installations_root, instance),
            game_version: instance.game_version.clone(),
        })
        .filter(|instance| instance.root.is_dir())
        .collect();

    if options.servers {
        servers::sync(
            &instances,
            store,
            options.servers_default_all_instances,
            skip,
            &mut report,
        );
    }
    if options.command_history {
        command_history::sync(&instances, skip, &mut report);
    }
    if options.hotbars {
        hotbars::sync(&instances, skip, &mut report);
    }
    tracing::info!(
        target: "vertexlauncher/instance_sync",
        servers_added = report.servers_added,
        command_history_files = report.command_history_files,
        hotbars_copied = report.hotbars_copied,
        errors = report.errors.len(),
        "Instance sync pass finished."
    );
    report
}

fn instances_for_settings(store: &InstanceStore, installations_root: &Path) -> Vec<SyncInstance> {
    store
        .instances
        .iter()
        .map(|instance| SyncInstance {
            id: instance.id.clone(),
            root: instance_root_path(installations_root, instance),
            game_version: instance.game_version.clone(),
        })
        .filter(|instance| instance.root.is_dir())
        .collect()
}

#[cfg(test)]
mod tests;
