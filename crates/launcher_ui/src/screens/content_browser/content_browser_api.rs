//! Toolkit-free view of the Content Browser: the same search, filter, paging, detail and install-queue logic the
//! egui screen runs, exposed as plain data and calls so any front end can draw it.

use super::*;

/// One search result.
#[derive(Clone, Debug)]
pub struct TileView {
    pub key: String,
    pub name: String,
    pub summary: String,
    pub kind: &'static str,
    pub sources: Vec<&'static str>,
    pub icon_url: Option<String>,
    /// The installed version's name (or "Installed") when this project is already in the instance.
    pub installed: Option<String>,
    pub downloading: bool,
    /// Waiting in the install queue behind the running download.
    pub queued: bool,
}

impl TileView {
    fn of(entry: &BrowserProjectEntry, manifest: &ContentInstallManifest, state: &ContentBrowserState) -> Self {
        Self {
            key: entry.dedupe_key.clone(),
            name: if entry.name.trim().is_empty() { "Unnamed".to_owned() } else { entry.name.trim().to_owned() },
            summary: if entry.summary.trim().is_empty() {
                "No description provided.".to_owned()
            } else {
                entry.summary.trim().to_owned()
            },
            kind: entry.content_type.label(),
            sources: entry.sources.iter().map(|source| source.label()).collect(),
            icon_url: entry.icon_url.clone(),
            installed: installed_project_for_entry(manifest, entry).map(|(_, project)| {
                project
                    .selected_version_name
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("Installed")
                    .to_owned()
            }),
            downloading: state.active_downloads.iter().any(|active| active.dedupe_key == entry.dedupe_key),
            queued: state.download_queue.iter().any(|queued| queued_dedupe_key(&queued.request) == entry.dedupe_key),
        }
    }
}

pub struct Group {
    pub heading: String,
    pub tiles: Vec<TileView>,
}

/// The dropdown labels and selections above the results.
#[derive(Clone, Debug)]
pub struct Controls {
    pub versions: Vec<String>,
    pub version: usize,
    pub scopes: Vec<String>,
    pub scope: usize,
    pub sorts: Vec<String>,
    pub sort: usize,
    pub loaders: Vec<String>,
    pub loader: usize,
    pub queue_status: String,
    pub identify_busy: bool,
}

/// The selected instance changed or this is the first frame: resets the browser to that instance's defaults.
fn ensure_instance(state: &mut ContentBrowserState, instance: &InstanceRecord) {
    if state.active_instance_id.as_deref() == Some(instance.id.as_str()) {
        return;
    }
    state.active_instance_id = Some(instance.id.clone());
    state.active_instance_name = Some(instance.name.clone());
    state.loader = browser_loader_from_modloader(instance.modloader.as_str());
    state.minecraft_version_filter = instance.game_version.clone();
    state.content_scope = ContentScope::Mods;
    state.mod_sort_mode = ModSortMode::Popularity;
    state.download_queue.clear();
    state.download_in_flight = false;
    state.active_downloads.clear();
    state.cached_manifest = None;
    state.manifest_dirty = true;
    state.current_page = 1;
    state.current_view = ContentBrowserPage::Browse;
    state.detail_entry = None;
    state.detail_tab = ContentDetailTab::Overview;
    state.detail_versions.clear();
    state.detail_versions_project_key = None;
    state.detail_versions_error = None;
    state.detail_versions_in_flight = false;
    state.detail_loader_filter = state.loader;
    state.detail_minecraft_version_filter = instance.game_version.clone();
    state.results = BrowserSearchSnapshot::default();
    state.active_search_request = None;
    state.search_completed_tasks = 0;
    state.search_total_tasks = 0;
    state.search_in_flight = false;
    state.search_notification_active = false;
    state.auto_populated_instance_id = None;
}

fn queued_dedupe_key(request: &ContentInstallRequest) -> &str {
    match request {
        ContentInstallRequest::Latest { entry, .. } | ContentInstallRequest::Exact { entry, .. } => entry.dedupe_key.as_str(),
    }
}

/// Sets the download limits (concurrent transfers, speed limit) the install queue runs under.
pub fn set_download_policy(state: &mut ContentBrowserState, policy: installation::DownloadPolicy) {
    state.download_policy = policy;
}

/// Keeps the install queue draining while the Content Browser is not on screen: finished downloads are collected and the
/// next queued one starts. Call every frame; it does nothing when nothing is queued or running.
pub fn pump_background(state: &mut ContentBrowserState) {
    if !state.download_in_flight && state.download_queue.is_empty() {
        return;
    }
    poll_downloads(state);
    let (Some(name), Some(root)) = (state.active_instance_name.clone(), state.active_instance_root.clone()) else {
        return;
    };
    maybe_start_queued_download(state, name.as_str(), root.as_path());
}

/// Polls background work and keeps the browser pointed at `instance`. Call every frame while the screen is visible;
/// returns whether something is in flight (the caller should look again soon).
pub fn tick(state: &mut ContentBrowserState, instance: &InstanceRecord, installations_root: &Path) -> bool {
    poll_search(state);
    poll_detail_versions(state);
    poll_downloads(state);
    poll_version_catalog(state);
    poll_identify_results(state);
    ensure_instance(state, instance);
    let instance_root = instance_root_path(installations_root, instance);
    state.active_instance_root = Some(instance_root.clone());
    request_version_catalog(state);
    apply_pending_external_detail_open(state);
    maybe_start_queued_download(state, instance.name.as_str(), instance_root.as_path());
    poll_manifest_load(state);
    if state.current_view == ContentBrowserPage::Browse {
        // Loading the manifest in the background keeps "installed" marks current.
        let _ = cached_manifest_for_instance(state, instance_root.as_path());
        if state.auto_populated_instance_id.as_deref() != Some(instance.id.as_str())
            && !state.search_in_flight
            && state.query_input.trim().is_empty()
            && state.search_tags.is_empty()
        {
            let request = BrowserSearchRequest {
                query: None,
                tags: Vec::new(),
                game_version: normalize_optional(state.minecraft_version_filter.as_str()),
                loader: state.loader,
                content_scope: ContentScope::Mods,
                mod_sort_mode: ModSortMode::Popularity,
                page: 1,
            };
            state.current_page = 1;
            request_search(state, request);
            state.auto_populated_instance_id = Some(instance.id.clone());
        }
    }
    state.search_in_flight
        || state.detail_versions_in_flight
        || state.download_in_flight
        || state.identify_in_flight
        || state.version_catalog_in_flight
        || state.manifest_load_in_flight
        || !state.download_queue.is_empty()
}

pub fn is_detail(state: &ContentBrowserState) -> bool {
    state.current_view == ContentBrowserPage::Detail
}

pub fn back_to_browse(state: &mut ContentBrowserState) {
    state.current_view = ContentBrowserPage::Browse;
}

/// "Instance: X | Minecraft 1.21 | Loader Fabric".
pub fn context_line(instance: &InstanceRecord) -> String {
    let game_version = instance.game_version.trim();
    format!(
        "Instance: {} | Minecraft {} | Loader {}",
        instance.name,
        if game_version.is_empty() { "n/a" } else { game_version },
        instance.modloader.trim()
    )
}

pub fn status(state: &ContentBrowserState) -> (Option<String>, Vec<String>) {
    (state.status_message.clone(), state.results.warnings.clone())
}

pub fn controls(state: &ContentBrowserState) -> Controls {
    let version = if state.minecraft_version_filter.trim().is_empty() {
        0
    } else {
        state
            .available_game_versions
            .iter()
            .position(|version| version.id == state.minecraft_version_filter)
            .map_or(0, |index| index + 1)
    };
    let queue_status = if state.download_in_flight {
        format!("Downloads: active, {} queued", state.download_queue.len())
    } else if state.download_queue.is_empty() {
        "Downloads: idle".to_owned()
    } else {
        format!("Downloads: idle, {} queued", state.download_queue.len())
    };
    Controls {
        versions: state.game_version_options.clone(),
        version,
        scopes: ContentScope::ALL.iter().map(|value| value.label().to_owned()).collect(),
        scope: ContentScope::ALL.iter().position(|value| *value == state.content_scope).unwrap_or(0),
        sorts: ModSortMode::ALL.iter().map(|value| value.label().to_owned()).collect(),
        sort: ModSortMode::ALL.iter().position(|value| *value == state.mod_sort_mode).unwrap_or(0),
        loaders: BrowserLoader::ALL.iter().map(|value| value.label().to_owned()).collect(),
        loader: BrowserLoader::ALL.iter().position(|value| *value == state.loader).unwrap_or(0),
        queue_status,
        identify_busy: state.identify_in_flight,
    }
}

/// Applies the dropdown selections and re-searches (from page 1) if any changed.
pub fn apply_controls(state: &mut ContentBrowserState, version: usize, scope: usize, sort: usize, loader: usize) {
    let current = controls(state);
    if (version, scope, sort, loader) == (current.version, current.scope, current.sort, current.loader) {
        return;
    }
    state.minecraft_version_filter = match version.checked_sub(1) {
        Some(index) => state.available_game_versions.get(index).map(|entry| entry.id.clone()).unwrap_or_default(),
        None => String::new(),
    };
    state.content_scope = ContentScope::ALL.get(scope).copied().unwrap_or(ContentScope::All);
    state.mod_sort_mode = ModSortMode::ALL.get(sort).copied().unwrap_or(ModSortMode::Popularity);
    state.loader = BrowserLoader::ALL.get(loader).copied().unwrap_or(BrowserLoader::Any);
    request_search_for_current_filters(state, true);
}

pub fn query_mut(state: &mut ContentBrowserState) -> &mut String {
    &mut state.query_input
}

pub fn tags(state: &ContentBrowserState) -> &[String] {
    &state.search_tags
}

pub fn remove_tag(state: &mut ContentBrowserState, index: usize) {
    if index < state.search_tags.len() {
        state.search_tags.remove(index);
        request_search_for_current_filters(state, true);
    }
}

/// Enter in the search box; with `as_tag` (Shift+Enter) the text becomes a tag chip.
pub fn submit(state: &mut ContentBrowserState, as_tag: bool) {
    if as_tag {
        let query = state.query_input.clone();
        if add_search_tag(&mut state.search_tags, &query) {
            state.query_input.clear();
        } else {
            return;
        }
    }
    if !state.search_in_flight {
        request_search_for_current_filters(state, true);
    }
}

pub fn searching(state: &ContentBrowserState) -> Option<String> {
    state.search_in_flight.then(|| {
        if state.search_total_tasks > 0 {
            format!(
                "Searching content... {}/{} result groups loaded.",
                state.search_completed_tasks, state.search_total_tasks
            )
        } else {
            "Searching content...".to_owned()
        }
    })
}

pub fn empty_message(state: &ContentBrowserState) -> &'static str {
    if state.search_in_flight {
        "Searching content..."
    } else if state.active_search_request.is_some() {
        "No content was found for this search. Try different words, another version or loader, or another content type."
    } else {
        "No results yet. Search to browse installable content."
    }
}

/// Whether a further page can have content: the search is done and this page came back full. (Providers do not say
/// how many results exist in total, so a short page is the sign of the last one.)
pub fn has_next_page(state: &ContentBrowserState) -> bool {
    !state.search_in_flight
        && state.results.entries.len() as u32 >= super::CONTENT_SEARCH_PER_PROVIDER_LIMIT
}

/// Fraction of the search that has finished, if one is running.
pub fn search_fraction(state: &ContentBrowserState) -> Option<f32> {
    (state.search_in_flight && state.search_total_tasks > 0)
        .then(|| (state.search_completed_tasks as f32 / state.search_total_tasks as f32).clamp(0.0, 1.0))
}

/// Results grouped by content type, with "installed" and "downloading" marks.
pub fn results(state: &mut ContentBrowserState, instance_root: &Path) -> Vec<Group> {
    let manifest = cached_manifest_for_instance(state, instance_root);
    let counts = count_entries_by_content_type(state.results.entries.as_slice());
    let mut groups: Vec<Group> = Vec::new();
    let mut current = None;
    for entry in &state.results.entries {
        if current != Some(entry.content_type) {
            current = Some(entry.content_type);
            groups.push(Group {
                heading: format!("{} ({})", entry.content_type.label(), counts[entry.content_type.index()]),
                tiles: Vec::new(),
            });
        }
        if let Some(group) = groups.last_mut() {
            group.tiles.push(TileView::of(entry, &manifest, state));
        }
    }
    groups
}

pub fn page(state: &ContentBrowserState) -> (u32, bool) {
    (state.current_page.max(1), state.search_in_flight)
}

pub fn go_to_page(state: &mut ContentBrowserState, page: u32) {
    let page = page.clamp(1, 10_000);
    if page != state.current_page {
        state.current_page = page;
        request_search_for_current_filters(state, false);
    }
}

/// Queues the latest compatible version of result `key` for install.
pub fn quick_install(state: &mut ContentBrowserState, key: &str) {
    let Some(entry) = state.results.entries.iter().find(|entry| entry.dedupe_key == key).cloned() else { return };
    let active = state.active_downloads.iter().any(|active| active.dedupe_key == key);
    if active || state.download_queue.iter().any(|queued| queued_dedupe_key(&queued.request) == key) {
        return;
    }
    state.download_queue.push_back(QueuedContentDownload {
        request: ContentInstallRequest::Latest {
            entry: entry.clone(),
            game_version: state.minecraft_version_filter.clone(),
            loader: state.loader,
        },
    });
    state.status_message = Some(format!("Queued {} for download ({} in queue).", entry.name, state.download_queue.len()));
}

pub fn open_detail(state: &mut ContentBrowserState, key: &str) {
    if let Some(entry) = state.results.entries.iter().find(|entry| entry.dedupe_key == key).cloned() {
        open_detail_page(state, &entry);
    }
}

/// Identify a content file from its hash (the "Identify Content File" button).
pub fn identify(state: &mut ContentBrowserState, path: PathBuf) {
    request_identify_file(state, path);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    Installed,
    Switch,
    Download,
}

#[derive(Clone, Debug)]
pub struct VersionRow {
    pub id: String,
    pub name: String,
    pub source: &'static str,
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
    pub file_line: String,
    pub action: RowAction,
}

#[derive(Clone, Debug)]
pub struct DetailView {
    pub tile: TileView,
    pub body: String,
    /// 0 = Overview, 1 = Versions.
    pub tab: usize,
    pub loaders: Vec<String>,
    pub loader: usize,
    pub versions: Vec<String>,
    pub version: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub rows: Vec<VersionRow>,
}

pub fn detail(state: &mut ContentBrowserState, instance_root: &Path) -> Option<DetailView> {
    request_detail_versions(state);
    let entry = state.detail_entry.clone()?;
    let manifest = cached_manifest_for_instance(state, instance_root);
    let version = if state.detail_minecraft_version_filter.trim().is_empty() {
        0
    } else {
        state
            .available_game_versions
            .iter()
            .position(|version| version.id == state.detail_minecraft_version_filter)
            .map_or(0, |index| index + 1)
    };
    let rows = state
        .detail_versions
        .iter()
        .filter(|version| version_matches_loader(version, state.detail_loader_filter))
        .filter(|version| version_matches_game_version(version, state.detail_minecraft_version_filter.as_str()))
        .map(|version| {
            let action = match installed_project_for_entry(&manifest, &entry) {
                None => RowAction::Download,
                Some((_, installed))
                    if installed.selected_source == Some(version.source.into())
                        && installed.selected_version_id.as_deref() == Some(version.version_id.as_str()) =>
                {
                    RowAction::Installed
                }
                Some(_) => RowAction::Switch,
            };
            VersionRow {
                id: version.version_id.clone(),
                name: version.version_name.clone(),
                source: version.source.label(),
                loaders: version.loaders.clone(),
                game_versions: version.game_versions.iter().take(3).cloned().collect(),
                file_line: format!("{} | {}", version.file_name, version.published_at),
                action,
            }
        })
        .collect();
    Some(DetailView {
        tile: TileView::of(&entry, &manifest, state),
        body: if entry.summary.trim().is_empty() {
            "No description provided.".to_owned()
        } else {
            entry.summary.trim().to_owned()
        },
        tab: usize::from(state.detail_tab == ContentDetailTab::Versions),
        loaders: BrowserLoader::ALL.iter().map(|value| value.label().to_owned()).collect(),
        loader: BrowserLoader::ALL.iter().position(|value| *value == state.detail_loader_filter).unwrap_or(0),
        versions: state.game_version_options.clone(),
        version,
        loading: state.detail_versions_in_flight,
        error: state.detail_versions_error.clone(),
        rows,
    })
}

pub fn set_detail_tab(state: &mut ContentBrowserState, tab: usize) {
    state.detail_tab = if tab == 1 { ContentDetailTab::Versions } else { ContentDetailTab::Overview };
}

pub fn set_detail_filters(state: &mut ContentBrowserState, loader: usize, version: usize) {
    state.detail_loader_filter = BrowserLoader::ALL.get(loader).copied().unwrap_or(BrowserLoader::Any);
    state.detail_minecraft_version_filter = match version.checked_sub(1) {
        Some(index) => state.available_game_versions.get(index).map(|entry| entry.id.clone()).unwrap_or_default(),
        None => String::new(),
    };
}

/// Queues installing (or switching to) one specific version of the open project.
pub fn queue_version(state: &mut ContentBrowserState, version_id: &str, action: RowAction) {
    let Some(entry) = state.detail_entry.clone() else { return };
    let Some(version) = state.detail_versions.iter().find(|version| version.version_id == version_id).cloned() else { return };
    let already = |request: &ContentInstallRequest| {
        matches!(request, ContentInstallRequest::Exact { entry: queued, version: queued_version, .. }
            if queued.dedupe_key == entry.dedupe_key && queued_version.version_id == version.version_id)
    };
    if state.download_queue.iter().any(|queued| already(&queued.request)) {
        return;
    }
    let game_version = if state.detail_minecraft_version_filter.trim().is_empty() {
        state.minecraft_version_filter.clone()
    } else {
        state.detail_minecraft_version_filter.clone()
    };
    state.download_queue.push_back(QueuedContentDownload {
        request: ContentInstallRequest::Exact {
            entry: entry.clone(),
            version,
            game_version,
            loader: state.detail_loader_filter,
        },
    });
    state.status_message = Some(format!(
        "Queued {} {}.",
        match action {
            RowAction::Switch => "switch for",
            RowAction::Installed => "installed",
            RowAction::Download => "install for",
        },
        entry.name
    ));
}
