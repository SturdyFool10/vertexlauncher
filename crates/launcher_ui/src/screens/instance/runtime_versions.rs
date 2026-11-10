use vertex_i18n::tr;
use super::*;

const VERSION_CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(75);
const MODLOADER_VERSIONS_FETCH_TIMEOUT: Duration = Duration::from_secs(45);

pub(in super::super) fn split_modloader(modloader: &str) -> (usize, String) {
    for (index, option) in MODLOADER_OPTIONS.iter().enumerate() {
        if option.eq_ignore_ascii_case(modloader.trim()) {
            return (index, String::new());
        }
    }

    (CUSTOM_MODLOADER_INDEX, modloader.trim().to_owned())
}

pub(in super::super) fn selected_modloader_value(state: &InstanceScreenState) -> String {
    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
        state.custom_modloader.trim().to_owned()
    } else {
        MODLOADER_OPTIONS
            .get(state.selected_modloader)
            .copied()
            .unwrap_or(MODLOADER_OPTIONS[0])
            .to_owned()
    }
}

pub(in super::super) fn save_instance_metadata_and_versions(
    state: &mut InstanceScreenState,
    instance_id: &str,
    instances: &mut InstanceStore,
) -> Result<(), String> {
    let trimmed_name = state.name_input.trim();
    if trimmed_name.is_empty() {
        return Err(tr!("Name cannot be empty.").into_owned());
    }

    let modloader = selected_modloader_value(state);
    let game_version = state.game_version_input.trim().to_owned();
    if game_version.is_empty() {
        return Err(tr!("Minecraft game version cannot be empty.").into_owned());
    }
    if modloader.trim().is_empty() {
        return Err(tr!("Modloader cannot be empty.").into_owned());
    }
    let resolved_modloader_version =
        resolve_modloader_version_for_settings(state, modloader.as_str(), game_version.as_str())?;

    tracing::info!(
        target: "vertexlauncher/ui/instance",
        instance_id = %instance_id,
        requested_modloader = %modloader,
        requested_game_version = %game_version,
        requested_modloader_version = %resolved_modloader_version,
        "Saving instance metadata and versions from settings modal."
    );

    if let Some(instance) = instances.find_mut(instance_id) {
        instance.name = trimmed_name.to_owned();
        instance.description = normalize_optional(state.description_input.as_str());
        instance.thumbnail_path = if state.thumbnail_input.as_os_str().is_empty() {
            None
        } else {
            Some(state.thumbnail_input.clone())
        };
    } else {
        return Err(tr!("Instance was removed before save.").into_owned());
    }

    set_instance_versions(
        instances,
        instance_id,
        modloader,
        game_version,
        resolved_modloader_version,
    )
    .map_err(|err| err.to_string())
}

pub(in super::super) fn poll_background_tasks(
    state: &mut InstanceScreenState,
    config: &mut Config,
    instances: &mut InstanceStore,
    instance_id: &str,
) {
    poll_version_catalog(state);
    poll_modloader_versions(state);
    poll_content_lookup_results(state);
    poll_runtime_progress(state);
    poll_runtime_prepare(state, config, instances, instance_id);
}

pub(in super::super) fn sync_version_catalog(
    state: &mut InstanceScreenState,
    filter: VersionCatalogFilter,
    force_refresh: bool,
) {
    let should_refresh = force_refresh
        || state.version_catalog_filter != Some(filter)
        || (state.available_game_versions.is_empty() && state.version_catalog_error.is_none());
    if !should_refresh || state.version_catalog_in_flight {
        return;
    }

    let tx = state.version_catalog.sender();

    state.version_catalog_in_flight = true;
    state.version_catalog_error = None;
    state.version_catalog_filter = Some(filter);
    tracing::info!(
        target: "vertexlauncher/instance_runtime",
        ?filter,
        force_refresh,
        "Starting instance version catalog fetch."
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let result: Result<VersionCatalog, String> = match launcher_runtime::timeout(
            VERSION_CATALOG_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || {
                fetch_version_catalog_with_refresh(filter, force_refresh)
                    .map_err(|err| err.to_string())
            }),
        )
        .await
        {
            Ok(join_result) => join_result
                .map_err(|err| err.to_string())
                .and_then(|result| result),
            Err(_) => Err(tr!("version catalog request timed out after {seconds}s", seconds = VERSION_CATALOG_FETCH_TIMEOUT.as_secs()).into_owned()),
        };
        match &result {
            Ok(catalog) => tracing::info!(
                target: "vertexlauncher/instance_runtime",
                ?filter,
                force_refresh,
                game_versions = catalog.game_versions.len(),
                "Instance version catalog fetch completed."
            ),
            Err(error) => tracing::warn!(
                target: "vertexlauncher/instance_runtime",
                ?filter,
                force_refresh,
                error = %error,
                "Instance version catalog fetch failed."
            ),
        }
        if let Err(err) = tx.send((filter, result)) {
            tracing::error!(
                target: "vertexlauncher/instance_runtime",
                ?filter,
                force_refresh,
                error = %err,
                "Failed to deliver instance version catalog result."
            );
        }
    });
}

pub(in super::super) fn apply_version_catalog(
    state: &mut InstanceScreenState,
    filter: VersionCatalogFilter,
    catalog: VersionCatalog,
) {
    state.available_game_versions = catalog.game_versions;
    state.loader_support = catalog.loader_support;
    state.loader_versions = catalog.loader_versions;
    state.version_catalog_error = None;
    state.version_catalog_filter = Some(filter);

    if state.available_game_versions.is_empty() {
        state.selected_game_version_index = 0;
        state.game_version_input.clear();
        return;
    }

    let preferred_index = if state.game_version_input.trim().is_empty() {
        0
    } else {
        state
            .available_game_versions
            .iter()
            .position(|entry| entry.id == state.game_version_input)
            .unwrap_or(0)
    };
    state.selected_game_version_index = preferred_index;
    if let Some(selected) = state.available_game_versions.get(preferred_index) {
        state.game_version_input = selected.id.clone();
    }
}

pub(in super::super) fn apply_version_catalog_error(
    state: &mut InstanceScreenState,
    filter: VersionCatalogFilter,
    error: &str,
) {
    state.version_catalog_error = Some(tr!("Failed to fetch version catalog: {error}", error = error).into_owned());
    state.available_game_versions.clear();
    state.loader_support = LoaderSupportIndex::default();
    state.loader_versions = LoaderVersionIndex::default();
    state.version_catalog_filter = Some(filter);
    state.selected_game_version_index = 0;
    state.game_version_input.clear();
}

pub(in super::super) fn poll_version_catalog(state: &mut InstanceScreenState) {
    let drained = state.version_catalog.drain();
    if drained.disconnected {
        tracing::error!(
            target: "vertexlauncher/instance_runtime",
            "Instance version catalog worker stopped unexpectedly."
        );
        state.version_catalog_in_flight = false;
        state.version_catalog_error =
            Some(tr!("Version catalog worker stopped unexpectedly.").into_owned());
    }
    let updates = drained.items;

    for (filter, result) in updates {
        state.version_catalog_in_flight = false;
        match result {
            Ok(catalog) => apply_version_catalog(state, filter, catalog),
            Err(err) => apply_version_catalog_error(state, filter, &err),
        }
    }
}

pub(in super::super) fn selected_modloader_versions<'a>(
    state: &'a InstanceScreenState,
    game_version: &str,
) -> &'a [String] {
    if game_version.trim().is_empty() {
        return &[];
    }
    let selected_label = MODLOADER_OPTIONS
        .get(state.selected_modloader)
        .copied()
        .unwrap_or(MODLOADER_OPTIONS[0]);
    state
        .loader_versions
        .versions_for_loader(selected_label, game_version)
        .unwrap_or(&[])
}

pub(in super::super) fn modloader_versions_cache_key(
    loader_label: &str,
    game_version: &str,
) -> String {
    format!(
        "{}|{}",
        loader_label.trim().to_ascii_lowercase(),
        game_version.trim()
    )
}

pub(in super::super) fn request_modloader_versions(
    state: &mut InstanceScreenState,
    loader_label: &str,
    game_version: &str,
    force_refresh: bool,
) {
    let loader_label = loader_label.trim();
    let game_version = game_version.trim();
    if loader_label.is_empty() || game_version.is_empty() {
        return;
    }

    let key = modloader_versions_cache_key(loader_label, game_version);
    if force_refresh {
        state.modloader_versions_cache.remove(&key);
    } else if state.modloader_versions_cache.contains_key(&key)
        || state.modloader_versions_in_flight.contains(&key)
    {
        return;
    }

    let tx = state.modloader_versions_results.sender();

    state.modloader_versions_in_flight.insert(key.clone());
    state.modloader_versions_status_key = Some(key.clone());
    state.modloader_versions_status = Some(tr!("Fetching {loader_label} versions for Minecraft {game_version}...", loader_label = loader_label, game_version = game_version).into_owned());
    tracing::info!(
        target: "vertexlauncher/instance_runtime",
        loader = loader_label,
        game_version,
        force_refresh,
        "Starting instance modloader version fetch."
    );

    let loader = loader_label.to_owned();
    let game = game_version.to_owned();
    let loader_for_log = loader.clone();
    let game_for_log = game.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let result: Result<Vec<String>, String> = match launcher_runtime::timeout(
            MODLOADER_VERSIONS_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || {
                fetch_loader_versions_for_game(loader.as_str(), game.as_str(), force_refresh)
                    .map_err(|err| err.to_string())
            }),
        )
        .await
        {
            Ok(join_result) => join_result
                .map_err(|err| err.to_string())
                .and_then(|result| result),
            Err(_) => Err(tr!("modloader version request timed out after {seconds}s", seconds = MODLOADER_VERSIONS_FETCH_TIMEOUT.as_secs()).into_owned()),
        };
        match &result {
            Ok(versions) => tracing::info!(
                target: "vertexlauncher/instance_runtime",
                loader = %loader_for_log,
                game_version = %game_for_log,
                versions = versions.len(),
                "Instance modloader version fetch completed."
            ),
            Err(error) => tracing::warn!(
                target: "vertexlauncher/instance_runtime",
                loader = %loader_for_log,
                game_version = %game_for_log,
                error = %error,
                "Instance modloader version fetch failed."
            ),
        }
        if let Err(err) = tx.send((key.clone(), result)) {
            tracing::error!(
                target: "vertexlauncher/instance_runtime",
                key = %key,
                loader = %loader_for_log,
                game_version = %game_for_log,
                error = %err,
                "Failed to deliver instance modloader version result."
            );
        }
    });
}

pub(in super::super) fn poll_modloader_versions(state: &mut InstanceScreenState) {
    let drained = state.modloader_versions_results.drain();
    let updates = drained.items;

    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/instance_runtime", "modloader_versions_results worker channel stopped unexpectedly.");
        state.modloader_versions_in_flight.clear();
        state.modloader_versions_status =
            Some(tr!("Modloader version worker stopped unexpectedly.").into_owned());
    }

    for (key, result) in updates {
        state.modloader_versions_in_flight.remove(&key);
        state.modloader_versions_status_key = Some(key.clone());
        match result {
            Ok(versions) => {
                state.modloader_versions_cache.insert(key, versions.clone());
                state.modloader_versions_status = if versions.is_empty() {
                    Some(tr!("No modloader versions found for this Minecraft version.").into_owned())
                } else {
                    Some(tr!("Loaded {count} modloader versions.", count = versions.len()).into_owned())
                };
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/instance_runtime",
                    cache_key = %key,
                    error = %err,
                    "Applying instance modloader-version fetch failure."
                );
                state.modloader_versions_cache.insert(key, Vec::new());
                state.modloader_versions_status =
                    Some(tr!("Failed to fetch modloader versions: {err}", err = err).into_owned());
            }
        }
    }
}

pub(in super::super) fn is_latest_modloader_version_alias(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "latest" | "latest available" | "use latest version" | "auto" | "default"
    )
}

pub(in super::super) fn resolve_latest_modloader_version_from_state(
    state: &InstanceScreenState,
    modloader_label: &str,
    game_version: &str,
) -> Option<String> {
    if game_version.trim().is_empty() {
        return None;
    }

    if let Some(version) = state
        .loader_versions
        .versions_for_loader(modloader_label, game_version)
        .and_then(|versions| versions.first())
    {
        return Some(version.clone());
    }

    let key = modloader_versions_cache_key(modloader_label, game_version);
    state
        .modloader_versions_cache
        .get(&key)
        .and_then(|versions| versions.first().cloned())
}

pub(in super::super) fn resolve_modloader_version_for_settings(
    state: &InstanceScreenState,
    modloader_label: &str,
    game_version: &str,
) -> Result<String, String> {
    let raw_modloader_version = state.modloader_version_input.trim();
    let normalized_loader = modloader_label.trim().to_ascii_lowercase();
    let catalog_loader = matches!(
        normalized_loader.as_str(),
        "fabric" | "forge" | "neoforge" | "quilt"
    );

    if !catalog_loader {
        return Ok(raw_modloader_version.to_owned());
    }

    if raw_modloader_version.is_empty() || is_latest_modloader_version_alias(raw_modloader_version)
    {
        return resolve_latest_modloader_version_from_state(state, modloader_label, game_version)
            .ok_or_else(|| {
                tr!("Could not resolve latest {modloader_label} version for Minecraft {game_version}. Refresh modloader versions and try again.", modloader_label = modloader_label, game_version = game_version).into_owned()
            });
    }

    let matches_catalog = state
        .loader_versions
        .versions_for_loader(modloader_label, game_version)
        .is_some_and(|versions| {
            versions
                .iter()
                .any(|version| version.eq_ignore_ascii_case(raw_modloader_version))
        });
    let matches_cache = state
        .modloader_versions_cache
        .get(&modloader_versions_cache_key(modloader_label, game_version))
        .is_some_and(|versions| {
            versions
                .iter()
                .any(|version| version.eq_ignore_ascii_case(raw_modloader_version))
        });
    if matches_catalog || matches_cache {
        Ok(raw_modloader_version.to_owned())
    } else {
        Err(tr!("{modloader_label} {raw_modloader_version} is not available for Minecraft {game_version}.", modloader_label = modloader_label, raw_modloader_version = raw_modloader_version, game_version = game_version).into_owned())
    }
}

pub(in super::super) fn selected_game_version(state: &InstanceScreenState) -> &str {
    state
        .available_game_versions
        .get(state.selected_game_version_index)
        .map(|entry| entry.id.as_str())
        .unwrap_or_else(|| state.game_version_input.as_str())
}

pub(in super::super) fn configured_java_path_options(config: &Config) -> Vec<(u8, String)> {
    let mut options = Vec::new();
    for runtime in JavaRuntimeVersion::ALL {
        let Some(path) = config.java_runtime_path_ref(runtime) else {
            continue;
        };
        let trimmed = path.as_os_str().to_string_lossy().trim().to_owned();
        if trimmed.is_empty() {
            continue;
        }
        options.push((
            runtime.major(),
            tr!("Java {seconds} ({trimmed})", seconds = runtime.major(), trimmed = trimmed).into_owned(),
        ));
    }
    options
}
