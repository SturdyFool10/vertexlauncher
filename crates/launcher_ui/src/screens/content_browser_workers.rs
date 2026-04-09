use super::*;

pub(super) fn ensure_search_channel(state: &mut ContentBrowserState) {
    if state.search_tx.is_some() && state.search_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<SearchUpdate>();
    state.search_tx = Some(tx);
    state.search_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn ensure_detail_versions_channel(state: &mut ContentBrowserState) {
    if state.detail_versions_tx.is_some() && state.detail_versions_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<DetailVersionsResult>();
    state.detail_versions_tx = Some(tx);
    state.detail_versions_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn request_detail_versions(state: &mut ContentBrowserState) {
    let Some(entry) = state.detail_entry.clone() else {
        return;
    };
    if let Some(cached) = state
        .detail_versions_cache
        .get(entry.dedupe_key.as_str())
        .cloned()
    {
        state.detail_versions_in_flight = false;
        state.detail_versions_project_key = Some(entry.dedupe_key.clone());
        match cached {
            Ok(versions) => {
                state.detail_versions = versions;
                state.detail_versions_error = None;
            }
            Err(error) => {
                state.detail_versions.clear();
                state.detail_versions_error = Some(error);
            }
        }
        return;
    }
    if state.detail_versions_in_flight {
        return;
    }
    if state.detail_versions_project_key.as_deref() == Some(entry.dedupe_key.as_str())
        && (!state.detail_versions.is_empty() || state.detail_versions_error.is_some())
    {
        return;
    }

    ensure_detail_versions_channel(state);
    let Some(tx) = state.detail_versions_tx.as_ref().cloned() else {
        return;
    };

    state.detail_versions_in_flight = true;
    state.detail_versions_error = None;
    let project_key = entry.dedupe_key.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let versions: Result<Vec<BrowserVersionEntry>, String> = match tokio::time::timeout(
            DETAIL_VERSIONS_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || fetch_versions_for_entry(&entry)),
        )
        .await
        {
            Ok(join_result) => join_result
                .map_err(|err| err.to_string())
                .and_then(|result| result),
            Err(_) => Err(format!(
                "detail version request timed out after {}s",
                DETAIL_VERSIONS_FETCH_TIMEOUT.as_secs()
            )),
        };
        if let Err(err) = tx.send(DetailVersionsResult {
            project_key,
            versions,
        }) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                error = %err,
                "Failed to deliver content detail-version result."
            );
        }
    });
}

pub(super) fn request_version_catalog(state: &mut ContentBrowserState) {
    if state.version_catalog_in_flight
        || !state.available_game_versions.is_empty()
        || state.version_catalog_error.is_some()
    {
        return;
    }

    ensure_version_catalog_channel(state);
    let Some(tx) = state.version_catalog_tx.as_ref().cloned() else {
        return;
    };

    state.version_catalog_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result: Result<Vec<MinecraftVersionEntry>, String> = match tokio::time::timeout(
            VERSION_CATALOG_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || {
                fetch_version_catalog(false)
                    .map(|catalog| catalog.game_versions)
                    .map_err(|err| err.to_string())
            }),
        )
        .await
        {
            Ok(join_result) => join_result
                .map_err(|err| err.to_string())
                .and_then(|result| result),
            Err(_) => Err(format!(
                "version catalog request timed out after {}s",
                VERSION_CATALOG_FETCH_TIMEOUT.as_secs()
            )),
        };
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                error = %err,
                "Failed to deliver content version catalog result."
            );
        }
    });
}

pub(super) fn apply_pending_external_detail_open(state: &mut ContentBrowserState) {
    let Some(store) = PENDING_EXTERNAL_DETAIL_OPEN.get() else {
        return;
    };
    let Ok(mut pending) = store.lock() else {
        tracing::error!(
            target: "vertexlauncher/content_browser",
            "Content browser pending external detail-open store mutex was poisoned."
        );
        return;
    };
    let Some(entry) = pending.take() else {
        return;
    };

    let Ok(browser_entry) = browser_entry_from_unified_content(&entry) else {
        return;
    };

    open_detail_page(state, &browser_entry);
}

pub(super) fn browser_entry_from_unified_content(
    entry: &UnifiedContentEntry,
) -> Result<BrowserProjectEntry, String> {
    let Some(content_type) = parse_content_type(entry.content_type.as_str()) else {
        return Err(format!("Unsupported content type for {}.", entry.name));
    };
    let name_key = normalize_search_key(entry.name.as_str());
    if name_key.is_empty() {
        return Err("Content entry name cannot be empty.".to_owned());
    }

    let mut browser_entry = BrowserProjectEntry {
        dedupe_key: format!("{}::{name_key}", content_type.label().to_ascii_lowercase()),
        name: entry.name.clone(),
        summary: entry.summary.clone(),
        content_type,
        icon_url: entry.icon_url.clone(),
        modrinth_project_id: None,
        curseforge_project_id: None,
        sources: vec![entry.source],
        popularity_score: None,
        updated_at: None,
        relevance_rank: 0,
    };

    match entry.source {
        ContentSource::Modrinth => {
            browser_entry.modrinth_project_id = entry
                .id
                .strip_prefix("modrinth:")
                .map(str::to_owned)
                .or_else(|| (!entry.id.trim().is_empty()).then(|| entry.id.clone()));
        }
        ContentSource::CurseForge => {
            browser_entry.curseforge_project_id = entry
                .id
                .strip_prefix("curseforge:")
                .or_else(|| (!entry.id.trim().is_empty()).then_some(entry.id.as_str()))
                .and_then(|value| value.parse::<u64>().ok());
        }
    }

    Ok(browser_entry)
}

fn ensure_version_catalog_channel(state: &mut ContentBrowserState) {
    if state.version_catalog_tx.is_some() && state.version_catalog_rx.is_some() {
        return;
    }

    let (tx, rx) = mpsc::channel::<Result<Vec<MinecraftVersionEntry>, String>>();
    state.version_catalog_tx = Some(tx);
    state.version_catalog_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn poll_version_catalog(state: &mut ContentBrowserState) {
    let mut should_reset_channel = false;
    let mut updates = Vec::new();

    if let Some(rx) = state.version_catalog_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/content_browser",
                            "Content-browser version catalog worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/content_browser",
                    "Content-browser version catalog receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.version_catalog_tx = None;
        state.version_catalog_rx = None;
        state.version_catalog_in_flight = false;
        state.version_catalog_error =
            Some("Version catalog worker stopped unexpectedly.".to_owned());
    }

    for update in updates {
        state.version_catalog_in_flight = false;
        match update {
            Ok(versions) => {
                state.available_game_versions = versions;
                state.version_catalog_error = None;
            }
            Err(err) => {
                state.version_catalog_error = Some(err);
            }
        }
    }
}

fn ensure_identify_channel(state: &mut ContentBrowserState) {
    if state.identify_tx.is_some() && state.identify_rx.is_some() {
        return;
    }

    let (tx, rx) = mpsc::channel::<(PathBuf, Result<UnifiedContentEntry, String>)>();
    state.identify_tx = Some(tx);
    state.identify_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn request_identify_file(state: &mut ContentBrowserState, selected_path: PathBuf) {
    if state.identify_in_flight {
        return;
    }
    if detect_installed_content_kind(selected_path.as_path()).is_none() {
        state.status_message = Some(format!(
            "Unsupported content file: {}. Expected a mod .jar or supported pack .zip.",
            selected_path.display()
        ));
        return;
    }

    ensure_identify_channel(state);
    let Some(tx) = state.identify_tx.as_ref().cloned() else {
        return;
    };

    state.identify_in_flight = true;
    state.status_message = Some(format!(
        "Identifying {} in the background...",
        selected_path.display()
    ));
    let _ = tokio_runtime::spawn_detached(async move {
        let path_for_result = selected_path.clone();
        let join = tokio_runtime::spawn_blocking(move || {
            identify_mod_file_by_hash(selected_path.as_path())
        });
        let result = match join.await {
            Ok(r) => r,
            Err(err) => Err(format!("content identification worker panicked: {err}")),
        };
        if let Err(err) = tx.send((path_for_result.clone(), result)) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                path = %path_for_result.display(),
                error = %err,
                "Failed to deliver content identification result."
            );
        }
    });
}

pub(super) fn poll_identify_results(state: &mut ContentBrowserState) {
    let Some(rx) = state.identify_rx.as_ref() else {
        return;
    };

    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    match rx.lock() {
        Ok(receiver) => loop {
            match receiver.try_recv() {
                Ok(update) => updates.push(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::error!(
                        target: "vertexlauncher/content_browser",
                        "Content identification worker disconnected unexpectedly."
                    );
                    should_reset_channel = true;
                    break;
                }
            }
        },
        Err(_) => {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                "Content identification receiver mutex was poisoned."
            );
            should_reset_channel = true;
        }
    }

    if should_reset_channel {
        state.identify_tx = None;
        state.identify_rx = None;
        state.identify_in_flight = false;
        state.status_message =
            Some("Content identification worker stopped unexpectedly.".to_owned());
    }

    for (path, result) in updates {
        state.identify_in_flight = false;
        match result {
            Ok(entry) => {
                let project_name = entry.name.clone();
                request_open_detail_for_content(entry);
                apply_pending_external_detail_open(state);
                state.status_message = Some(format!(
                    "Identified {} from {}.",
                    project_name,
                    path.display()
                ));
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/content_browser",
                    path = %path.display(),
                    error = %err,
                    "Content identification failed."
                );
                state.status_message =
                    Some(format!("Could not identify {}: {err}", path.display()));
            }
        }
    }
}

#[path = "content_browser_workers/provider_search_entry.rs"]
mod provider_search_entry;
#[path = "content_browser_workers/search_task_outcome.rs"]
mod search_task_outcome;

use self::provider_search_entry::ProviderSearchEntry;
use self::search_task_outcome::SearchTaskOutcome;

pub(super) fn request_search(state: &mut ContentBrowserState, request: BrowserSearchRequest) {
    if state.search_in_flight {
        return;
    }
    let total_tasks = content_scope_task_count(request.content_scope);
    if let Some(cached) = state.search_cache.get(&request).cloned() {
        state.query_input = request.query.clone().unwrap_or_default();
        state.active_search_request = Some(request);
        state.search_completed_tasks = total_tasks;
        state.search_total_tasks = total_tasks;
        state.results = cached;
        trim_content_browser_search_cache(state);
        return;
    }

    ensure_search_channel(state);
    let Some(tx) = state.search_tx.as_ref().cloned() else {
        return;
    };

    state.active_search_request = Some(request.clone());
    state.search_completed_tasks = 0;
    state.search_total_tasks = total_tasks;
    state.results = BrowserSearchSnapshot::default();
    state.search_in_flight = true;
    state.search_notification_active = true;
    notification::progress!(
        notification::Severity::Info,
        "content-browser/search",
        0.1f32,
        "Searching content..."
    );
    let request_for_failure = request.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let worker_tx = tx.clone();
        let join = tokio_runtime::spawn_blocking(move || run_search_request(request, worker_tx));
        let result = match join.await {
            Ok(r) => r,
            Err(err) => Err(format!("content search worker panicked: {err}")),
        };
        match result {
            Ok(()) => {}
            Err(err) => {
                if let Err(err_send) = tx.send(SearchUpdate::Failed {
                    request: request_for_failure,
                    error: err,
                }) {
                    tracing::error!(
                        target: "vertexlauncher/content_browser",
                        error = %err_send,
                        "Failed to deliver content search failure update."
                    );
                }
            }
        }
    });
}

fn fetch_versions_for_entry(
    entry: &BrowserProjectEntry,
) -> Result<Vec<BrowserVersionEntry>, String> {
    let modrinth = ModrinthClient::default();
    let curseforge = CurseForgeClient::from_env();
    let mut versions = Vec::new();

    if let Some(project_id) = entry.modrinth_project_id.as_deref() {
        let project_versions = modrinth
            .list_project_versions(project_id, &[], &[])
            .map_err(|err| format!("Modrinth versions failed for {project_id}: {err}"))?;
        let dependency_version_projects = modrinth_dependency_project_ids(
            &modrinth,
            project_versions
                .iter()
                .flat_map(|version| version.dependencies.iter().cloned())
                .collect::<Vec<_>>()
                .as_slice(),
        );
        for version in project_versions {
            let Some(file) = version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())
            else {
                continue;
            };
            let dependencies = modrinth_dependency_refs(
                version.dependencies.as_slice(),
                &dependency_version_projects,
            );
            versions.push(BrowserVersionEntry {
                source: ManagedContentSource::Modrinth,
                version_id: version.id,
                version_name: version.version_number,
                file_name: file.filename.clone(),
                file_url: file.url.clone(),
                published_at: version.date_published,
                loaders: version.loaders,
                game_versions: version.game_versions,
                dependencies,
            });
        }
    }

    if let Some(curseforge_project_id) = entry.curseforge_project_id
        && let Some(curseforge) = curseforge.as_ref()
    {
        let files = fetch_curseforge_versions(curseforge, curseforge_project_id)?;
        for file in files {
            let Some(download_url) = file.download_url else {
                continue;
            };
            let mut dependencies = Vec::new();
            for dep in file.dependencies {
                if dep.relation_type == CONTENT_DOWNLOAD_REQUIRED_DEPENDENCY_RELATION_TYPE {
                    dependencies.push(DependencyRef::CurseForgeProject(dep.mod_id));
                }
            }
            let (loaders, game_versions) = split_curseforge_game_versions(file.game_versions);
            versions.push(BrowserVersionEntry {
                source: ManagedContentSource::CurseForge,
                version_id: file.id.to_string(),
                version_name: file.display_name.clone(),
                file_name: file.file_name,
                file_url: download_url,
                published_at: file.file_date,
                loaders,
                game_versions,
                dependencies,
            });
        }
    }

    versions.sort_by(|left, right| {
        right
            .published_at
            .cmp(&left.published_at)
            .then_with(|| left.version_name.cmp(&right.version_name))
    });
    Ok(versions)
}

pub(super) fn fetch_exact_version_for_entry(
    entry: &BrowserProjectEntry,
    source: ManagedContentSource,
    version_id: &str,
) -> Result<BrowserVersionEntry, String> {
    match source {
        ManagedContentSource::Modrinth => {
            let modrinth = ModrinthClient::default();
            let version = modrinth
                .get_version(version_id)
                .map_err(|err| format!("Modrinth version lookup failed for {version_id}: {err}"))?;
            let file = version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())
                .ok_or_else(|| {
                    format!("No downloadable file found for Modrinth version {version_id}.")
                })?;
            let dependency_version_projects =
                modrinth_dependency_project_ids(&modrinth, version.dependencies.as_slice());
            let dependencies = modrinth_dependency_refs(
                version.dependencies.as_slice(),
                &dependency_version_projects,
            );
            Ok(BrowserVersionEntry {
                source,
                version_id: version.id,
                version_name: version.version_number,
                file_name: file.filename.clone(),
                file_url: file.url.clone(),
                published_at: version.date_published,
                loaders: version.loaders,
                game_versions: version.game_versions,
                dependencies,
            })
        }
        ManagedContentSource::CurseForge => {
            let curseforge = CurseForgeClient::from_env()
                .ok_or_else(|| "CurseForge API key missing.".to_owned())?;
            let version_id_u64 = version_id
                .trim()
                .parse::<u64>()
                .map_err(|err| format!("Invalid CurseForge version id {version_id}: {err}"))?;
            let file = curseforge
                .get_files(&[version_id_u64])
                .map_err(|err| {
                    format!("CurseForge version lookup failed for {version_id_u64}: {err}")
                })?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    format!(
                        "Could not find CurseForge version {} for {}.",
                        version_id, entry.name
                    )
                })?;
            let download_url = file
                .download_url
                .clone()
                .ok_or_else(|| format!("CurseForge version {} has no download URL.", version_id))?;
            let mut dependencies = Vec::new();
            for dep in file.dependencies {
                if dep.relation_type == CONTENT_DOWNLOAD_REQUIRED_DEPENDENCY_RELATION_TYPE {
                    dependencies.push(DependencyRef::CurseForgeProject(dep.mod_id));
                }
            }
            let (loaders, game_versions) = split_curseforge_game_versions(file.game_versions);
            Ok(BrowserVersionEntry {
                source,
                version_id: file.id.to_string(),
                version_name: file.display_name.clone(),
                file_name: file.file_name,
                file_url: download_url,
                published_at: file.file_date,
                loaders,
                game_versions,
                dependencies,
            })
        }
    }
}

fn fetch_curseforge_versions(
    client: &CurseForgeClient,
    project_id: u64,
) -> Result<Vec<curseforge::File>, String> {
    let mut index = 0u32;
    let mut files = Vec::new();
    for _ in 0..DETAIL_VERSION_FETCH_MAX_PAGES {
        let batch = client
            .list_mod_files(
                project_id,
                None,
                None,
                index,
                DETAIL_VERSION_FETCH_PAGE_SIZE,
            )
            .map_err(|err| format!("CurseForge files failed for {project_id}: {err}"))?;
        let batch_len = batch.len() as u32;
        files.extend(batch);
        if batch_len < DETAIL_VERSION_FETCH_PAGE_SIZE {
            break;
        }
        index = index.saturating_add(DETAIL_VERSION_FETCH_PAGE_SIZE);
    }
    Ok(files)
}

fn split_curseforge_game_versions(values: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut loaders = Vec::new();
    let mut game_versions = Vec::new();
    for value in values {
        let normalized = value.trim().to_ascii_lowercase();
        if matches!(
            normalized.as_str(),
            "fabric" | "forge" | "neoforge" | "quilt"
        ) {
            loaders.push(value);
        } else if value.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
            game_versions.push(value);
        }
    }
    (loaders, game_versions)
}

fn run_search_request(
    request: BrowserSearchRequest,
    tx: mpsc::Sender<SearchUpdate>,
) -> Result<(), String> {
    let query = request
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let game_version = request
        .game_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let modrinth = ModrinthClient::default();
    let curseforge = CurseForgeClient::from_env();

    let mut warnings = Vec::new();
    let curseforge_class_ids = if let Some(client) = curseforge.as_ref() {
        resolve_curseforge_class_ids_cached(client, &mut warnings)
    } else {
        warnings.push(
            "CurseForge API key missing (set VERTEX_CURSEFORGE_API_KEY or CURSEFORGE_API_KEY). Showing Modrinth results only."
                .to_owned(),
        );
        HashMap::new()
    };

    let page = request.page.max(1);
    let provider_offset = page
        .saturating_sub(1)
        .saturating_mul(CONTENT_SEARCH_PER_PROVIDER_LIMIT);
    let total_tasks = content_scope_task_count(request.content_scope);
    if total_tasks == 0 {
        if let Err(err) = tx.send(SearchUpdate::Snapshot {
            request,
            snapshot: BrowserSearchSnapshot {
                entries: Vec::new(),
                warnings,
            },
            completed_tasks: 0,
            total_tasks: 0,
            finished: true,
        }) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                error = %err,
                "Failed to deliver empty content search snapshot."
            );
        }
        return Ok(());
    }

    let mut provider_entries = Vec::new();
    let outcomes = thread::scope(|scope| -> Result<Vec<SearchTaskOutcome>, String> {
        let mut tasks = Vec::new();
        for content_type in BrowserContentType::ORDERED {
            if !request.content_scope.includes(content_type) {
                continue;
            }
            let query_for_type = query
                .clone()
                .unwrap_or_else(|| content_type.default_discovery_query().to_owned());
            let game_version = game_version.clone();
            let modrinth = modrinth.clone();
            let curseforge = curseforge.clone();
            let curseforge_class_id = curseforge_class_ids.get(&content_type).copied();
            let loader = request.loader;
            tasks.push((
                content_type,
                scope.spawn(move || {
                    search_content_type_providers(
                        content_type,
                        query_for_type,
                        game_version,
                        provider_offset,
                        loader,
                        modrinth,
                        curseforge,
                        curseforge_class_id,
                    )
                }),
            ));
        }

        let mut outcomes = Vec::with_capacity(tasks.len());
        for (content_type, task) in tasks {
            outcomes.push(task.join().map_err(|_| {
                format!(
                    "{} search worker panicked unexpectedly.",
                    content_type.label()
                )
            })?);
        }
        Ok(outcomes)
    })?;

    let mut completed_tasks = 0usize;
    for outcome in outcomes {
        completed_tasks = completed_tasks.saturating_add(1);
        provider_entries.extend(outcome.entries);
        warnings.extend(outcome.warnings);
        if let Err(err) = tx.send(SearchUpdate::Snapshot {
            request: request.clone(),
            snapshot: build_search_snapshot(
                provider_entries.as_slice(),
                warnings.as_slice(),
                request.mod_sort_mode,
            ),
            completed_tasks,
            total_tasks,
            finished: completed_tasks >= total_tasks,
        }) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                completed_tasks,
                total_tasks,
                error = %err,
                "Failed to deliver incremental content search snapshot."
            );
        }
    }

    Ok(())
}

fn search_content_type_providers(
    content_type: BrowserContentType,
    query_for_type: String,
    game_version: Option<String>,
    provider_offset: u32,
    loader: BrowserLoader,
    modrinth: ModrinthClient,
    curseforge: Option<CurseForgeClient>,
    curseforge_class_id: Option<u32>,
) -> SearchTaskOutcome {
    let mut outcome = SearchTaskOutcome::default();
    let mod_loader = if content_type == BrowserContentType::Mod {
        loader.modrinth_slug()
    } else {
        None
    };

    match modrinth.search_projects_with_filters(
        query_for_type.as_str(),
        CONTENT_SEARCH_PER_PROVIDER_LIMIT,
        provider_offset,
        Some(content_type.modrinth_project_type()),
        game_version.as_deref(),
        mod_loader,
        None,
    ) {
        Ok(entries) => {
            outcome
                .entries
                .extend(
                    entries
                        .into_iter()
                        .enumerate()
                        .map(|(idx, entry)| ProviderSearchEntry {
                            name: entry.title,
                            summary: entry.description,
                            content_type,
                            source: ContentSource::Modrinth,
                            modrinth_project_id: Some(entry.project_id),
                            curseforge_project_id: None,
                            icon_url: entry.icon_url,
                            popularity_score: Some(entry.downloads),
                            updated_at: entry.date_modified,
                            relevance_rank: idx as u32,
                        }),
                );
        }
        Err(err) => outcome.warnings.push(format!(
            "Modrinth search failed for {}: {err}",
            content_type.label()
        )),
    }

    let Some(curseforge) = curseforge.as_ref() else {
        return outcome;
    };
    let Some(class_id) = curseforge_class_id else {
        return outcome;
    };
    let mod_loader_type = if content_type == BrowserContentType::Mod {
        loader.curseforge_mod_loader_type()
    } else {
        None
    };

    match curseforge.search_projects_with_filters(
        MINECRAFT_GAME_ID,
        query_for_type.as_str(),
        provider_offset,
        CONTENT_SEARCH_PER_PROVIDER_LIMIT,
        Some(class_id),
        game_version.as_deref(),
        mod_loader_type,
        None,
    ) {
        Ok(entries) => {
            outcome
                .entries
                .extend(
                    entries
                        .into_iter()
                        .enumerate()
                        .map(|(idx, entry)| ProviderSearchEntry {
                            name: entry.name,
                            summary: entry.summary,
                            content_type,
                            source: ContentSource::CurseForge,
                            modrinth_project_id: None,
                            curseforge_project_id: Some(entry.id),
                            icon_url: entry.icon_url,
                            popularity_score: Some(entry.download_count),
                            updated_at: entry.date_modified,
                            relevance_rank: idx as u32,
                        }),
                );
        }
        Err(err) => outcome.warnings.push(format!(
            "CurseForge search failed for {}: {err}",
            content_type.label()
        )),
    }

    outcome
}

fn resolve_curseforge_class_ids_cached(
    client: &CurseForgeClient,
    warnings: &mut Vec<String>,
) -> HashMap<BrowserContentType, u32> {
    static CACHE: OnceLock<Mutex<Option<HashMap<BrowserContentType, u32>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(cache) = cache.lock()
        && let Some(class_ids) = cache.as_ref()
    {
        return class_ids.clone();
    }

    let class_ids = resolve_curseforge_class_ids(client, warnings);
    if let Ok(mut cache) = cache.lock() {
        *cache = Some(class_ids.clone());
    }
    class_ids
}

fn resolve_curseforge_class_ids(
    client: &CurseForgeClient,
    warnings: &mut Vec<String>,
) -> HashMap<BrowserContentType, u32> {
    let mut by_type = HashMap::new();
    match client.list_content_classes(MINECRAFT_GAME_ID) {
        Ok(classes) => {
            for class_entry in classes {
                let normalized = normalize_search_key(class_entry.name.as_str());
                if normalized.contains("shader") {
                    by_type.insert(BrowserContentType::Shader, class_entry.id);
                } else if normalized.contains("resource")
                    || normalized.contains("texture pack")
                    || normalized.contains("texture")
                {
                    by_type.insert(BrowserContentType::ResourcePack, class_entry.id);
                } else if normalized.contains("data pack") || normalized.contains("datapack") {
                    by_type.insert(BrowserContentType::DataPack, class_entry.id);
                } else if normalized.contains("mod") {
                    by_type.insert(BrowserContentType::Mod, class_entry.id);
                }
            }
        }
        Err(err) => warnings.push(format!("CurseForge class discovery failed: {err}")),
    }
    by_type.entry(BrowserContentType::Mod).or_insert(6);
    by_type
}

pub(super) fn poll_search(state: &mut ContentBrowserState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.search_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/content_browser",
                            request = ?state.active_search_request,
                            "Content search worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/content_browser",
                    request = ?state.active_search_request,
                    "Content search receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.search_tx = None;
        state.search_rx = None;
        state.search_in_flight = false;
        state.search_completed_tasks = 0;
        state.search_total_tasks = 0;
        state
            .results
            .warnings
            .push("Content search worker stopped unexpectedly.".to_owned());
    }

    for update in updates {
        match update {
            SearchUpdate::Snapshot {
                request,
                snapshot,
                completed_tasks,
                total_tasks,
                finished,
            } => {
                if state.active_search_request.as_ref() != Some(&request) {
                    continue;
                }
                state.results = snapshot.clone();
                state.search_completed_tasks = completed_tasks;
                state.search_total_tasks = total_tasks;
                if finished {
                    state.search_in_flight = false;
                    if state.search_notification_active {
                        state.search_notification_active = false;
                    }
                    state.search_cache.insert(request, snapshot);
                    trim_content_browser_search_cache(state);
                    notification::progress!(
                        notification::Severity::Info,
                        "content-browser/search",
                        1.0f32,
                        "Content search complete."
                    );
                } else {
                    let progress = if total_tasks == 0 {
                        0.5
                    } else {
                        0.1f32 + (0.8f32 * (completed_tasks as f32 / total_tasks as f32))
                    };
                    notification::progress!(
                        notification::Severity::Info,
                        "content-browser/search",
                        progress.min(0.95),
                        "Searching content... ({}/{})",
                        completed_tasks,
                        total_tasks
                    );
                }
            }
            SearchUpdate::Failed { request, error } => {
                if state.active_search_request.as_ref() != Some(&request) {
                    continue;
                }
                state.search_in_flight = false;
                state.search_completed_tasks = 0;
                state.search_total_tasks = 0;
                if state.search_notification_active {
                    state.search_notification_active = false;
                }
                tracing::warn!(
                    target: "vertexlauncher/content_browser",
                    request = ?request,
                    error = %error,
                    "Content search failed."
                );
                state.results.warnings.push(error.clone());
                notification::warn!("content-browser/search", "Content search failed: {}", error);
            }
        }
    }
}

pub(super) fn poll_detail_versions(state: &mut ContentBrowserState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.detail_versions_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/content_browser",
                            project = ?state.detail_versions_project_key,
                            "Detail-versions worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/content_browser",
                    project = ?state.detail_versions_project_key,
                    "Detail-versions receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.detail_versions_tx = None;
        state.detail_versions_rx = None;
        state.detail_versions_in_flight = false;
        state.detail_versions_error =
            Some("Version details worker stopped unexpectedly.".to_owned());
    }

    for update in updates {
        state.detail_versions_in_flight = false;
        state
            .detail_versions_cache
            .insert(update.project_key.clone(), update.versions.clone());
        if state
            .detail_entry
            .as_ref()
            .is_some_and(|entry| entry.dedupe_key == update.project_key)
        {
            match update.versions {
                Ok(versions) => {
                    state.detail_versions_project_key = Some(update.project_key);
                    state.detail_versions = versions;
                    state.detail_versions_error = None;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "vertexlauncher/content_browser",
                        project = %update.project_key,
                        error = %err,
                        "Detail version lookup failed."
                    );
                    state.detail_versions_project_key = Some(update.project_key);
                    state.detail_versions.clear();
                    state.detail_versions_error = Some(err);
                }
            }
        }
    }
}

fn dedupe_browser_entries(entries: Vec<ProviderSearchEntry>) -> Vec<BrowserProjectEntry> {
    let mut by_key = HashMap::<String, BrowserProjectEntry>::new();
    for entry in entries {
        let ProviderSearchEntry {
            name,
            summary,
            content_type,
            source,
            modrinth_project_id,
            curseforge_project_id,
            icon_url,
            popularity_score,
            updated_at,
            relevance_rank,
        } = entry;
        let name_key = normalize_search_key(name.as_str());
        if name_key.is_empty() {
            continue;
        }
        let dedupe_key = format!("{}::{name_key}", content_type.label().to_ascii_lowercase());

        let merged = by_key
            .entry(dedupe_key.clone())
            .or_insert_with(|| BrowserProjectEntry {
                dedupe_key: dedupe_key.clone(),
                name: name.clone(),
                summary: summary.clone(),
                content_type,
                icon_url: icon_url.clone(),
                modrinth_project_id: modrinth_project_id.clone(),
                curseforge_project_id,
                sources: Vec::new(),
                popularity_score,
                updated_at: updated_at.clone(),
                relevance_rank,
            });
        if merged.summary.trim().len() < summary.trim().len() {
            merged.summary = summary;
        }
        if merged.icon_url.is_none() {
            merged.icon_url = icon_url;
        }
        if merged.modrinth_project_id.is_none() {
            merged.modrinth_project_id = modrinth_project_id;
        }
        if merged.curseforge_project_id.is_none() {
            merged.curseforge_project_id = curseforge_project_id;
        }
        if let Some(popularity) = popularity_score
            && merged.popularity_score.unwrap_or(0) < popularity
        {
            merged.popularity_score = Some(popularity);
        }
        if let Some(updated_at) = updated_at
            && merged
                .updated_at
                .as_deref()
                .is_none_or(|current| current < updated_at.as_str())
        {
            merged.updated_at = Some(updated_at);
        }
        if relevance_rank < merged.relevance_rank {
            merged.relevance_rank = relevance_rank;
        }
        if !merged.sources.contains(&source) {
            merged.sources.push(source);
            merged.sources.sort_by_key(|source| source.label());
        }
    }
    by_key.into_values().collect()
}

fn build_search_snapshot(
    provider_entries: &[ProviderSearchEntry],
    warnings: &[String],
    mod_sort_mode: ModSortMode,
) -> BrowserSearchSnapshot {
    let mut entries = dedupe_browser_entries(provider_entries.to_vec());
    entries.sort_by(|left, right| {
        left.content_type.cmp(&right.content_type).then_with(|| {
            if left.content_type == BrowserContentType::Mod {
                compare_mod_entries(left, right, mod_sort_mode)
            } else {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            }
        })
    });
    BrowserSearchSnapshot {
        entries,
        warnings: warnings.to_vec(),
    }
}

pub(super) fn count_entries_by_content_type(entries: &[BrowserProjectEntry]) -> [usize; 4] {
    let mut counts = [0usize; 4];
    for entry in entries {
        counts[entry.content_type.index()] = counts[entry.content_type.index()].saturating_add(1);
    }
    counts
}

fn content_scope_task_count(scope: ContentScope) -> usize {
    BrowserContentType::ORDERED
        .iter()
        .filter(|content_type| scope.includes(**content_type))
        .count()
}

fn compare_mod_entries(
    left: &BrowserProjectEntry,
    right: &BrowserProjectEntry,
    mode: ModSortMode,
) -> std::cmp::Ordering {
    match mode {
        ModSortMode::Relevance => left
            .relevance_rank
            .cmp(&right.relevance_rank)
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            }),
        ModSortMode::LastUpdated => right
            .updated_at
            .as_deref()
            .unwrap_or("")
            .cmp(left.updated_at.as_deref().unwrap_or(""))
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            }),
        ModSortMode::Popularity => right
            .popularity_score
            .unwrap_or(0)
            .cmp(&left.popularity_score.unwrap_or(0))
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            }),
    }
}

fn ensure_download_channel(state: &mut ContentBrowserState) {
    if state.download_tx.is_some() && state.download_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<Result<ContentDownloadOutcome, String>>();
    state.download_tx = Some(tx);
    state.download_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn maybe_start_queued_download(
    state: &mut ContentBrowserState,
    instance_name: &str,
    instance_root: &Path,
) {
    if state.download_in_flight {
        return;
    }
    let Some(next) = state.download_queue.pop_front() else {
        return;
    };

    ensure_download_channel(state);
    let Some(tx) = state.download_tx.as_ref().cloned() else {
        return;
    };

    state.download_in_flight = true;
    state.active_download = Some(active_download_from_request(&next.request));
    state.download_notification_active = true;
    install_activity::set_status(
        instance_name,
        installation::InstallStage::DownloadingCore,
        "Applying content changes...",
    );
    notification::progress!(
        notification::Severity::Info,
        "content-browser/download",
        0.1f32,
        "Applying queued content operation..."
    );
    let root = instance_root.to_path_buf();
    let request = next.request.clone();

    let _ = tokio_runtime::spawn_detached(async move {
        let join = tokio_runtime::spawn_blocking(move || {
            apply_content_install_request(root.as_path(), request)
        });
        let result = match join.await {
            Ok(r) => r,
            Err(err) => Err(format!("content install worker panicked: {err}")),
        };
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                error = %err,
                "Failed to deliver queued content operation result."
            );
        }
    });
}

pub(super) fn poll_downloads(state: &mut ContentBrowserState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.download_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/content_browser",
                            active_download = ?state.active_download,
                            "Content download worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/content_browser",
                    active_download = ?state.active_download,
                    "Content download receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.download_tx = None;
        state.download_rx = None;
        state.download_in_flight = false;
        state.active_download = None;
        if let Some(instance_name) = state.active_instance_name.as_deref() {
            install_activity::clear_instance(instance_name);
        }
        state.status_message = Some("Content download worker stopped unexpectedly.".to_owned());
    }

    for update in updates {
        state.download_in_flight = false;
        state.active_download = None;
        if state.download_notification_active {
            state.download_notification_active = false;
        }
        if let Some(instance_name) = state.active_instance_name.as_deref() {
            install_activity::clear_instance(instance_name);
        }
        match update {
            Ok(result) => {
                state.manifest_dirty = true;
                state.status_message = Some(format!(
                    "Applied {}: {} added, {} removed.",
                    result.project_name,
                    result.added_files.len(),
                    result.removed_files.len()
                ));
                notification::progress!(
                    notification::Severity::Info,
                    "content-browser/download",
                    1.0f32,
                    "Content operation complete."
                );
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/content_browser",
                    error = %err,
                    "Queued content operation failed."
                );
                state.status_message = Some(format!("Content download failed: {err}"));
                notification::error!(
                    "content-browser/download",
                    "Content download failed: {}",
                    err
                );
            }
        }
    }
}
