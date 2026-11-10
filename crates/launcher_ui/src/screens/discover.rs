use vertex_i18n::tr;
use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use curseforge::{Client as CurseForgeClient, MINECRAFT_GAME_ID};
use installation::{MinecraftVersionEntry, VersionCatalogFilter, fetch_version_catalog};
use modrinth::Client as ModrinthClient;


const DISCOVER_PROVIDER_LIMIT: u32 = 36;


const VERSION_CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(75);
const DETAIL_VERSIONS_FETCH_TIMEOUT: Duration = Duration::from_secs(45);
const DISCOVER_SEARCH_CACHE_MAX_ENTRIES: usize = 10;

#[path = "discover/cached_discover_masonry_layout.rs"]
mod cached_discover_masonry_layout;
#[path = "discover/discover_api.rs"]
pub mod api;
#[path = "discover_detail.rs"]
mod discover_detail;
#[path = "discover/discover_entry.rs"]
mod discover_entry;
#[path = "discover/discover_install_request.rs"]
mod discover_install_request;
#[path = "discover/discover_install_source.rs"]
mod discover_install_source;
#[path = "discover/discover_loader_filter.rs"]
mod discover_loader_filter;
#[path = "discover/discover_masonry_column.rs"]
mod discover_masonry_column;
#[path = "discover/discover_masonry_item.rs"]
mod discover_masonry_item;
#[path = "discover/discover_masonry_layout.rs"]
mod discover_masonry_layout;
#[path = "discover/discover_output.rs"]
mod discover_output;
#[path = "discover/discover_project_ref.rs"]
mod discover_project_ref;
#[path = "discover/discover_provider_entry.rs"]
mod discover_provider_entry;
#[path = "discover/discover_provider_filter.rs"]
mod discover_provider_filter;
#[path = "discover/discover_provider_ref.rs"]
mod discover_provider_ref;
#[path = "discover/discover_search_request.rs"]
mod discover_search_request;
#[path = "discover/discover_search_result.rs"]
mod discover_search_result;
#[path = "discover/discover_search_snapshot.rs"]
mod discover_search_snapshot;
#[path = "discover/discover_sort_mode.rs"]
mod discover_sort_mode;
#[path = "discover/discover_source.rs"]
mod discover_source;
#[path = "discover/discover_state.rs"]
mod discover_state;
#[path = "discover/discover_tile_render_result.rs"]
mod discover_tile_render_result;
#[path = "discover/discover_ui_metrics.rs"]
mod discover_ui_metrics;
#[path = "content_browser_state/game_version_options.rs"]
mod game_version_options;
#[path = "discover/search_mode.rs"]
mod search_mode;
#[path = "content_browser_ui/search_tag_chips.rs"]
mod search_tag_chips;

use self::cached_discover_masonry_layout::CachedDiscoverMasonryLayout;
use self::discover_detail::DiscoverVersionEntry;
use self::discover_detail::DiscoverVersionsResult;
use self::discover_detail::format_compact_number;
use self::discover_detail::format_short_date;
use self::discover_detail::open_detail_page;
use self::discover_entry::DiscoverEntry;
pub use self::discover_install_request::DiscoverInstallRequest;
pub use self::discover_install_source::DiscoverInstallSource;
use self::discover_loader_filter::DiscoverLoaderFilter;
use self::discover_masonry_column::DiscoverMasonryColumn;
use self::discover_masonry_item::DiscoverMasonryItem;
use self::discover_masonry_layout::DiscoverMasonryLayout;
use self::discover_project_ref::DiscoverProjectRef;
use self::discover_provider_entry::DiscoverProviderEntry;
use self::discover_provider_filter::DiscoverProviderFilter;
use self::discover_provider_ref::DiscoverProviderRef;
use self::discover_search_request::DiscoverSearchRequest;
use self::discover_search_result::DiscoverSearchResult;
use self::discover_search_snapshot::DiscoverSearchSnapshot;
use self::discover_sort_mode::DiscoverSortMode;
use self::discover_source::DiscoverSource;
pub use self::discover_state::DiscoverState;
use self::game_version_options::game_version_option_labels;
use self::search_mode::SearchMode;
use self::search_tag_chips::add_search_tag;
use crate::app::tokio_runtime;


















fn request_search(state: &mut DiscoverState, show_cached_status: bool, mode: SearchMode) {
    if state.search_in_flight {
        return;
    }
    let request = current_request(state, mode);
    if let Some(snapshot) = state.cached_snapshots.get(&request).cloned() {
        apply_search_snapshot(state, &request, snapshot, mode);
        state.search_in_flight = false;
        if show_cached_status {
            state.status_message = Some(tr!("Loaded cached discover results.").into_owned());
        }
        return;
    }

    let tx = state.search_results.sender();
    state.search_request_serial = state.search_request_serial.saturating_add(1);
    let request_serial = state.search_request_serial;
    state.search_in_flight = true;
    if mode == SearchMode::Replace {
        state.page = 1;
        state.has_more_results = true;
    }
    state.status_message = Some(tr!("Searching {request} for modpacks...", request = request.provider_filter.label()).into_owned());
    let request_for_task = request.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let outcome = Ok(perform_search(&request_for_task));
        if let Err(err) = tx.send(DiscoverSearchResult {
            request_serial,
            request,
            outcome,
        }) {
            tracing::error!(
                target: "vertexlauncher/discover",
                request_serial,
                error = %err,
                "Failed to deliver discover search result."
            );
        }
    });
}

fn trim_discover_search_cache(state: &mut DiscoverState) {
    if state.cached_snapshots.len() <= DISCOVER_SEARCH_CACHE_MAX_ENTRIES {
        return;
    }
    let active_request = current_request(state, SearchMode::Replace);
    state.cached_snapshots.retain(|request, _| {
        request == &active_request || request.page > state.page.saturating_sub(2)
    });
    if state.cached_snapshots.len() <= DISCOVER_SEARCH_CACHE_MAX_ENTRIES {
        return;
    }
    let mut requests = state.cached_snapshots.keys().cloned().collect::<Vec<_>>();
    requests.sort_by_key(|request| request.page);
    for request in requests {
        if state.cached_snapshots.len() <= DISCOVER_SEARCH_CACHE_MAX_ENTRIES {
            break;
        }
        if request != active_request {
            state.cached_snapshots.remove(&request);
        }
    }
}

fn poll_search_results(state: &mut DiscoverState) {
    let drained = state.search_results.drain();
    if drained.disconnected {
        tracing::error!(
            target: "vertexlauncher/discover",
            request_serial = state.search_request_serial,
            "Discover search worker disconnected unexpectedly."
        );
        state.search_in_flight = false;
        state.status_message = Some(tr!("Discover search worker stopped unexpectedly.").into_owned());
    }
    for result in drained.items {
        if result.request_serial != state.search_request_serial {
            tracing::debug!(
                target: "vertexlauncher/discover",
                request_serial = result.request_serial,
                active_request_serial = state.search_request_serial,
                "Ignoring stale discover search result."
            );
            continue;
        }
        state.search_in_flight = false;
        match result.outcome {
            Ok(snapshot) => {
                let mode = if result.request.page <= 1 {
                    SearchMode::Replace
                } else {
                    SearchMode::Append
                };
                apply_search_snapshot(state, &result.request, snapshot.clone(), mode);
                state.cached_snapshots.insert(result.request, snapshot);
                trim_discover_search_cache(state);
                state.status_message = Some(tr!("Showing {count} modpacks.", count = state.entries.len()).into_owned());
            }
            Err(error) => {
                tracing::warn!(
                    target: "vertexlauncher/discover",
                    request_serial = result.request_serial,
                    error = %error,
                    "Discover search failed."
                );
                state.status_message = Some(tr!("Discover search failed: {error}", error = error).into_owned());
                state.entries.clear();
                state.warnings.clear();
            }
        }
    }
}

fn current_request(state: &DiscoverState, mode: SearchMode) -> DiscoverSearchRequest {
    let combined_query =
        compose_search_query(state.query_input.as_str(), state.search_tags.as_slice());
    DiscoverSearchRequest {
        query: combined_query,
        tags: state.search_tags.clone(),
        game_version: instances::normalize_optional(state.game_version_filter.as_str()),
        provider_filter: state.provider_filter,
        loader_filter: state.loader_filter,
        sort_mode: state.sort_mode,
        page: match mode {
            SearchMode::Replace => 1,
            SearchMode::Append => state.page.saturating_add(1).max(1),
        },
    }
}

fn perform_search(request: &DiscoverSearchRequest) -> DiscoverSearchSnapshot {
    let modrinth = ModrinthClient::default();
    let curseforge = CurseForgeClient::from_env();
    let mut warnings = Vec::new();
    let offset = request
        .page
        .saturating_sub(1)
        .saturating_mul(DISCOVER_PROVIDER_LIMIT);
    let mut provider_entries = Vec::new();
    let mut provider_result_count = 0usize;

    if matches!(
        request.provider_filter,
        DiscoverProviderFilter::All | DiscoverProviderFilter::Modrinth
    ) {
        let modrinth_sort_index = match request.sort_mode {
            DiscoverSortMode::Popularity => Some("downloads"),
            DiscoverSortMode::LastUpdated => Some("updated"),
            DiscoverSortMode::Relevance => None,
        };
        match modrinth.search_projects_with_filters(
            request.query.as_str(),
            DISCOVER_PROVIDER_LIMIT,
            offset,
            Some("modpack"),
            request.game_version.as_deref(),
            request.loader_filter.modrinth_slug(),
            modrinth_sort_index,
        ) {
            Ok(entries) => {
                provider_result_count += entries.len();
                provider_entries.extend(entries.into_iter().enumerate().map(|(index, entry)| {
                    DiscoverProviderEntry {
                        project_ref: DiscoverProjectRef::Modrinth {
                            project_id: entry.project_id,
                        },
                        name: entry.title,
                        summary: entry.description,
                        author: entry.author,
                        icon_url: entry.icon_url,
                        primary_url: Some(entry.project_url),
                        source: DiscoverSource::Modrinth,
                        popularity_score: Some(entry.downloads),
                        updated_at: entry.date_modified,
                        relevance_rank: index as u32,
                    }
                }));
            }
            Err(error) => warnings.push(tr!("Modrinth search failed: {error}", error = error).into_owned()),
        }
    }

    if matches!(
        request.provider_filter,
        DiscoverProviderFilter::All | DiscoverProviderFilter::CurseForge
    ) {
        match curseforge {
            Some(client) => {
                let class_id = resolve_curseforge_modpack_class_id_cached(&client, &mut warnings);
                if let Some(class_id) = class_id {
                    // CurseForge sortField: 6 = TotalDownloads, 3 = LastUpdated
                    let curseforge_sort_field = match request.sort_mode {
                        DiscoverSortMode::Popularity => Some(6),
                        DiscoverSortMode::LastUpdated => Some(3),
                        DiscoverSortMode::Relevance => None,
                    };
                    match client.search_projects_with_filters(
                        MINECRAFT_GAME_ID,
                        request.query.as_str(),
                        offset,
                        DISCOVER_PROVIDER_LIMIT,
                        Some(class_id),
                        request.game_version.as_deref(),
                        request.loader_filter.curseforge_mod_loader_type(),
                        curseforge_sort_field,
                    ) {
                        Ok(entries) => {
                            provider_result_count += entries.len();
                            provider_entries.extend(entries.into_iter().enumerate().map(
                                |(index, entry)| DiscoverProviderEntry {
                                    project_ref: DiscoverProjectRef::CurseForge {
                                        project_id: entry.id,
                                    },
                                    name: entry.name,
                                    summary: entry.summary,
                                    author: None,
                                    icon_url: entry.icon_url,
                                    primary_url: entry.website_url,
                                    source: DiscoverSource::CurseForge,
                                    popularity_score: Some(entry.download_count),
                                    updated_at: entry.date_modified,
                                    relevance_rank: index as u32,
                                },
                            ));
                        }
                        Err(error) => warnings.push(tr!("CurseForge search failed: {error}", error = error).into_owned()),
                    }
                }
            }
            None => warnings.push(
                tr!("CurseForge API key missing in settings. Showing Modrinth results only.").into_owned(),
            ),
        }
    }

    let entries = build_snapshot_entries(provider_entries, request.sort_mode);
    let expected_page_size =
        enabled_provider_count(request).saturating_mul(DISCOVER_PROVIDER_LIMIT as usize);
    DiscoverSearchSnapshot {
        entries,
        warnings,
        has_more: expected_page_size > 0 && provider_result_count >= expected_page_size,
    }
}

fn build_snapshot_entries(
    provider_entries: Vec<DiscoverProviderEntry>,
    sort_mode: DiscoverSortMode,
) -> Vec<DiscoverEntry> {
    let mut deduped = HashMap::<String, DiscoverEntry>::new();
    for entry in provider_entries {
        let dedupe_key = normalize_search_key(entry.name.as_str());
        match deduped.get_mut(&dedupe_key) {
            Some(existing) => {
                if !existing.sources.contains(&entry.source) {
                    existing.sources.push(entry.source);
                }
                if existing.summary.len() < entry.summary.len() {
                    existing.summary = entry.summary.clone();
                }
                if existing.author.is_none() {
                    existing.author = entry.author.clone();
                }
                if existing.icon_url.is_none() {
                    existing.icon_url = entry.icon_url.clone();
                }
                if existing.primary_url.is_none() {
                    existing.primary_url = entry.primary_url.clone();
                }
                if !existing.provider_refs.iter().any(|provider| {
                    provider.source == entry.source
                        && match (&provider.project_ref, &entry.project_ref) {
                            (
                                DiscoverProjectRef::Modrinth {
                                    project_id: left_project_id,
                                },
                                DiscoverProjectRef::Modrinth {
                                    project_id: right_project_id,
                                },
                            ) => left_project_id == right_project_id,
                            (
                                DiscoverProjectRef::CurseForge {
                                    project_id: left_project_id,
                                },
                                DiscoverProjectRef::CurseForge {
                                    project_id: right_project_id,
                                },
                            ) => left_project_id == right_project_id,
                            _ => false,
                        }
                }) {
                    existing.provider_refs.push(DiscoverProviderRef {
                        source: entry.source,
                        project_ref: entry.project_ref.clone(),
                        primary_url: entry.primary_url.clone(),
                    });
                }
                existing.popularity_score =
                    match (existing.popularity_score, entry.popularity_score) {
                        (Some(left), Some(right)) => Some(left.max(right)),
                        (None, right) => right,
                        (left, None) => left,
                    };
                existing.updated_at = existing
                    .updated_at
                    .take()
                    .or_else(|| entry.updated_at.clone());
                existing.relevance_rank = existing.relevance_rank.min(entry.relevance_rank);
            }
            None => {
                let primary_url = entry.primary_url.clone();
                deduped.insert(
                    dedupe_key.clone(),
                    DiscoverEntry {
                        dedupe_key,
                        name: entry.name,
                        summary: entry.summary,
                        author: entry.author,
                        icon_url: entry.icon_url,
                        primary_url: primary_url.clone(),
                        sources: vec![entry.source],
                        provider_refs: vec![DiscoverProviderRef {
                            source: entry.source,
                            project_ref: entry.project_ref,
                            primary_url,
                        }],
                        popularity_score: entry.popularity_score,
                        updated_at: entry.updated_at,
                        relevance_rank: entry.relevance_rank,
                    },
                );
            }
        }
    }

    let mut entries = deduped.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| match sort_mode {
        DiscoverSortMode::Popularity => right
            .popularity_score
            .cmp(&left.popularity_score)
            .then_with(|| left.name.cmp(&right.name)),
        DiscoverSortMode::LastUpdated => right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.name.cmp(&right.name)),
        DiscoverSortMode::Relevance => left
            .relevance_rank
            .cmp(&right.relevance_rank)
            .then_with(|| right.popularity_score.cmp(&left.popularity_score))
            .then_with(|| left.name.cmp(&right.name)),
    });
    entries
}

fn resolve_curseforge_modpack_class_id_cached(
    client: &CurseForgeClient,
    warnings: &mut Vec<String>,
) -> Option<u32> {
    static CACHE: OnceLock<Mutex<Option<Option<u32>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(cache) = cache.lock()
        && let Some(class_id) = *cache
    {
        return class_id;
    }

    let class_id = resolve_curseforge_modpack_class_id(client, warnings);
    if let Ok(mut cache) = cache.lock() {
        *cache = Some(class_id);
    }
    class_id
}

fn resolve_curseforge_modpack_class_id(
    client: &CurseForgeClient,
    warnings: &mut Vec<String>,
) -> Option<u32> {
    match client.list_content_classes(MINECRAFT_GAME_ID) {
        Ok(classes) => classes
            .into_iter()
            .find(|class_entry| {
                let normalized = normalize_search_key(class_entry.name.as_str());
                normalized.contains("modpack") || normalized.contains("mod pack")
            })
            .map(|class_entry| class_entry.id),
        Err(error) => {
            warnings.push(tr!("CurseForge modpack class discovery failed: {error}", error = error).into_owned());
            None
        }
    }
}

fn normalize_search_key(value: &str) -> String {
    value
        .trim()
        .chars()
        .flat_map(|ch| ch.to_lowercase())
        .filter(|ch| ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace())
        .collect::<String>()
}

fn compose_search_query(input: &str, tags: &[String]) -> String {
    let mut parts = Vec::with_capacity(tags.len() + 1);
    for tag in tags {
        let trimmed = tag.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_owned());
        }
    }
    let trimmed_input = input.trim();
    if !trimmed_input.is_empty() {
        parts.push(trimmed_input.to_owned());
    }
    parts.join(" ")
}



fn request_version_catalog(state: &mut DiscoverState) {
    if state.version_catalog_in_flight
        || !state.available_game_versions.is_empty()
        || state.version_catalog_error.is_some()
    {
        return;
    }

    let tx = state.version_catalog.sender();

    state.version_catalog_in_flight = true;
    tracing::info!(target: "vertexlauncher/discover", "Starting discover version catalog fetch.");
    let _ = tokio_runtime::spawn_detached(async move {
        let result: Result<Vec<MinecraftVersionEntry>, String> = match launcher_runtime::timeout(
            VERSION_CATALOG_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || {
                fetch_version_catalog(VersionCatalogFilter::default())
                    .map(|catalog| catalog.game_versions)
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
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/discover",
                error = %err,
                "Failed to deliver discover version catalog result."
            );
        }
    });
}

fn poll_version_catalog(state: &mut DiscoverState) {
    let drained = state.version_catalog.drain();
    if drained.disconnected {
        tracing::error!(
            target: "vertexlauncher/discover",
            "Discover version catalog worker stopped unexpectedly."
        );
        state.version_catalog_in_flight = false;
        state.version_catalog_error =
            Some(tr!("Version catalog worker stopped unexpectedly.").into_owned());
    }
    let updates = drained.items;

    for update in updates {
        state.version_catalog_in_flight = false;
        match update {
            Ok(versions) => {
                state.game_version_options = game_version_option_labels(&versions);
                state.available_game_versions = versions;
                state.version_catalog_error = None;
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/discover",
                    error = %err,
                    "Discover version catalog fetch failed."
                );
                state.version_catalog_error = Some(err);
            }
        }
    }
}

fn apply_search_snapshot(
    state: &mut DiscoverState,
    request: &DiscoverSearchRequest,
    snapshot: DiscoverSearchSnapshot,
    mode: SearchMode,
) {
    match mode {
        SearchMode::Replace => {
            state.entries = snapshot.entries;
            state.page = request.page.max(1);
        }
        SearchMode::Append => {
            for entry in snapshot.entries {
                if !state
                    .entries
                    .iter()
                    .any(|existing| existing.dedupe_key == entry.dedupe_key)
                {
                    state.entries.push(entry);
                }
            }
            state.page = request.page.max(state.page);
        }
    }
    state.warnings = snapshot.warnings;
    state.has_more_results = snapshot.has_more;
    state.masonry_layout_cache = None;
    retain_discover_tile_heights(state);
}

fn retain_discover_tile_heights(state: &mut DiscoverState) {
    state.tile_height_cache.retain(|(dedupe_key, _), _| {
        state
            .entries
            .iter()
            .any(|entry| entry.dedupe_key.as_str() == dedupe_key.as_str())
    });
    state.tile_height_cache_revision = state.tile_height_cache_revision.saturating_add(1);
    state.masonry_layout_cache = None;
}

fn enabled_provider_count(request: &DiscoverSearchRequest) -> usize {
    match request.provider_filter {
        DiscoverProviderFilter::All => 1 + usize::from(CurseForgeClient::from_env().is_some()),
        DiscoverProviderFilter::Modrinth => 1,
        DiscoverProviderFilter::CurseForge => usize::from(CurseForgeClient::from_env().is_some()),
    }
}
