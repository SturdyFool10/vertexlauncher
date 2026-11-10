//! Toolkit-free view of the instance screen's installed-content list: the same scanning, lookups, filtering, paging
//! and actions the egui list runs, exposed as plain data and calls.

use vertex_i18n::tr;
use super::*;

pub const PAGE_SIZES: [usize; 4] = INSTALLED_CONTENT_PAGE_SIZES;

/// One installed file as the list shows it.
#[derive(Clone, Debug)]
pub struct ContentRow {
    pub lookup_key: String,
    pub file_path: PathBuf,
    pub disabled: bool,
    pub display_name: String,
    pub description: String,
    pub platform: String,
    pub version: Option<String>,
    pub warning: Option<String>,
    pub update_label: Option<String>,
    pub update_version_id: Option<String>,
    pub icon_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ContentView {
    pub kind: InstalledContentKind,
    pub kinds: Vec<InstalledContentKind>,
    /// A scan of this tab is still running.
    pub loading: bool,
    /// What to show instead of the list when it has no rows.
    pub empty_message: Option<String>,
    pub search: String,
    pub updatable_only: bool,
    pub page: usize,
    pub total_pages: usize,
    pub page_size: usize,
    pub count_label: String,
    /// (label, tooltip) of the "update all" button when some row has a known update.
    pub bulk_update: Option<(String, String)>,
    pub apply_in_flight: bool,
    pub rows: Vec<ContentRow>,
}

/// Polls the scans/lookups and builds what the list shows for the current tab, page and search.
pub fn view(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    config: &Config,
) -> ContentView {
    poll_installed_content_scan_results(state);
    poll_content_hash_cache_load_results(state);
    poll_content_hash_cache_save_results(state);
    poll_content_apply_results(state, instance_root);
    ensure_content_hash_cache_loaded(state, instance_root);

    let kind = state.selected_content_tab;
    let kinds = InstalledContentKind::ALL.to_vec();
    let mut view = ContentView {
        kind,
        kinds,
        loading: false,
        empty_message: None,
        search: state.installed_content_search_query.clone(),
        updatable_only: state.installed_content_updatable_only,
        page: state.installed_content_page,
        total_pages: 1,
        page_size: state.installed_content_page_size,
        count_label: String::new(),
        bulk_update: None,
        apply_in_flight: state.content_apply_in_flight,
        rows: Vec::new(),
    };

    let files = installed_content_files_for_tab(state, instance_root, kind);
    // A rescan of a list already on screen keeps showing it; only a first scan has nothing to show but "loading".
    if files.is_empty() && state.installed_content_cache.scans_in_flight.contains(&kind) {
        view.loading = true;
        view.empty_message = Some(tr!("Loading installed {kind}...", kind = kind.label().to_lowercase()).into_owned());
        return view;
    }
    if files.is_empty() {
        view.empty_message = Some(tr!("No {kind} installed.", kind = kind.label().to_lowercase()).into_owned());
        return view;
    }

    let filtered = filter_installed_content_indices(state, files.as_ref());
    let total_items = filtered.as_ref().map_or(files.len(), Vec::len);
    view.count_label = installed_content_count_label(
        total_items,
        files.len(),
        !state.installed_content_search_query.trim().is_empty() || state.installed_content_updatable_only,
    );
    if total_items == 0 {
        let query = state.installed_content_search_query.trim();
        view.empty_message = Some(if query.is_empty() && state.installed_content_updatable_only {
            tr!("No {kind} have an update available.", kind = kind.label().to_lowercase()).into_owned()
        } else if query.is_empty() {
            tr!("No {kind} installed.", kind = kind.label().to_lowercase()).into_owned()
        } else {
            tr!("No {kind} match '{query}'.", kind = kind.label().to_lowercase(), query = query).into_owned()
        });
        return view;
    }

    let page_size = if INSTALLED_CONTENT_PAGE_SIZES.contains(&state.installed_content_page_size) {
        state.installed_content_page_size
    } else {
        INSTALLED_CONTENT_PAGE_SIZES[1]
    };
    state.installed_content_page_size = page_size;
    let total_pages = total_items.div_ceil(page_size).max(1);
    state.installed_content_page = state.installed_content_page.clamp(1, total_pages);
    view.page = state.installed_content_page;
    view.page_size = page_size;
    view.total_pages = total_pages;

    let game_version = selected_game_version(state).to_owned();
    let modloader = selected_modloader_value(state);
    prefetch_bulk_update_metadata(state, files.as_ref(), kind, game_version.as_str(), modloader.as_str());
    if tab_has_known_available_update(state, files.as_ref()) {
        view.bulk_update = Some((bulk_update_button_label(kind), bulk_update_button_tooltip(kind)));
    }

    let start = (state.installed_content_page - 1) * page_size;
    let end = (start + page_size).min(total_items);
    let visible: Vec<InstalledContentFile> = match filtered.as_ref() {
        Some(indices) => indices[start..end].iter().map(|&index| files[index].clone()).collect(),
        None => files[start..end].to_vec(),
    };
    let _ = request_content_metadata_lookup_batch(
        state,
        &visible,
        kind,
        game_version.as_str(),
        modloader.as_str(),
        false,
        CONTENT_LOOKUP_BATCH_SIZE,
    );
    for entry in &visible {
        let metadata = state.content_metadata_cache.get(&entry.lookup_key).and_then(|meta| meta.as_ref());
        let display = InstalledEntryDisplay::new(entry, metadata);
        view.rows.push(ContentRow {
            lookup_key: entry.lookup_key.clone(),
            file_path: entry.file_path.clone(),
            disabled: entry.disabled,
            display_name: display.display_name,
            description: display.description,
            platform: display.platform_label,
            version: display.version_label,
            warning: display.warning_message,
            update_label: display.update_label,
            update_version_id: display.update_version_id,
            icon_url: display.icon_url,
        });
    }
    flush_content_hash_cache(state, instance_root);
    let _ = config;
    view
}

pub fn select_tab(state: &mut InstanceScreenState, kind: InstalledContentKind) {
    if state.selected_content_tab != kind {
        state.selected_content_tab = kind;
        state.installed_content_page = 1;
    }
}

pub fn set_search(state: &mut InstanceScreenState, query: &str) {
    if state.installed_content_search_query != query {
        state.installed_content_search_query = query.to_owned();
        state.installed_content_page = 1;
    }
}

pub fn set_updatable_only(state: &mut InstanceScreenState, updatable_only: bool) {
    if state.installed_content_updatable_only != updatable_only {
        state.installed_content_updatable_only = updatable_only;
        state.installed_content_page = 1;
    }
}

pub fn set_page(state: &mut InstanceScreenState, page: usize) {
    state.installed_content_page = page.max(1);
}

pub fn set_page_size(state: &mut InstanceScreenState, size: usize) {
    if INSTALLED_CONTENT_PAGE_SIZES.contains(&size) {
        state.installed_content_page_size = size;
        state.installed_content_page = 1;
    }
}

pub fn delete(state: &mut InstanceScreenState, instance_root: &Path, row: &ContentRow) {
    request_content_delete(state, instance_root, state.selected_content_tab, row.lookup_key.as_str(), row.file_path.as_path());
}

pub fn toggle(state: &mut InstanceScreenState, row: &ContentRow) {
    request_content_toggle(state, state.selected_content_tab, row.lookup_key.as_str(), row.file_path.as_path(), row.disabled);
}

pub fn update(state: &mut InstanceScreenState, instance_root: &Path, row: &ContentRow) {
    let Some(version_id) = row.update_version_id.as_deref() else { return };
    let Some(metadata) = state.content_metadata_cache.get(&row.lookup_key).and_then(|meta| meta.clone()) else { return };
    let game_version = selected_game_version(state).to_owned();
    let modloader = selected_modloader_value(state);
    request_content_update(
        state,
        instance_root,
        row.lookup_key.as_str(),
        metadata.entry,
        row.file_path.as_path(),
        version_id,
        game_version.as_str(),
        modloader.as_str(),
    );
}

pub fn bulk_update(state: &mut InstanceScreenState, instance_root: &Path, config: &Config) {
    let policy = DownloadPolicy {
        max_concurrent_downloads: config.download_max_concurrent().max(1),
        max_download_bps: config.parsed_download_speed_limit_bps(),
    };
    let game_version = selected_game_version(state).to_owned();
    let modloader = selected_modloader_value(state);
    request_bulk_content_update(
        state,
        instance_root,
        state.selected_content_tab,
        game_version.as_str(),
        modloader.as_str(),
        &policy,
    );
}

pub fn add_local_files(state: &mut InstanceScreenState, instance_root: &Path, paths: Vec<PathBuf>) {
    if !paths.is_empty() {
        request_local_content_import(state, instance_root, paths);
    }
}

pub fn apply_patch(state: &mut InstanceScreenState, instance_root: &Path, path: PathBuf) {
    request_vtmpatch_apply(state, instance_root, path);
}

pub fn refresh(state: &mut InstanceScreenState, instance_root: &Path) {
    refresh_installed_content_state(state, instance_root);
    state.status_message = Some(tr!("Refreshed installed content metadata and hash cache.").into_owned());
}

/// Opens the Content Browser's detail page for an installed row, once its metadata has resolved. Returns an error
/// message to show when that is not possible yet.
pub fn open_detail(state: &mut InstanceScreenState, row: &ContentRow, locked: bool) -> Result<(), String> {
    if locked {
        return Err(tr!("Content Browser is unavailable while instance prep is running.").into_owned());
    }
    match state.content_metadata_cache.get(&row.lookup_key).and_then(|meta| meta.clone()) {
        Some(metadata) => {
            crate::screens::content_browser::request_open_detail_for_content(metadata.entry);
            Ok(())
        }
        None => Err(tr!("Still loading content metadata. Try again in a moment.").into_owned()),
    }
}
