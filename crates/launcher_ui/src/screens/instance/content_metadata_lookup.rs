use super::*;

pub(super) fn should_request_content_metadata_lookup(
    state: &mut InstanceScreenState,
    lookup_key: &str,
    force_refresh: bool,
) -> bool {
    let now = Instant::now();
    if lookup_key.trim().is_empty() || state.content_lookup_in_flight.contains(lookup_key) {
        return false;
    }
    if !force_refresh {
        if state
            .content_metadata_cache
            .get(lookup_key)
            .is_some_and(|resolution| resolution.is_some())
        {
            return false;
        }
        if let Some(retry_at) = state.content_lookup_retry_after_by_key.get(lookup_key)
            && *retry_at > now
        {
            return false;
        }
    }
    true
}

pub(super) fn mark_content_metadata_lookup_requested(
    state: &mut InstanceScreenState,
    lookup_key: &str,
) -> u64 {
    state.content_lookup_request_serial = state.content_lookup_request_serial.saturating_add(1);
    let request_serial = state.content_lookup_request_serial;
    state.content_lookup_in_flight.insert(lookup_key.to_owned());
    state.content_lookup_retry_after_by_key.remove(lookup_key);
    state
        .content_lookup_latest_serial_by_key
        .insert(lookup_key.to_owned(), request_serial);
    request_serial
}

#[allow(clippy::too_many_arguments)]
pub(super) fn request_content_metadata_lookup_batch(
    state: &mut InstanceScreenState,
    files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader: &str,
    force_refresh: bool,
    max_batch_size: usize,
) -> usize {
    if files.is_empty() || max_batch_size == 0 || state.content_hash_cache.is_none() {
        return 0;
    }

    let tx = state.content_lookup_results.sender();

    let mut work_items = Vec::new();
    for file in files {
        if work_items.len() >= max_batch_size {
            break;
        }
        if !should_request_content_metadata_lookup(state, file.lookup_key.as_str(), force_refresh) {
            continue;
        }
        let request_serial =
            mark_content_metadata_lookup_requested(state, file.lookup_key.as_str());
        work_items.push((request_serial, file.clone()));
    }
    if work_items.is_empty() {
        return 0;
    }
    let scheduled_count = work_items.len();

    let hash_cache = state.content_hash_cache.clone().unwrap_or_default();
    let game_version = game_version.trim().to_owned();
    let loader = loader.trim().to_owned();
    let _ = tokio_runtime::spawn_detached(async move {
        let join = tokio_runtime::spawn_blocking({
            let game_version = game_version.clone();
            let loader = loader.clone();
            move || {
                resolve_installed_content_lookup_batch(
                    work_items.as_slice(),
                    kind,
                    game_version.as_str(),
                    loader.as_str(),
                    hash_cache,
                )
            }
        });
        let result = match join.await {
            Ok(r) => r,
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/instance_content",
                    kind = %kind.folder_name(),
                    error = %err,
                    "Content metadata lookup worker panicked."
                );
                return;
            }
        };
        if let Err(_) = tx.send(result) {
            // The result channel is closed when the instance screen state is
            // refreshed (e.g. after a patch apply).  In-flight lookup tasks
            // that complete after the reset simply drop their results — this
            // is harmless and expected, so we log at debug rather than error.
            tracing::debug!(
                target: "vertexlauncher/instance_content",
                kind = %kind.folder_name(),
                scheduled_count,
                game_version = %game_version,
                loader = %loader,
                "installed-content metadata lookup completed after state reset; result discarded"
            );
        }
    });

    scheduled_count
}

pub(in super::super) fn poll_content_lookup_results(state: &mut InstanceScreenState) {
    let drained = state.content_lookup_results.drain();
    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/instance_content", "content_lookup_results receiver was poisoned; channel reset.");
    }
    for result in drained.items {
        let cache = state
            .content_hash_cache
            .get_or_insert_with(InstalledContentHashCache::default);
        if cache.apply_updates(result.hash_cache_updates) {
            state.content_hash_cache_serial = state.content_hash_cache_serial.saturating_add(1);
            state.content_hash_cache_dirty = true;
            state.content_hash_cache_dirty_since = Some(Instant::now());
        }

        for result in result.results {
            let is_latest = state
                .content_lookup_latest_serial_by_key
                .get(result.lookup_key.as_str())
                .copied()
                == Some(result.request_serial);
            if !is_latest {
                continue;
            }
            state
                .content_lookup_in_flight
                .remove(result.lookup_key.as_str());
            if let Some(resolution) = result.resolution {
                state
                    .content_lookup_retry_after_by_key
                    .remove(result.lookup_key.as_str());
                state
                    .content_lookup_failure_count_by_key
                    .remove(result.lookup_key.as_str());
                state
                    .content_metadata_cache
                    .insert(result.lookup_key, Some(resolution));
                continue;
            }

            let failure_count = state
                .content_lookup_failure_count_by_key
                .entry(result.lookup_key.clone())
                .and_modify(|count| *count = count.saturating_add(1))
                .or_insert(1);
            state.content_lookup_retry_after_by_key.insert(
                result.lookup_key.clone(),
                Instant::now() + content_lookup_retry_delay(*failure_count),
            );
            state
                .content_metadata_cache
                .entry(result.lookup_key)
                .or_insert(None);
        }
    }
}

pub(super) fn resolve_installed_content_lookup_batch(
    work_items: &[(u64, InstalledContentFile)],
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    mut hash_cache: InstalledContentHashCache,
) -> ContentLookupResult {
    let installed_files = work_items
        .iter()
        .map(|(_, file)| file.clone())
        .collect::<Vec<_>>();
    let original_hash_cache = hash_cache.clone();
    let resolved_by_lookup_key = resolve_installed_content_metadata_batch(
        installed_files.as_slice(),
        kind,
        game_version,
        loader_label,
        &mut hash_cache,
    )
    .into_iter()
    .map(|(file, resolution)| (file.lookup_key, resolution))
    .collect::<std::collections::HashMap<_, _>>();

    ContentLookupResult {
        results: work_items
            .iter()
            .map(|(request_serial, file)| ContentLookupResultEntry {
                request_serial: *request_serial,
                lookup_key: file.lookup_key.clone(),
                resolution: resolved_by_lookup_key
                    .get(file.lookup_key.as_str())
                    .cloned(),
            })
            .collect(),
        hash_cache_updates: hash_cache_diff_updates(&original_hash_cache, &hash_cache),
    }
}

pub(super) fn hash_cache_diff_updates(
    previous: &InstalledContentHashCache,
    current: &InstalledContentHashCache,
) -> Vec<content_resolver::InstalledContentHashCacheUpdate> {
    current
        .entries
        .iter()
        .filter(|(hash_key, resolution)| {
            previous.entries.get(hash_key.as_str()) != Some(*resolution)
        })
        .map(
            |(hash_key, resolution)| content_resolver::InstalledContentHashCacheUpdate {
                hash_key: hash_key.clone(),
                resolution: resolution.clone(),
            },
        )
        .collect()
}

/// Rescans `kind` after a change and forgets what was resolved for `changed_lookup_keys` only; every other file keeps
/// its resolved metadata (and its entry in the hash cache), since nothing about it changed.
pub(super) fn refresh_cached_metadata_after_apply(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    kind: InstalledContentKind,
    changed_lookup_keys: &[String],
) {
    let changed_keys = changed_lookup_keys.iter().cloned().collect::<HashSet<_>>();

    state.installed_content_cache.files_by_tab.remove(&kind);
    state
        .content_metadata_cache
        .retain(|key, _| !changed_keys.contains(key));
    state
        .installed_content_entry_ui_cache
        .retain(|key, _| !changed_keys.contains(key));
    state
        .content_lookup_in_flight
        .retain(|key| !changed_keys.contains(key));
    state
        .content_lookup_latest_serial_by_key
        .retain(|key, _| !changed_keys.contains(key));
    state
        .content_lookup_retry_after_by_key
        .retain(|key, _| !changed_keys.contains(key));
    state
        .content_lookup_failure_count_by_key
        .retain(|key, _| !changed_keys.contains(key));

    request_installed_content_scan(state, instance_root, kind);
}

pub(super) fn content_lookup_retry_delay(failure_count: u8) -> Duration {
    match failure_count {
        0 | 1 => Duration::from_secs(2),
        2 => Duration::from_secs(10),
        3 => Duration::from_secs(30),
        _ => Duration::from_secs(120),
    }
}
