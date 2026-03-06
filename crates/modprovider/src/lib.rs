use std::collections::{BTreeSet, HashMap};

use curseforge::{Client as CurseForgeClient, MINECRAFT_GAME_ID};
use modrinth::Client as ModrinthClient;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContentSource {
    Modrinth,
    CurseForge,
}

impl ContentSource {
    pub fn label(self) -> &'static str {
        match self {
            ContentSource::Modrinth => "Modrinth",
            ContentSource::CurseForge => "CurseForge",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnifiedContentEntry {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub content_type: String,
    pub source: ContentSource,
    pub project_url: Option<String>,
    pub icon_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnifiedSearchResult {
    pub entries: Vec<UnifiedContentEntry>,
    pub discovered_types: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum UnifiedSearchError {
    #[error("search query cannot be empty")]
    EmptyQuery,
}

pub fn search_minecraft_content(
    query: &str,
    per_provider_limit: u32,
) -> Result<UnifiedSearchResult, UnifiedSearchError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(UnifiedSearchError::EmptyQuery);
    }

    let mut result = UnifiedSearchResult::default();
    let mut discovered_types = BTreeSet::new();
    let limit = per_provider_limit.clamp(1, 50);

    let modrinth = ModrinthClient::default();
    match modrinth.list_project_types() {
        Ok(types) => {
            for project_type in types {
                discovered_types.insert(format!("Modrinth: {project_type}"));
            }
        }
        Err(err) => result
            .warnings
            .push(format!("Modrinth project type discovery failed: {err}")),
    }

    match modrinth.search_projects(trimmed, limit, 0) {
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
        Err(err) => result
            .warnings
            .push(format!("Modrinth search failed: {err}")),
    }

    let curseforge = CurseForgeClient::from_env();
    if let Some(curseforge) = curseforge {
        let class_map = match curseforge.list_content_classes(MINECRAFT_GAME_ID) {
            Ok(classes) => {
                let mut class_map = HashMap::new();
                for class_entry in classes {
                    discovered_types.insert(format!("CurseForge: {}", class_entry.name));
                    class_map.insert(class_entry.id, class_entry.name);
                }
                class_map
            }
            Err(err) => {
                result
                    .warnings
                    .push(format!("CurseForge class discovery failed: {err}"));
                HashMap::new()
            }
        };

        match curseforge.search_projects(MINECRAFT_GAME_ID, trimmed, 0, limit) {
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
            Err(err) => result
                .warnings
                .push(format!("CurseForge search failed: {err}")),
        }
    } else {
        result.warnings.push(
            "CurseForge API key missing (set VERTEX_CURSEFORGE_API_KEY or CURSEFORGE_API_KEY). \
Showing Modrinth results only."
                .to_owned(),
        );
    }

    result.entries.sort_by(|a, b| {
        let left = a.name.to_ascii_lowercase();
        let right = b.name.to_ascii_lowercase();
        left.cmp(&right)
            .then_with(|| a.source.label().cmp(b.source.label()))
    });
    result.discovered_types = discovered_types.into_iter().collect();
    Ok(result)
}
