//! Toolkit-free view of the Discover screen: the same search, filtering, detail and install logic the egui screen
//! runs, exposed as plain data and calls so any front end can draw it.

use vertex_i18n::tr;
use super::*;
use super::discover_detail::{
    build_install_request, poll_detail_versions, request_detail_versions, selected_detail_provider_ref,
    selected_detail_source,
};

/// One search result tile.
#[derive(Clone, Debug)]
pub struct EntryView {
    pub key: String,
    pub name: String,
    pub summary: String,
    pub author: Option<String>,
    pub icon_url: Option<String>,
    pub sources: Vec<String>,
    pub downloads: Option<String>,
    pub updated: Option<String>,
}

impl EntryView {
    fn of(entry: &DiscoverEntry) -> Self {
        Self {
            key: entry.dedupe_key.clone(),
            name: entry.name.clone(),
            summary: entry.summary.clone(),
            author: entry.author.clone().filter(|author| !author.trim().is_empty()),
            icon_url: entry.icon_url.clone(),
            sources: entry.sources.iter().map(|source| source.label().into_owned()).collect(),
            downloads: entry.popularity_score.map(format_compact_number),
            updated: entry.updated_at.as_deref().map(format_short_date),
        }
    }
}

/// The four filter dropdowns' labels and current selections.
#[derive(Clone, Debug)]
pub struct Filters {
    pub providers: Vec<String>,
    pub loaders: Vec<String>,
    pub sorts: Vec<String>,
    pub game_versions: Vec<String>,
    pub provider: usize,
    pub loader: usize,
    pub sort: usize,
    pub game_version: usize,
}

#[derive(Clone, Debug, Default)]
pub struct BrowseStatus {
    pub message: Option<String>,
    pub warnings: Vec<String>,
    pub searching: bool,
    pub has_more: bool,
}

/// Polls background work and starts the first search. Call every frame while the screen is visible; returns whether
/// something is still in flight (the caller should look again soon).
pub fn tick(state: &mut DiscoverState) -> bool {
    poll_version_catalog(state);
    request_version_catalog(state);
    poll_search_results(state);
    if !state.initial_search_requested {
        state.initial_search_requested = true;
        request_search(state, false, SearchMode::Replace);
    }
    state.search_in_flight || state.version_catalog_in_flight
}

pub fn entries(state: &DiscoverState) -> Vec<EntryView> {
    state.entries.iter().map(EntryView::of).collect()
}

pub fn status(state: &DiscoverState) -> BrowseStatus {
    BrowseStatus {
        message: state.status_message.clone(),
        warnings: state.warnings.clone(),
        searching: state.search_in_flight,
        has_more: state.has_more_results,
    }
}

pub fn filters(state: &DiscoverState) -> Filters {
    let game_version = if state.game_version_filter.trim().is_empty() {
        0
    } else {
        state
            .available_game_versions
            .iter()
            .position(|version| version.id == state.game_version_filter)
            .map_or(0, |index| index + 1)
    };
    Filters {
        providers: DiscoverProviderFilter::ALL.iter().map(|value| value.label().into_owned()).collect(),
        loaders: DiscoverLoaderFilter::ALL.iter().map(|value| value.label().into_owned()).collect(),
        sorts: DiscoverSortMode::ALL.iter().map(|value| value.label().into_owned()).collect(),
        game_versions: state.game_version_options.clone(),
        provider: DiscoverProviderFilter::ALL.iter().position(|value| *value == state.provider_filter).unwrap_or(0),
        loader: DiscoverLoaderFilter::ALL.iter().position(|value| *value == state.loader_filter).unwrap_or(0),
        sort: DiscoverSortMode::ALL.iter().position(|value| *value == state.sort_mode).unwrap_or(0),
        game_version,
    }
}

/// Applies the dropdown selections and re-runs the search if any changed.
pub fn set_filters(state: &mut DiscoverState, provider: usize, loader: usize, sort: usize, game_version: usize) {
    let current = filters(state);
    if (provider, loader, sort, game_version) == (current.provider, current.loader, current.sort, current.game_version) {
        return;
    }
    state.provider_filter = DiscoverProviderFilter::ALL.get(provider).copied().unwrap_or_default();
    state.loader_filter = DiscoverLoaderFilter::ALL.get(loader).copied().unwrap_or_default();
    state.sort_mode = DiscoverSortMode::ALL.get(sort).copied().unwrap_or_default();
    state.game_version_filter = match game_version.checked_sub(1) {
        Some(index) => state.available_game_versions.get(index).map(|version| version.id.clone()).unwrap_or_default(),
        None => String::new(),
    };
    request_search(state, true, SearchMode::Replace);
}

pub fn query_mut(state: &mut DiscoverState) -> &mut String {
    &mut state.query_input
}

pub fn tags(state: &DiscoverState) -> &[String] {
    &state.search_tags
}

pub fn remove_tag(state: &mut DiscoverState, index: usize) {
    if index < state.search_tags.len() {
        state.search_tags.remove(index);
        request_search(state, true, SearchMode::Replace);
    }
}

/// Enter in the search box. With `as_tag` (Shift+Enter) the text becomes a tag chip instead of the query.
pub fn submit(state: &mut DiscoverState, as_tag: bool) {
    if as_tag {
        let query = state.query_input.clone();
        if add_search_tag(&mut state.search_tags, &query) {
            state.query_input.clear();
        } else {
            return;
        }
    }
    request_search(state, true, SearchMode::Replace);
}

/// Appends the next page of results (no-op while searching or when there are no more).
pub fn load_more(state: &mut DiscoverState) {
    if state.has_more_results && !state.search_in_flight {
        request_search(state, false, SearchMode::Append);
    }
}

/// Opens the detail page for the result with `key`.
pub fn open_detail(state: &mut DiscoverState, key: &str) -> bool {
    let Some(entry) = state.entries.iter().find(|entry| entry.dedupe_key == key).cloned() else { return false };
    open_detail_page(state, &entry);
    true
}

#[derive(Clone, Debug)]
pub struct VersionView {
    pub id: String,
    pub name: String,
    pub published: Option<String>,
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
    pub downloads: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DetailView {
    pub entry: EntryView,
    pub sources: Vec<String>,
    pub selected_source: usize,
    pub project_url: Option<String>,
    pub versions: Vec<VersionView>,
    pub versions_loading: bool,
    pub versions_error: Option<String>,
    pub install_in_flight: bool,
    pub install_message: Option<String>,
    /// Install progress 0..=1 once the total is known.
    pub install_progress: Option<f32>,
    pub install_error: Option<String>,
}

/// Polls the detail page's background work; returns whether something is in flight.
pub fn detail_tick(state: &mut DiscoverState) -> bool {
    poll_detail_versions(state);
    request_detail_versions(state);
    state.detail_versions_in_flight || state.install_in_flight
}

pub fn detail(state: &DiscoverState) -> Option<DetailView> {
    let entry = state.detail_entry.as_ref()?;
    let selected = selected_detail_source(state, entry);
    Some(DetailView {
        entry: EntryView::of(entry),
        sources: entry.provider_refs.iter().map(|provider| provider.source.label().into_owned()).collect(),
        selected_source: entry.provider_refs.iter().position(|provider| provider.source == selected).unwrap_or(0),
        project_url: selected_detail_provider_ref(entry, selected).and_then(|provider| provider.primary_url.clone()),
        versions: state
            .detail_versions
            .iter()
            .map(|version| VersionView {
                id: version.version_id.clone(),
                name: version.version_name.clone(),
                published: version.published_at.as_deref().map(format_short_date),
                loaders: version.loaders.clone(),
                game_versions: version.game_versions.clone(),
                downloads: version.download_count.map(format_compact_number),
            })
            .collect(),
        versions_loading: state.detail_versions_in_flight,
        versions_error: state.detail_versions_error.clone(),
        install_in_flight: state.install_in_flight,
        install_message: state.install_message.clone(),
        install_progress: (state.install_total_steps > 0)
            .then(|| state.install_completed_steps as f32 / state.install_total_steps as f32),
        install_error: state.install_error.clone(),
    })
}

pub fn detail_select_source(state: &mut DiscoverState, index: usize) {
    let Some(entry) = state.detail_entry.as_ref() else { return };
    let source = entry.provider_refs.get(index).map(|provider| provider.source);
    if source != state.detail_selected_source {
        state.detail_selected_source = source;
        state.detail_versions.clear();
        state.detail_versions_error = None;
        state.detail_versions_in_flight = false;
    }
}

/// Starts creating an instance from version `version_id`. The returned request is what the app hands to the
/// install flow; `None` if that version cannot be installed.
pub fn detail_install(state: &mut DiscoverState, version_id: &str) -> Option<DiscoverInstallRequest> {
    let entry = state.detail_entry.clone()?;
    let version = state.detail_versions.iter().find(|version| version.version_id == version_id)?.clone();
    let request = build_install_request(&entry, &version)?;
    state.begin_install(tr!("Preparing {version}...", version = version.version_name).into_owned());
    Some(request)
}
