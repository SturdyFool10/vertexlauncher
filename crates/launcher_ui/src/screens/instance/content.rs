use vertex_i18n::tr;
use super::content_lookup_result::ContentLookupResultEntry;
use super::*;
use content_resolver::detect_installed_content_kind;
use std::collections::{BTreeMap, HashSet};

#[path = "content_actions.rs"]
mod content_actions;
#[path = "content_api.rs"]
pub mod api;
#[path = "content_hash_cache.rs"]
mod content_hash_cache;
#[path = "content_metadata_lookup.rs"]
mod content_metadata_lookup;
#[path = "content_updates.rs"]
mod content_updates;
#[path = "installed_entry_display.rs"]
mod installed_entry_display;

use self::content_actions::poll_content_apply_results;
use self::content_actions::refresh_installed_content_state;
use self::content_actions::request_bulk_content_update;
use self::content_actions::request_content_delete;
use self::content_actions::request_content_toggle;
use self::content_actions::request_content_update;
use self::content_actions::request_local_content_import;
use self::content_actions::request_vtmpatch_apply;
use self::content_hash_cache::*;
pub(super) use self::content_metadata_lookup::poll_content_lookup_results;
use self::content_metadata_lookup::*;
use self::content_updates::{
    bulk_update_button_label, bulk_update_button_tooltip,
    prefetch_bulk_update_metadata, resolve_installed_content_metadata_batch,
    tab_has_known_available_update,
};
use self::installed_entry_display::InstalledEntryDisplay;

const CONTENT_HASH_CACHE_FLUSH_DEBOUNCE: Duration = Duration::from_millis(750);
const CONTENT_LOOKUP_BATCH_SIZE: usize = 24;
const CONTENT_UPDATE_PREFETCH_BATCH_SIZE: usize = 4;
const CONTENT_UPDATE_LOG_TARGET: &str = "vertexlauncher/content_update";








#[derive(Clone, Debug)]
pub(super) struct ContentApplyResult {
    pub(super) kind: InstalledContentKind,
    pub(super) focus_lookup_keys: Vec<String>,
    pub(super) refresh_all_content: bool,
    pub(super) status_message: Result<String, String>,
}



















pub(super) fn filter_installed_content_indices(
    state: &InstanceScreenState,
    files: &[InstalledContentFile],
) -> Option<Vec<usize>> {
    let query = state
        .installed_content_search_query
        .trim()
        .to_ascii_lowercase();
    if query.is_empty() && !state.installed_content_updatable_only {
        return None;
    }

    Some(
        files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                !state.installed_content_updatable_only || installed_content_file_has_update(state, file)
            })
            .filter(|(_, file)| query.is_empty() || installed_content_file_matches_search(state, file, query.as_str()))
            .map(|(index, _)| index)
            .collect(),
    )
}

fn installed_content_file_has_update(state: &InstanceScreenState, file: &InstalledContentFile) -> bool {
    state
        .content_metadata_cache
        .get(&file.lookup_key)
        .and_then(|resolution| resolution.as_ref())
        .is_some_and(|resolution| resolution.update.is_some())
}

fn installed_content_file_matches_search(
    state: &InstanceScreenState,
    file: &InstalledContentFile,
    query: &str,
) -> bool {
    let mut haystack = vec![
        file.file_name.as_str(),
        file.lookup_query.as_str(),
        file.lookup_key.as_str(),
    ];
    if let Some(value) = file.fallback_lookup_query.as_deref() {
        haystack.push(value);
    }
    if let Some(value) = file.fallback_lookup_key.as_deref() {
        haystack.push(value);
    }
    if let Some(identity) = file.managed_identity.as_ref() {
        haystack.push(identity.name.as_str());
        haystack.push(identity.selected_version_id.as_str());
    }
    if let Some(metadata) = state
        .content_metadata_cache
        .get(&file.lookup_key)
        .and_then(|metadata| metadata.as_ref())
    {
        haystack.push(metadata.entry.name.as_str());
        haystack.push(metadata.entry.summary.as_str());
        haystack.push(metadata.entry.source.label());
        if let Some(version) = metadata.installed_version_label.as_deref() {
            haystack.push(version);
        }
        if let Some(warning) = metadata.warning_message.as_deref() {
            haystack.push(warning);
        }
        if let Some(update) = metadata.update.as_ref() {
            haystack.push(update.latest_version_label.as_str());
        }
    }

    haystack
        .iter()
        .any(|value| contains_ignore_ascii_case(value, query))
}

/// Equivalent to `value.to_ascii_lowercase().contains(query)` for an already lowercased
/// `query`, without allocating.
fn contains_ignore_ascii_case(value: &str, query: &str) -> bool {
    let query = query.as_bytes();
    if query.is_empty() {
        return true;
    }
    value
        .as_bytes()
        .windows(query.len())
        .any(|window| window.eq_ignore_ascii_case(query))
}

pub(super) fn installed_content_count_label(matching: usize, total: usize, filtered: bool) -> String {
    if !filtered || matching == total {
        tr!("{total} installed", total = total).into_owned()
    } else {
        tr!("{matching} matching / {total} installed", matching = matching, total = total).into_owned()
    }
}

pub(super) fn poll_installed_content_scan_results(state: &mut InstanceScreenState) {
    let drained = state.installed_content_cache.scan_results.drain();
    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/instance_content", "scan_results receiver was poisoned; channel reset.");
    }
    for result in drained.items {
        if result.generation != state.installed_content_cache.scan_generation {
            continue;
        }
        state
            .installed_content_cache
            .scans_in_flight
            .remove(&result.kind);
        state.installed_content_cache.managed_identities = Some(result.managed_identities);
        state
            .installed_content_cache
            .files_by_tab
            .insert(result.kind, result.files);
        state.installed_content_cache.stale_tabs.remove(&result.kind);
    }
}

fn request_installed_content_scan(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    tab: InstalledContentKind,
) {
    if state.installed_content_cache.scans_in_flight.contains(&tab) {
        return;
    }
    let tx = state.installed_content_cache.scan_results.sender();

    let generation = state.installed_content_cache.scan_generation;
    let instance_root = instance_root.to_path_buf();
    state.installed_content_cache.scans_in_flight.insert(tab);
    let _ = tokio_runtime::spawn_detached(async move {
        let join = tokio_runtime::spawn_blocking(move || {
            let managed_identities = load_managed_content_identities(instance_root.as_path());
            let files: Arc<[InstalledContentFile]> =
                InstalledContentResolver::scan_installed_content_files(
                    instance_root.as_path(),
                    tab,
                    &managed_identities,
                )
                .into();
            InstalledContentScanResult {
                generation,
                kind: tab,
                managed_identities,
                files,
            }
        });
        match join.await {
            Ok(result) => {
                let _ = tx.send(result);
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/instance_content",
                    kind = %tab.folder_name(),
                    error = %err,
                    "Installed-content scan worker panicked."
                );
            }
        }
    });
}

pub(super) fn installed_content_files_for_tab(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    tab: InstalledContentKind,
) -> Arc<[InstalledContentFile]> {
    if let Some(files) = state.installed_content_cache.files_by_tab.get(&tab).cloned() {
        if state.installed_content_cache.stale_tabs.contains(&tab) {
            request_installed_content_scan(state, instance_root, tab);
        }
        return files;
    }

    request_installed_content_scan(state, instance_root, tab);
    Arc::from([])
}








