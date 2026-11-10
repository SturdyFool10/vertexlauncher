use vertex_i18n::tr;
use super::*;

#[path = "discover_detail/discover_version_entry.rs"]
mod discover_version_entry;
#[path = "discover_detail/discover_versions_result.rs"]
mod discover_versions_result;

pub(super) use self::discover_version_entry::DiscoverVersionEntry;
pub(super) use self::discover_versions_result::DiscoverVersionsResult;



pub(super) fn open_detail_page(state: &mut DiscoverState, entry: &DiscoverEntry) {
    let same_entry = state
        .detail_entry
        .as_ref()
        .is_some_and(|current| current.dedupe_key == entry.dedupe_key);
    if !same_entry {
        state.detail_entry = Some(entry.clone());
        state.detail_selected_source = entry.provider_refs.first().map(|provider| provider.source);
        state.detail_versions.clear();
        state.detail_versions_error = None;
        state.detail_versions_in_flight = false;
        state.detail_version_request_serial = 0;
        state.install_in_flight = false;
        state.install_message = None;
        state.install_error = None;
        state.install_completed_steps = 0;
        state.install_total_steps = 0;
    }
}

pub(super) fn selected_detail_source(state: &DiscoverState, entry: &DiscoverEntry) -> DiscoverSource {
    state
        .detail_selected_source
        .filter(|source| {
            entry
                .provider_refs
                .iter()
                .any(|provider| provider.source == *source)
        })
        .or_else(|| entry.provider_refs.first().map(|provider| provider.source))
        .unwrap_or(DiscoverSource::Modrinth)
}

pub(super) fn selected_detail_provider_ref<'a>(
    entry: &'a DiscoverEntry,
    selected_source: DiscoverSource,
) -> Option<&'a DiscoverProviderRef> {
    entry
        .provider_refs
        .iter()
        .find(|provider| provider.source == selected_source)
}

pub(super) fn build_install_request(
    entry: &DiscoverEntry,
    version: &DiscoverVersionEntry,
) -> Option<DiscoverInstallRequest> {
    let provider = selected_provider_for_version(entry, version)?;
    let source = match (&provider.project_ref, version.source) {
        (DiscoverProjectRef::Modrinth { project_id }, DiscoverSource::Modrinth) => {
            DiscoverInstallSource::Modrinth {
                project_id: project_id.clone(),
                version_id: version.version_id.clone(),
                file_url: version.file_url.clone()?,
                file_name: version.file_name.clone(),
            }
        }
        (DiscoverProjectRef::CurseForge { project_id }, DiscoverSource::CurseForge) => {
            DiscoverInstallSource::CurseForge {
                project_id: *project_id,
                file_id: version.version_id.parse().ok()?,
                file_name: version.file_name.clone(),
                download_url: version.file_url.clone(),
                manual_download_path: None,
            }
        }
        _ => return None,
    };
    Some(DiscoverInstallRequest {
        instance_name: entry.name.clone(),
        project_summary: instances::normalize_optional(entry.summary.as_str()),
        icon_url: entry.icon_url.clone(),
        version_name: version.version_name.clone(),
        source,
    })
}

fn selected_provider_for_version<'a>(
    entry: &'a DiscoverEntry,
    version: &DiscoverVersionEntry,
) -> Option<&'a DiscoverProviderRef> {
    entry
        .provider_refs
        .iter()
        .find(|provider| provider.source == version.source)
}

pub(super) fn request_detail_versions(state: &mut DiscoverState) {
    if state.detail_versions_in_flight
        || !state.detail_versions.is_empty()
        || state.detail_versions_error.is_some()
    {
        return;
    }
    let Some(entry) = state.detail_entry.as_ref().cloned() else {
        return;
    };
    let selected_source = selected_detail_source(state, &entry);
    let Some(provider_ref) = selected_detail_provider_ref(&entry, selected_source).cloned() else {
        return;
    };

    let tx = state.detail_version_results.sender();

    state.detail_versions_in_flight = true;
    state.detail_version_request_serial = state.detail_version_request_serial.saturating_add(1);
    let request_serial = state.detail_version_request_serial;
    let loader_filter = state.loader_filter;
    let game_version_filter = instances::normalize_optional(state.game_version_filter.as_str());
    let _ = tokio_runtime::spawn_detached(async move {
        let versions: Result<Vec<DiscoverVersionEntry>, String> = match launcher_runtime::timeout(
            DETAIL_VERSIONS_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || {
                load_detail_versions(
                    &provider_ref,
                    selected_source,
                    loader_filter,
                    game_version_filter.as_deref(),
                )
            }),
        )
        .await
        {
            Ok(join_result) => join_result
                .map_err(|err| err.to_string())
                .and_then(|result| result),
            Err(_) => Err(tr!("detail version request timed out after {seconds}s", seconds = DETAIL_VERSIONS_FETCH_TIMEOUT.as_secs()).into_owned()),
        };
        if let Err(err) = tx.send(DiscoverVersionsResult {
            request_serial,
            versions,
        }) {
            tracing::error!(
                target: "vertexlauncher/discover",
                request_serial,
                error = %err,
                "Failed to deliver discover detail versions result."
            );
        }
    });
}

pub(super) fn poll_detail_versions(state: &mut DiscoverState) {
    let drained = state.detail_version_results.drain();
    if drained.disconnected {
        tracing::error!(
            target: "vertexlauncher/discover",
            request_serial = state.detail_version_request_serial,
            "Discover detail-version worker disconnected unexpectedly."
        );
        state.detail_versions_in_flight = false;
        state.detail_versions.clear();
        state.detail_versions_error =
            Some(tr!("Version detail worker stopped unexpectedly.").into_owned());
    }
    for result in drained.items {
        if result.request_serial != state.detail_version_request_serial {
            tracing::debug!(
                target: "vertexlauncher/discover",
                request_serial = result.request_serial,
                active_request_serial = state.detail_version_request_serial,
                "Ignoring stale discover detail-version result."
            );
            continue;
        }
        state.detail_versions_in_flight = false;
        match result.versions {
            Ok(versions) => {
                state.detail_versions = versions;
                state.detail_versions_error = None;
            }
            Err(error) => {
                tracing::warn!(
                    target: "vertexlauncher/discover",
                    request_serial = result.request_serial,
                    error = %error,
                    "Discover detail-version fetch failed."
                );
                state.detail_versions.clear();
                state.detail_versions_error = Some(error);
            }
        }
    }
}

fn load_detail_versions(
    provider_ref: &DiscoverProviderRef,
    source: DiscoverSource,
    loader_filter: DiscoverLoaderFilter,
    game_version_filter: Option<&str>,
) -> Result<Vec<DiscoverVersionEntry>, String> {
    match (&provider_ref.project_ref, source) {
        (DiscoverProjectRef::Modrinth { project_id }, DiscoverSource::Modrinth) => {
            let loaders = loader_filter
                .modrinth_slug()
                .map(|loader| vec![loader.to_owned()])
                .unwrap_or_default();
            let game_versions = game_version_filter
                .map(|version| vec![version.to_owned()])
                .unwrap_or_default();
            ModrinthClient::default()
                .list_project_versions(
                    project_id.as_str(),
                    loaders.as_slice(),
                    game_versions.as_slice(),
                )
                .map_err(|err| tr!("failed to load Modrinth versions: {err}", err = err).into_owned())
                .map(|versions| {
                    versions
                        .into_iter()
                        .filter_map(|version| {
                            let file = version
                                .files
                                .iter()
                                .find(|file| file.primary)
                                .or_else(|| version.files.first())?;
                            Some(DiscoverVersionEntry {
                                source: DiscoverSource::Modrinth,
                                version_id: version.id,
                                version_name: version.version_number,
                                published_at: instances::normalize_optional(
                                    version.date_published.as_str(),
                                ),
                                file_name: file.filename.clone(),
                                file_url: Some(file.url.clone()),
                                game_versions: version.game_versions,
                                loaders: version.loaders,
                                download_count: Some(version.downloads),
                            })
                        })
                        .collect()
                })
        }
        (DiscoverProjectRef::CurseForge { project_id }, DiscoverSource::CurseForge) => {
            let client = CurseForgeClient::from_env()
                .ok_or_else(|| tr!("CurseForge API key missing in settings.").into_owned())?;
            client
                .list_mod_files(
                    *project_id,
                    game_version_filter,
                    loader_filter.curseforge_mod_loader_type(),
                    0,
                    50,
                )
                .map_err(|err| tr!("failed to load CurseForge files: {err}", err = err).into_owned())
                .map(|files| {
                    files
                        .into_iter()
                        .map(|file| DiscoverVersionEntry {
                            source: DiscoverSource::CurseForge,
                            version_id: file.id.to_string(),
                            version_name: file.display_name,
                            published_at: instances::normalize_optional(file.file_date.as_str()),
                            file_name: file.file_name,
                            file_url: file.download_url,
                            game_versions: file.game_versions,
                            loaders: Vec::new(),
                            download_count: Some(file.download_count),
                        })
                        .collect()
                })
        }
        _ => Ok(Vec::new()),
    }
}

pub(super) fn format_compact_number(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.1}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

pub(super) fn format_short_date(value: &str) -> String {
    value.get(0..10).unwrap_or(value).to_owned()
}
