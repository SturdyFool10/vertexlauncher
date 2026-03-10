use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::io::Read as _;
use tracing::{debug, warn};

const DEFAULT_MODRINTH_API_BASE_URL: &str = "https://api.modrinth.com/v2";
const DEFAULT_USER_AGENT: &str =
    "VertexLauncher/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";

/// Errors returned by Modrinth API requests.
#[derive(Debug, thiserror::Error)]
pub enum ModrinthError {
    #[error("HTTP status {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("HTTP transport error: {0}")]
    Transport(String),
    #[error("Response read error: {0}")]
    Read(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Lightweight Modrinth API client.
///
/// By default this targets the public v2 API and sends a launcher-specific
/// user agent. `with_base_url` is intended for testing or custom deployments.
#[derive(Clone, Debug)]
pub struct Client {
    agent: ureq::Agent,
    base_url: String,
    user_agent: String,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            agent: ureq::Agent::new_with_defaults(),
            base_url: DEFAULT_MODRINTH_API_BASE_URL.to_owned(),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
        }
    }
}

/// A normalized search entry returned from Modrinth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchProject {
    pub project_id: String,
    pub slug: Option<String>,
    pub title: String,
    pub description: String,
    pub project_type: String,
    pub icon_url: Option<String>,
    pub author: Option<String>,
    pub project_url: String,
    pub downloads: u64,
    pub date_modified: Option<String>,
}

/// Detailed project metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub project_id: String,
    pub slug: Option<String>,
    pub title: String,
    pub description: String,
    pub project_type: String,
    pub icon_url: Option<String>,
    pub project_url: String,
}

/// A compatible Modrinth version entry for a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectVersion {
    pub id: String,
    pub project_id: String,
    pub version_number: String,
    pub date_published: String,
    pub downloads: u64,
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
    pub dependencies: Vec<ProjectDependency>,
    pub files: Vec<ProjectVersionFile>,
}

/// Dependency edge declared by a Modrinth project version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectDependency {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub dependency_type: String,
    pub file_name: Option<String>,
}

/// A downloadable file on a Modrinth project version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectVersionFile {
    pub url: String,
    pub filename: String,
    pub primary: bool,
}

impl Client {
    /// Overrides the base API URL.
    ///
    /// `base_url` should point to a Modrinth-compatible API root, for example
    /// `https://api.modrinth.com/v2`.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Overrides the `User-Agent` header sent on all API requests.
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Lists available Modrinth project types.
    pub fn list_project_types(&self) -> Result<Vec<String>, ModrinthError> {
        debug!(
            target: "vertexlauncher/modrinth",
            endpoint = "/tag/project_type",
            "listing Modrinth project types"
        );
        let project_types: Vec<String> = self.get_json("/tag/project_type", &[])?;
        debug!(
            target: "vertexlauncher/modrinth",
            count = project_types.len(),
            "received Modrinth project types"
        );
        Ok(project_types)
    }

    /// Searches Modrinth projects by free-text query.
    ///
    /// Valid values:
    /// - `query`: non-empty once trimmed.
    /// - `limit`: clamped to `1..=100`.
    /// - `offset`: passed directly to Modrinth pagination.
    ///
    /// Returns an empty vector for blank queries instead of failing.
    pub fn search_projects(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SearchProject>, ModrinthError> {
        self.search_projects_with_filters(query, limit, offset, None, None, None)
    }

    /// Searches Modrinth projects with optional compatibility filters.
    ///
    /// - `project_type`: values such as `mod`, `resourcepack`, `shader`, `datapack`.
    /// - `game_version`: Minecraft version string (for example `1.20.1`).
    /// - `loader`: mod loader slug (for example `fabric`, `forge`, `neoforge`, `quilt`).
    ///
    /// `loader` is only meaningful for mod projects.
    pub fn search_projects_with_filters(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
        project_type: Option<&str>,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<SearchProject>, ModrinthError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            debug!(
                target: "vertexlauncher/modrinth",
                "skipping Modrinth search because query is empty"
            );
            return Ok(Vec::new());
        }

        let limit = limit.clamp(1, 100);
        debug!(
            target: "vertexlauncher/modrinth",
            query = trimmed,
            limit,
            offset,
            project_type = project_type.unwrap_or(""),
            game_version = game_version.unwrap_or(""),
            loader = loader.unwrap_or(""),
            "searching Modrinth projects"
        );
        let mut search_query = vec![
            ("query", trimmed.to_owned()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        let mut facets: Vec<Vec<String>> = Vec::new();
        if let Some(project_type) = non_empty(project_type) {
            facets.push(vec![format!("project_type:{project_type}")]);
        }
        if let Some(game_version) = non_empty(game_version) {
            facets.push(vec![format!("versions:{game_version}")]);
        }
        if let Some(loader) = non_empty(loader) {
            facets.push(vec![format!("categories:{}", loader.to_ascii_lowercase())]);
        }
        if !facets.is_empty() {
            let facets_json = serde_json::to_string(&facets).map_err(ModrinthError::Json)?;
            search_query.push(("facets", facets_json));
        }

        let response: SearchResponse = self.get_json("/search", &search_query)?;

        let projects: Vec<SearchProject> = response
            .hits
            .into_iter()
            .map(SearchHit::into_search_project)
            .collect();
        debug!(
            target: "vertexlauncher/modrinth",
            query = trimmed,
            returned = projects.len(),
            "Modrinth search complete"
        );
        Ok(projects)
    }

    /// Fetches detailed project metadata.
    pub fn get_project(&self, project_id_or_slug: &str) -> Result<Project, ModrinthError> {
        let project_key = project_id_or_slug.trim();
        if project_key.is_empty() {
            return Err(ModrinthError::Transport(
                "project id or slug cannot be empty".to_owned(),
            ));
        }

        debug!(
            target: "vertexlauncher/modrinth",
            project = project_key,
            "fetching Modrinth project"
        );
        let path = format!("/project/{project_key}");
        let project: ProjectRecord = self.get_json(path.as_str(), &[])?;
        Ok(project.into_project())
    }

    /// Lists compatible versions for a project.
    ///
    /// - `project_id_or_slug`: Modrinth project ID or slug.
    /// - `loaders`: optional loader slugs used to narrow compatibility.
    /// - `game_versions`: optional Minecraft versions used to narrow compatibility.
    pub fn list_project_versions(
        &self,
        project_id_or_slug: &str,
        loaders: &[String],
        game_versions: &[String],
    ) -> Result<Vec<ProjectVersion>, ModrinthError> {
        let project_key = project_id_or_slug.trim();
        if project_key.is_empty() {
            return Ok(Vec::new());
        }

        debug!(
            target: "vertexlauncher/modrinth",
            project = project_key,
            loaders = loaders.len(),
            game_versions = game_versions.len(),
            "listing Modrinth project versions"
        );
        let mut query = Vec::new();
        if !loaders.is_empty() {
            let json = serde_json::to_string(loaders).map_err(ModrinthError::Json)?;
            query.push(("loaders", json));
        }
        if !game_versions.is_empty() {
            let json = serde_json::to_string(game_versions).map_err(ModrinthError::Json)?;
            query.push(("game_versions", json));
        }

        let path = format!("/project/{project_key}/version");
        let versions: Vec<ProjectVersionRecord> = self.get_json(path.as_str(), &query)?;
        Ok(versions
            .into_iter()
            .map(ProjectVersionRecord::into_project_version)
            .collect())
    }

    /// Fetches a specific version by version ID.
    pub fn get_version(&self, version_id: &str) -> Result<ProjectVersion, ModrinthError> {
        let version_id = version_id.trim();
        if version_id.is_empty() {
            return Err(ModrinthError::Transport(
                "version id cannot be empty".to_owned(),
            ));
        }

        debug!(
            target: "vertexlauncher/modrinth",
            version_id,
            "fetching Modrinth version"
        );
        let path = format!("/version/{version_id}");
        let version: ProjectVersionRecord = self.get_json(path.as_str(), &[])?;
        Ok(version.into_project_version())
    }

    /// Executes a GET request and deserializes the JSON body.
    ///
    /// `path` is appended to the configured `base_url`.
    fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, ModrinthError> {
        debug!(
            target: "vertexlauncher/modrinth",
            path,
            query_count = query.len(),
            "sending Modrinth request"
        );

        let mut request = self
            .agent
            .get(&format!("{}{}", self.base_url, path))
            .header("User-Agent", &self.user_agent);

        for (key, value) in query {
            request = request.query(key, value);
        }

        let mut response = match request.config().http_status_as_error(false).build().call() {
            Ok(ok) => ok,
            Err(err) => {
                warn!(
                    target: "vertexlauncher/modrinth",
                    path,
                    error = %err,
                    "Modrinth transport error"
                );
                return Err(ModrinthError::Transport(err.to_string()));
            }
        };

        let status = response.status().as_u16();
        let mut raw = String::new();
        response
            .body_mut()
            .as_reader()
            .read_to_string(&mut raw)
            .map_err(|err| {
                warn!(
                    target: "vertexlauncher/modrinth",
                    path,
                    error = %err,
                    "failed to read Modrinth response body"
                );
                ModrinthError::Read(err)
            })?;
        if status >= 400 {
            warn!(
                target: "vertexlauncher/modrinth",
                path,
                status,
                body_len = raw.len(),
                "Modrinth returned non-success status"
            );
            return Err(ModrinthError::HttpStatus { status, body: raw });
        }

        serde_json::from_str(&raw).map_err(|err| {
            warn!(
                target: "vertexlauncher/modrinth",
                path,
                error = %err,
                "failed to parse Modrinth response body"
            );
            ModrinthError::Json(err)
        })
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    hits: Vec<SearchHit>,
}

#[derive(Debug, Deserialize)]
struct SearchHit {
    project_id: String,
    slug: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    project_type: String,
    icon_url: Option<String>,
    author: Option<String>,
    #[serde(default)]
    downloads: u64,
    date_modified: Option<String>,
}

impl SearchHit {
    fn into_search_project(self) -> SearchProject {
        let project_url = build_project_url(
            self.project_type.as_str(),
            self.slug.as_deref(),
            self.project_id.as_str(),
        );

        SearchProject {
            project_id: self.project_id,
            slug: self.slug,
            title: self.title,
            description: self.description,
            project_type: self.project_type,
            icon_url: self.icon_url,
            author: self.author,
            project_url,
            downloads: self.downloads,
            date_modified: self.date_modified,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProjectRecord {
    id: String,
    slug: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    project_type: String,
    icon_url: Option<String>,
}

impl ProjectRecord {
    fn into_project(self) -> Project {
        let project_url = build_project_url(
            self.project_type.as_str(),
            self.slug.as_deref(),
            self.id.as_str(),
        );

        Project {
            project_id: self.id,
            slug: self.slug,
            title: self.title,
            description: self.description,
            project_type: self.project_type,
            icon_url: self.icon_url,
            project_url,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProjectVersionRecord {
    id: String,
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    version_number: String,
    #[serde(default)]
    date_published: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    dependencies: Vec<ProjectDependencyRecord>,
    #[serde(default)]
    files: Vec<ProjectVersionFileRecord>,
}

impl ProjectVersionRecord {
    fn into_project_version(self) -> ProjectVersion {
        ProjectVersion {
            id: self.id,
            project_id: self.project_id,
            version_number: self.version_number,
            date_published: self.date_published,
            downloads: self.downloads,
            loaders: self.loaders,
            game_versions: self.game_versions,
            dependencies: self
                .dependencies
                .into_iter()
                .map(ProjectDependencyRecord::into_project_dependency)
                .collect(),
            files: self
                .files
                .into_iter()
                .map(|file| ProjectVersionFile {
                    url: file.url,
                    filename: file.filename,
                    primary: file.primary,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProjectDependencyRecord {
    project_id: Option<String>,
    version_id: Option<String>,
    #[serde(default)]
    dependency_type: String,
    file_name: Option<String>,
}

impl ProjectDependencyRecord {
    fn into_project_dependency(self) -> ProjectDependency {
        ProjectDependency {
            project_id: self.project_id,
            version_id: self.version_id,
            dependency_type: self.dependency_type,
            file_name: self.file_name,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProjectVersionFileRecord {
    url: String,
    filename: String,
    #[serde(default)]
    primary: bool,
}

fn build_project_url(project_type: &str, slug: Option<&str>, fallback_id: &str) -> String {
    let canonical_slug = slug
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_id);
    let canonical_type = project_type.trim();
    format!("https://modrinth.com/{canonical_type}/{canonical_slug}")
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}
