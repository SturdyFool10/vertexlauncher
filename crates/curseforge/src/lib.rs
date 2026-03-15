use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::Read as _;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

const DEFAULT_CURSEFORGE_API_BASE_URL: &str = "https://api.curseforge.com";
const DEFAULT_USER_AGENT: &str =
    "VertexLauncher/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";
const DEFAULT_RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(60);
const DEFAULT_MIN_REQUEST_SPACING: Duration = Duration::from_millis(500);
pub const MINECRAFT_GAME_ID: u32 = 432;
static API_KEY_OVERRIDE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

#[derive(Clone, Copy, Debug)]
struct RateLimitState {
    next_request_at: Instant,
    cooldown_until: Option<Instant>,
}

impl RateLimitState {
    fn new() -> Self {
        Self {
            next_request_at: Instant::now(),
            cooldown_until: None,
        }
    }
}

fn api_key_override_store() -> &'static Mutex<Option<String>> {
    API_KEY_OVERRIDE.get_or_init(|| Mutex::new(None))
}

fn rate_limit_store() -> &'static Mutex<RateLimitState> {
    static STORE: OnceLock<Mutex<RateLimitState>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(RateLimitState::new()))
}

/// Errors returned by CurseForge API requests.
#[derive(Debug, thiserror::Error)]
pub enum CurseForgeError {
    #[error("CurseForge API key is missing (set VERTEX_CURSEFORGE_API_KEY or CURSEFORGE_API_KEY)")]
    MissingApiKey,
    #[error("HTTP status {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("HTTP transport error: {0}")]
    Transport(String),
    #[error("Response read error: {0}")]
    Read(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

/// CurseForge API client.
///
/// Requires an API key from CurseForge for all requests.
#[derive(Clone, Debug)]
pub struct Client {
    agent: ureq::Agent,
    base_url: String,
    user_agent: String,
    api_key: String,
}

/// A top-level CurseForge content class, such as Mods or Resource Packs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentClass {
    pub id: u32,
    pub name: String,
    pub slug: Option<String>,
}

/// A normalized CurseForge project entry from search results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchProject {
    pub id: u64,
    pub name: String,
    pub summary: String,
    pub slug: Option<String>,
    pub class_id: u32,
    pub primary_category_id: Option<u32>,
    pub website_url: Option<String>,
    pub icon_url: Option<String>,
    pub download_count: u64,
    pub date_modified: Option<String>,
}

/// Detailed CurseForge project metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub summary: String,
    pub slug: Option<String>,
    pub class_id: u32,
    pub primary_category_id: Option<u32>,
    pub website_url: Option<String>,
    pub icon_url: Option<String>,
    pub latest_files_indexes: Vec<LatestFileIndex>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatestFileIndex {
    pub file_id: u64,
    pub filename: String,
    pub game_version: String,
    pub mod_loader: Option<u32>,
    pub game_version_type_id: Option<u32>,
}

/// Downloadable file metadata for a CurseForge project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub id: u64,
    pub display_name: String,
    pub file_name: String,
    pub file_date: String,
    pub download_count: u64,
    pub download_url: Option<String>,
    pub dependencies: Vec<FileDependency>,
    pub game_versions: Vec<String>,
}

/// Dependency relationship declared by a CurseForge file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDependency {
    pub mod_id: u64,
    pub relation_type: u32,
}

impl Client {
    /// Builds a client from process environment variables.
    ///
    /// The first available key is used:
    /// - `VERTEX_CURSEFORGE_API_KEY`
    /// - `CURSEFORGE_API_KEY`
    ///
    /// Returns `None` if no key exists or if the key is blank/invalid.
    pub fn from_env() -> Option<Self> {
        if let Ok(override_key) = api_key_override_store().lock()
            && let Some(key) = override_key.as_deref()
        {
            return Self::from_api_key(key.to_owned()).ok();
        }
        let key = std::env::var("VERTEX_CURSEFORGE_API_KEY")
            .ok()
            .or_else(|| std::env::var("CURSEFORGE_API_KEY").ok())?;
        debug!(
            target: "vertexlauncher/curseforge",
            "loaded CurseForge API key from environment"
        );
        Self::from_api_key(key).ok()
    }

    /// Builds a client from a raw API key string.
    ///
    /// The key must be non-empty after trimming whitespace.
    pub fn from_api_key(api_key: impl Into<String>) -> Result<Self, CurseForgeError> {
        let api_key = api_key.into().trim().to_owned();
        if api_key.is_empty() {
            warn!(
                target: "vertexlauncher/curseforge",
                "attempted to construct CurseForge client with empty API key"
            );
            return Err(CurseForgeError::MissingApiKey);
        }

        Ok(Self {
            agent: ureq::Agent::new_with_defaults(),
            base_url: DEFAULT_CURSEFORGE_API_BASE_URL.to_owned(),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            api_key,
        })
    }

    /// Overrides the base API URL.
    ///
    /// `base_url` should point to a CurseForge-compatible API root.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Lists available top-level content classes for the given game.
    pub fn list_content_classes(&self, game_id: u32) -> Result<Vec<ContentClass>, CurseForgeError> {
        debug!(
            target: "vertexlauncher/curseforge",
            game_id,
            endpoint = "/v1/categories",
            "listing CurseForge content classes"
        );
        let response: DataResponse<Vec<CategoryRecord>> = self.get_json(
            "/v1/categories",
            &[
                ("gameId", game_id.to_string()),
                ("classesOnly", "true".to_owned()),
            ],
        )?;

        let classes: Vec<ContentClass> = response
            .data
            .into_iter()
            .map(|category| ContentClass {
                id: category.id,
                name: category.name,
                slug: category.slug,
            })
            .collect();
        debug!(
            target: "vertexlauncher/curseforge",
            game_id,
            count = classes.len(),
            "received CurseForge content classes"
        );
        Ok(classes)
    }

    /// Searches CurseForge projects.
    ///
    /// Valid values:
    /// - `query`: non-empty once trimmed.
    /// - `page_size`: clamped to `1..=50`.
    /// - `index`: passed directly as the starting offset.
    pub fn search_projects(
        &self,
        game_id: u32,
        query: &str,
        index: u32,
        page_size: u32,
    ) -> Result<Vec<SearchProject>, CurseForgeError> {
        self.search_projects_with_filters(game_id, query, index, page_size, None, None, None)
    }

    /// Searches CurseForge projects with optional class and compatibility filters.
    pub fn search_projects_with_filters(
        &self,
        game_id: u32,
        query: &str,
        index: u32,
        page_size: u32,
        class_id: Option<u32>,
        game_version: Option<&str>,
        mod_loader_type: Option<u32>,
    ) -> Result<Vec<SearchProject>, CurseForgeError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            debug!(
                target: "vertexlauncher/curseforge",
                "skipping CurseForge search because query is empty"
            );
            return Ok(Vec::new());
        }

        let page_size = page_size.clamp(1, 50);
        debug!(
            target: "vertexlauncher/curseforge",
            game_id,
            query = trimmed,
            index,
            page_size,
            class_id = class_id.unwrap_or_default(),
            game_version = game_version.unwrap_or(""),
            mod_loader_type = mod_loader_type.unwrap_or_default(),
            "searching CurseForge projects"
        );
        let mut query_params = vec![
            ("gameId", game_id.to_string()),
            ("searchFilter", trimmed.to_owned()),
            ("index", index.to_string()),
            ("pageSize", page_size.to_string()),
        ];
        if let Some(class_id) = class_id {
            query_params.push(("classId", class_id.to_string()));
        }
        if let Some(game_version) = non_empty(game_version) {
            query_params.push(("gameVersion", game_version.to_owned()));
        }
        if let Some(mod_loader_type) = mod_loader_type {
            query_params.push(("modLoaderType", mod_loader_type.to_string()));
        }

        let response: DataResponse<Vec<ModRecord>> =
            self.get_json("/v1/mods/search", &query_params)?;

        let projects: Vec<SearchProject> = response
            .data
            .into_iter()
            .map(ModRecord::into_search_project)
            .collect();
        debug!(
            target: "vertexlauncher/curseforge",
            game_id,
            query = trimmed,
            returned = projects.len(),
            "CurseForge search complete"
        );
        Ok(projects)
    }

    /// Fetches a project by project ID.
    pub fn get_mod(&self, project_id: u64) -> Result<Project, CurseForgeError> {
        debug!(
            target: "vertexlauncher/curseforge",
            project_id,
            "fetching CurseForge project"
        );
        let path = format!("/v1/mods/{project_id}");
        let response: DataResponse<ModRecord> = self.get_json(path.as_str(), &[])?;
        Ok(response.data.into_project())
    }

    /// Fetches multiple projects by ID in one request.
    pub fn get_mods(&self, project_ids: &[u64]) -> Result<Vec<Project>, CurseForgeError> {
        let project_ids = prepare_u64_ids(project_ids);
        if project_ids.is_empty() {
            return Ok(Vec::new());
        }

        debug!(
            target: "vertexlauncher/curseforge",
            projects = project_ids.len(),
            "fetching CurseForge projects in batch"
        );
        let response: DataResponse<Vec<ModRecord>> = self.post_json(
            "/v1/mods",
            &ModIdsRequest {
                mod_ids: project_ids.as_slice(),
            },
        )?;
        Ok(response
            .data
            .into_iter()
            .map(ModRecord::into_project)
            .collect())
    }

    /// Lists files for a project, optionally filtered by compatibility.
    pub fn list_mod_files(
        &self,
        project_id: u64,
        game_version: Option<&str>,
        mod_loader_type: Option<u32>,
        index: u32,
        page_size: u32,
    ) -> Result<Vec<File>, CurseForgeError> {
        let page_size = page_size.clamp(1, 50);
        let mut query_params = vec![
            ("index", index.to_string()),
            ("pageSize", page_size.to_string()),
        ];
        if let Some(game_version) = non_empty(game_version) {
            query_params.push(("gameVersion", game_version.to_owned()));
        }
        if let Some(mod_loader_type) = mod_loader_type {
            query_params.push(("modLoaderType", mod_loader_type.to_string()));
        }

        debug!(
            target: "vertexlauncher/curseforge",
            project_id,
            page_size,
            game_version = game_version.unwrap_or(""),
            mod_loader_type = mod_loader_type.unwrap_or_default(),
            "listing CurseForge files"
        );
        let path = format!("/v1/mods/{project_id}/files");
        let response: DataResponse<Vec<FileRecord>> =
            self.get_json(path.as_str(), &query_params)?;
        Ok(response
            .data
            .into_iter()
            .map(FileRecord::into_file)
            .collect())
    }

    /// Fetches multiple files by ID in one request.
    pub fn get_files(&self, file_ids: &[u64]) -> Result<Vec<File>, CurseForgeError> {
        let file_ids = prepare_u64_ids(file_ids);
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        debug!(
            target: "vertexlauncher/curseforge",
            files = file_ids.len(),
            "fetching CurseForge files in batch"
        );
        let response: DataResponse<Vec<FileRecord>> = self.post_json(
            "/v1/mods/files",
            &FileIdsRequest {
                file_ids: file_ids.as_slice(),
            },
        )?;
        Ok(response
            .data
            .into_iter()
            .map(FileRecord::into_file)
            .collect())
    }

    /// Executes a GET request and deserializes the JSON body.
    ///
    /// `path` is appended to the configured `base_url`.
    fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, CurseForgeError> {
        self.acquire_rate_limit_slot(path);
        debug!(
            target: "vertexlauncher/curseforge",
            path,
            query_count = query.len(),
            "sending CurseForge request"
        );

        let mut request = self
            .agent
            .get(&format!("{}{}", self.base_url, path))
            .header("User-Agent", &self.user_agent)
            .header("x-api-key", &self.api_key);

        for (key, value) in query {
            request = request.query(key, value);
        }

        self.read_json_response(
            path,
            request.config().http_status_as_error(false).build().call(),
        )
    }

    fn post_json<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, CurseForgeError> {
        self.acquire_rate_limit_slot(path);
        debug!(
            target: "vertexlauncher/curseforge",
            path,
            "sending CurseForge POST request"
        );

        self.read_json_response(
            path,
            self.agent
                .post(&format!("{}{}", self.base_url, path))
                .header("User-Agent", &self.user_agent)
                .header("x-api-key", &self.api_key)
                .config()
                .http_status_as_error(false)
                .build()
                .send_json(body),
        )
    }

    fn read_json_response<T: DeserializeOwned>(
        &self,
        path: &str,
        response_result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<T, CurseForgeError> {
        let mut response = match response_result {
            Ok(ok) => ok,
            Err(err) => {
                warn!(
                    target: "vertexlauncher/curseforge",
                    path,
                    error = %err,
                    "CurseForge transport error"
                );
                return Err(CurseForgeError::Transport(err.to_string()));
            }
        };

        let status = response.status().as_u16();
        let retry_after_secs = parse_retry_after_seconds(response.headers());
        let mut raw = String::new();
        response
            .body_mut()
            .as_reader()
            .read_to_string(&mut raw)
            .map_err(|err| {
                warn!(
                    target: "vertexlauncher/curseforge",
                    path,
                    error = %err,
                    "failed to read CurseForge response body"
                );
                CurseForgeError::Read(err)
            })?;
        if status == 429 {
            self.note_rate_limit(path, retry_after_secs);
        }
        if status >= 400 {
            warn!(
                target: "vertexlauncher/curseforge",
                path,
                status,
                body_len = raw.len(),
                "CurseForge returned non-success status"
            );
            return Err(CurseForgeError::HttpStatus { status, body: raw });
        }

        self.note_successful_request();

        serde_json::from_str(&raw).map_err(|err| {
            warn!(
                target: "vertexlauncher/curseforge",
                path,
                error = %err,
                "failed to parse CurseForge response body"
            );
            CurseForgeError::Json(err)
        })
    }

    fn acquire_rate_limit_slot(&self, path: &str) {
        loop {
            let Some(wait) = self.reserve_next_request_slot() else {
                return;
            };
            debug!(
                target: "vertexlauncher/curseforge",
                path,
                wait_ms = wait.as_millis() as u64,
                "waiting for CurseForge rate-limit slot"
            );
            thread::sleep(wait);
        }
    }

    fn reserve_next_request_slot(&self) -> Option<Duration> {
        let Ok(mut guard) = rate_limit_store().lock() else {
            return None;
        };
        let now = Instant::now();
        if let Some(cooldown_until) = guard.cooldown_until {
            if cooldown_until > now {
                return Some(cooldown_until.saturating_duration_since(now));
            }
            guard.cooldown_until = None;
        }
        if guard.next_request_at > now {
            return Some(guard.next_request_at.saturating_duration_since(now));
        }
        guard.next_request_at = now + DEFAULT_MIN_REQUEST_SPACING;
        None
    }

    fn note_rate_limit(&self, path: &str, retry_after_secs: Option<u64>) {
        let cooldown = Duration::from_secs(
            retry_after_secs
                .unwrap_or(DEFAULT_RATE_LIMIT_COOLDOWN.as_secs())
                .max(1),
        );
        let until = Instant::now() + cooldown;
        if let Ok(mut guard) = rate_limit_store().lock() {
            if guard.cooldown_until.is_none_or(|existing| existing < until) {
                guard.cooldown_until = Some(until);
            }
            guard.next_request_at = guard.next_request_at.max(until);
        }
        warn!(
            target: "vertexlauncher/curseforge",
            path,
            retry_after_secs = retry_after_secs.unwrap_or(DEFAULT_RATE_LIMIT_COOLDOWN.as_secs()),
            "CurseForge rate limited request; backing off further API calls"
        );
    }

    fn note_successful_request(&self) {
        if let Ok(mut guard) = rate_limit_store().lock() {
            if guard
                .cooldown_until
                .is_some_and(|until| until <= Instant::now())
            {
                guard.cooldown_until = None;
            }
        }
    }
}

/// Sets an in-process API key override used by [`Client::from_env`].
///
/// Pass `None` to clear the override and fall back to environment variables.
pub fn set_api_key_override(api_key: Option<String>) {
    let normalized = api_key
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty());
    if let Ok(mut store) = api_key_override_store().lock() {
        *store = normalized;
    }
}

fn parse_retry_after_seconds(headers: &ureq::http::HeaderMap) -> Option<u64> {
    headers
        .get("retry-after")
        .or_else(|| headers.get("Retry-After"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

#[derive(Debug, Deserialize)]
struct DataResponse<T> {
    data: T,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModIdsRequest<'a> {
    mod_ids: &'a [u64],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileIdsRequest<'a> {
    file_ids: &'a [u64],
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CategoryRecord {
    id: u32,
    name: String,
    slug: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModRecord {
    id: u64,
    name: String,
    slug: Option<String>,
    summary: Option<String>,
    class_id: u32,
    primary_category_id: Option<u32>,
    links: Option<ModLinks>,
    logo: Option<ModLogo>,
    download_count: Option<f64>,
    date_modified: Option<String>,
    #[serde(default)]
    latest_files_indexes: Vec<LatestFileIndexRecord>,
}

impl ModRecord {
    fn into_search_project(self) -> SearchProject {
        SearchProject {
            id: self.id,
            name: self.name,
            summary: self.summary.clone().unwrap_or_default(),
            slug: self.slug,
            class_id: self.class_id,
            primary_category_id: self.primary_category_id,
            website_url: self.links.and_then(|links| links.website_url),
            icon_url: self.logo.and_then(|logo| logo.thumbnail_url.or(logo.url)),
            download_count: self.download_count.unwrap_or(0.0).max(0.0).round() as u64,
            date_modified: self.date_modified,
        }
    }

    fn into_project(self) -> Project {
        Project {
            id: self.id,
            name: self.name,
            summary: self.summary.unwrap_or_default(),
            slug: self.slug,
            class_id: self.class_id,
            primary_category_id: self.primary_category_id,
            website_url: self.links.and_then(|links| links.website_url),
            icon_url: self.logo.and_then(|logo| logo.thumbnail_url.or(logo.url)),
            latest_files_indexes: self
                .latest_files_indexes
                .into_iter()
                .map(LatestFileIndexRecord::into_latest_file_index)
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LatestFileIndexRecord {
    file_id: u64,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    game_version: String,
    mod_loader: Option<u32>,
    game_version_type_id: Option<u32>,
}

impl LatestFileIndexRecord {
    fn into_latest_file_index(self) -> LatestFileIndex {
        LatestFileIndex {
            file_id: self.file_id,
            filename: self.filename,
            game_version: self.game_version,
            mod_loader: self.mod_loader,
            game_version_type_id: self.game_version_type_id,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModLinks {
    website_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModLogo {
    thumbnail_url: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileRecord {
    id: u64,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    file_name: String,
    #[serde(default)]
    file_date: String,
    download_count: Option<f64>,
    download_url: Option<String>,
    #[serde(default)]
    dependencies: Vec<FileDependencyRecord>,
    #[serde(default)]
    game_versions: Vec<String>,
}

impl FileRecord {
    fn into_file(self) -> File {
        File {
            id: self.id,
            display_name: self.display_name,
            file_name: self.file_name,
            file_date: self.file_date,
            download_count: self.download_count.unwrap_or(0.0).max(0.0).round() as u64,
            download_url: self.download_url,
            dependencies: self
                .dependencies
                .into_iter()
                .map(|dependency| FileDependency {
                    mod_id: dependency.mod_id,
                    relation_type: dependency.relation_type,
                })
                .collect(),
            game_versions: self.game_versions,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileDependencyRecord {
    mod_id: u64,
    relation_type: u32,
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn prepare_u64_ids(values: &[u64]) -> Vec<u64> {
    let mut prepared = Vec::new();
    for value in values {
        if *value == 0 || prepared.contains(value) {
            continue;
        }
        prepared.push(*value);
    }
    prepared
}
