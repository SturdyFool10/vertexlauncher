use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Once;
use std::thread;

use curseforge::{Client as CurseForgeClient, MINECRAFT_GAME_ID};
use modrinth::Client as ModrinthClient;
use tracing::{debug, warn};

static CURSEFORGE_MISSING_KEY_WARN_ONCE: Once = Once::new();

/// Upstream platform that provided a unified content result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContentSource {
    Modrinth,
    CurseForge,
}

impl ContentSource {
    /// Human-readable source label used in UI and sorting.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ContentSource::Modrinth => "Modrinth",
            ContentSource::CurseForge => "CurseForge",
        }
    }
}

/// Provider-agnostic content record returned from search.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnifiedContentEntry {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub content_type: String,
    pub source: ContentSource,
    pub project_url: Option<String>,
    pub icon_url: Option<String>,
}

/// Combined response from all configured providers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnifiedSearchResult {
    pub entries: Vec<UnifiedContentEntry>,
    pub discovered_types: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default)]
struct ProviderSearchResult {
    entries: Vec<UnifiedContentEntry>,
    discovered_types: Vec<String>,
    warnings: Vec<String>,
}

/// Errors raised by unified provider search orchestration.
#[derive(Debug, thiserror::Error)]
pub enum UnifiedSearchError {
    #[error("search query cannot be empty")]
    EmptyQuery,
}

/// Searches Minecraft content across supported providers.
///
/// Valid values:
/// - `query`: non-empty once trimmed.
/// - `per_provider_limit`: clamped to `1..=50`.
///
/// The function always attempts both providers. Provider-specific failures are
/// surfaced in `UnifiedSearchResult::warnings` while keeping partial results.
pub fn search_minecraft_content(
    query: &str,
    per_provider_limit: u32,
) -> Result<UnifiedSearchResult, UnifiedSearchError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        debug!(
            target: "vertexlauncher/modprovider",
            "rejecting unified search because query is empty"
        );
        return Err(UnifiedSearchError::EmptyQuery);
    }

    let limit = per_provider_limit.clamp(1, 50);
    debug!(
        target: "vertexlauncher/modprovider",
        query = trimmed,
        requested_limit = per_provider_limit,
        limit,
        "starting unified content search"
    );
    let query_owned = trimmed.to_owned();
    let (modrinth_result, curseforge_result) = thread::scope(|scope| {
        let modrinth_query = query_owned.clone();
        let modrinth_task = scope.spawn(move || search_modrinth(modrinth_query.as_str(), limit));

        let curseforge_query = query_owned.clone();
        let curseforge_task =
            scope.spawn(move || search_curseforge(curseforge_query.as_str(), limit));

        let modrinth_result = join_provider_result("Modrinth", modrinth_task.join());
        let curseforge_result = join_provider_result("CurseForge", curseforge_task.join());
        (modrinth_result, curseforge_result)
    });

    let mut result = UnifiedSearchResult::default();
    merge_provider_result(&mut result, modrinth_result);
    merge_provider_result(&mut result, curseforge_result);

    let mut discovered_types = BTreeSet::new();
    discovered_types.extend(
        result
            .entries
            .iter()
            .map(|entry| format!("{}: {}", entry.source.label(), entry.content_type)),
    );
    discovered_types.extend(result.discovered_types.iter().cloned());

    result.entries.sort_by(|a, b| {
        let left = a.name.to_ascii_lowercase();
        let right = b.name.to_ascii_lowercase();
        left.cmp(&right)
            .then_with(|| a.source.label().cmp(b.source.label()))
    });
    result.discovered_types = discovered_types.into_iter().collect();
    debug!(
        target: "vertexlauncher/modprovider",
        query = trimmed,
        entries = result.entries.len(),
        discovered_types = result.discovered_types.len(),
        warnings = result.warnings.len(),
        "completed unified content search"
    );
    Ok(result)
}

/// Runs Modrinth discovery/search and converts records into unified entries.
fn search_modrinth(query: &str, limit: u32) -> ProviderSearchResult {
    debug!(
        target: "vertexlauncher/modprovider",
        query,
        limit,
        provider = "Modrinth",
        "querying provider"
    );
    let mut result = ProviderSearchResult::default();
    let modrinth = ModrinthClient::default();

    match modrinth.list_project_types() {
        Ok(types) => {
            result.discovered_types.extend(
                types
                    .into_iter()
                    .map(|project_type| format!("Modrinth: {project_type}")),
            );
        }
        Err(err) => {
            warn!(
                target: "vertexlauncher/modprovider",
                provider = "Modrinth",
                error = %err,
                "provider type discovery failed"
            );
            result
                .warnings
                .push(format!("Modrinth project type discovery failed: {err}"));
        }
    }

    match modrinth.search_projects(query, limit, 0) {
        Ok(items) => {
            result
                .entries
                .extend(items.into_iter().map(|item| UnifiedContentEntry {
                    id: format!("modrinth:{}", item.project_id),
                    name: item.title,
                    summary: item.description.trim().to_owned(),
                    content_type: item.project_type,
                    source: ContentSource::Modrinth,
                    project_url: Some(item.project_url),
                    icon_url: item.icon_url,
                }));
        }
        Err(err) => {
            warn!(
                target: "vertexlauncher/modprovider",
                provider = "Modrinth",
                error = %err,
                "provider search failed"
            );
            result
                .warnings
                .push(format!("Modrinth search failed: {err}"));
        }
    }

    debug!(
        target: "vertexlauncher/modprovider",
        provider = "Modrinth",
        entries = result.entries.len(),
        discovered_types = result.discovered_types.len(),
        warnings = result.warnings.len(),
        "provider query complete"
    );
    result
}

/// Runs CurseForge discovery/search and converts records into unified entries.
///
/// Missing API key is treated as a recoverable provider warning.
fn search_curseforge(query: &str, limit: u32) -> ProviderSearchResult {
    debug!(
        target: "vertexlauncher/modprovider",
        query,
        limit,
        provider = "CurseForge",
        "querying provider"
    );
    let mut result = ProviderSearchResult::default();
    let Some(curseforge) = CurseForgeClient::from_env() else {
        let mut emitted_warn = false;
        CURSEFORGE_MISSING_KEY_WARN_ONCE.call_once(|| {
            emitted_warn = true;
            warn!(
                target: "vertexlauncher/modprovider",
                provider = "CurseForge",
                "provider disabled because API key is missing"
            );
        });
        if !emitted_warn {
            debug!(
                target: "vertexlauncher/modprovider",
                provider = "CurseForge",
                "provider disabled because API key is missing (repeat suppressed)"
            );
        }
        result.warnings.push(
            "CurseForge API key missing (set VERTEX_CURSEFORGE_API_KEY or CURSEFORGE_API_KEY). \
Showing Modrinth results only."
                .to_owned(),
        );
        return result;
    };

    let class_map = match curseforge.list_content_classes(MINECRAFT_GAME_ID) {
        Ok(classes) => {
            let mut class_map = HashMap::new();
            for class_entry in classes {
                result
                    .discovered_types
                    .push(format!("CurseForge: {}", class_entry.name));
                class_map.insert(class_entry.id, class_entry.name);
            }
            class_map
        }
        Err(err) => {
            warn!(
                target: "vertexlauncher/modprovider",
                provider = "CurseForge",
                error = %err,
                "provider class discovery failed"
            );
            result
                .warnings
                .push(format!("CurseForge class discovery failed: {err}"));
            HashMap::new()
        }
    };

    match curseforge.search_projects(MINECRAFT_GAME_ID, query, 0, limit) {
        Ok(items) => {
            result.entries.extend(items.into_iter().map(|item| {
                UnifiedContentEntry {
                    id: format!("curseforge:{}", item.id),
                    name: item.name,
                    summary: item.summary.trim().to_owned(),
                    content_type: class_map
                        .get(&item.class_id)
                        .cloned()
                        .unwrap_or_else(|| format!("Class {}", item.class_id)),
                    source: ContentSource::CurseForge,
                    project_url: item.website_url,
                    icon_url: item.icon_url,
                }
            }));
        }
        Err(err) => {
            warn!(
                target: "vertexlauncher/modprovider",
                provider = "CurseForge",
                error = %err,
                "provider search failed"
            );
            result
                .warnings
                .push(format!("CurseForge search failed: {err}"));
        }
    }

    debug!(
        target: "vertexlauncher/modprovider",
        provider = "CurseForge",
        entries = result.entries.len(),
        discovered_types = result.discovered_types.len(),
        warnings = result.warnings.len(),
        "provider query complete"
    );
    result
}

/// Appends entries, discovered types, and warnings from one provider result.
fn merge_provider_result(result: &mut UnifiedSearchResult, provider_result: ProviderSearchResult) {
    result.entries.extend(provider_result.entries);
    result
        .discovered_types
        .extend(provider_result.discovered_types);
    result.warnings.extend(provider_result.warnings);
}

/// Converts thread join failures into warnings so one provider panic does not
/// terminate the full multi-provider search flow.
fn join_provider_result(
    provider: &'static str,
    join_result: std::thread::Result<ProviderSearchResult>,
) -> ProviderSearchResult {
    join_result.unwrap_or_else(|_| {
        warn!(
            target: "vertexlauncher/modprovider",
            provider,
            "provider search task panicked"
        );
        ProviderSearchResult {
            warnings: vec![format!("{provider} search task panicked.")],
            ..ProviderSearchResult::default()
        }
    })
}
