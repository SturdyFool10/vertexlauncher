use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::io::Read as _;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, warn};
use vertex_constants::modrinth::{
    API_BASE_URL as DEFAULT_MODRINTH_API_BASE_URL,
    MIN_REQUEST_SPACING as DEFAULT_MIN_REQUEST_SPACING,
    RATE_LIMIT_COOLDOWN as DEFAULT_RATE_LIMIT_COOLDOWN, USER_AGENT as DEFAULT_USER_AGENT,
};

use crate::response_records::{ProjectRecord, ProjectVersionRecord, SearchResponse};
use crate::{ModrinthError, Project, ProjectVersion, SearchProject};

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

fn rate_limit_store() -> &'static Mutex<RateLimitState> {
    static STORE: OnceLock<Mutex<RateLimitState>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(RateLimitState::new()))
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
        self.search_projects_with_filters(query, limit, offset, None, None, None, None)
    }

    /// Searches Modrinth projects with optional compatibility filters.
    ///
    /// - `project_type`: values such as `mod`, `resourcepack`, `shader`, `datapack`.
    /// - `game_version`: Minecraft version string (for example `1.20.1`).
    /// - `loader`: mod loader slug (for example `fabric`, `forge`, `neoforge`, `quilt`).
    /// - `sort_index`: Modrinth sort index — `"relevance"`, `"downloads"`, `"follows"`,
    ///   `"newest"`, or `"updated"`. Defaults to `"relevance"` when `None`.
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
        sort_index: Option<&str>,
    ) -> Result<Vec<SearchProject>, ModrinthError> {
        let trimmed = query.trim();
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
        if let Some(index) = non_empty(sort_index) {
            search_query.push(("index", index.to_owned()));
        }

        let response: SearchResponse = self.get_json("/search", &search_query)?;

        let projects: Vec<SearchProject> = response
            .hits
            .into_iter()
            .map(|hit| hit.into_search_project())
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

    /// Fetches multiple project records by ID or slug.
    pub fn get_projects(
        &self,
        project_ids_or_slugs: &[String],
    ) -> Result<Vec<Project>, ModrinthError> {
        let ids = prepare_string_ids(project_ids_or_slugs);
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        debug!(
            target: "vertexlauncher/modrinth",
            projects = ids.len(),
            "fetching Modrinth projects in batch"
        );
        let ids_json = serde_json::to_string(&ids).map_err(ModrinthError::Json)?;
        let records: Vec<ProjectRecord> = self.get_json("/projects", &[("ids", ids_json)])?;
        Ok(records
            .into_iter()
            .map(ProjectRecord::into_project)
            .collect())
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

    /// Fetches multiple version records by ID.
    pub fn get_versions(
        &self,
        version_ids: &[String],
    ) -> Result<Vec<ProjectVersion>, ModrinthError> {
        let ids = prepare_string_ids(version_ids);
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        debug!(
            target: "vertexlauncher/modrinth",
            versions = ids.len(),
            "fetching Modrinth versions in batch"
        );
        let ids_json = serde_json::to_string(&ids).map_err(ModrinthError::Json)?;
        let records: Vec<ProjectVersionRecord> =
            self.get_json("/versions", &[("ids", ids_json)])?;
        Ok(records
            .into_iter()
            .map(ProjectVersionRecord::into_project_version)
            .collect())
    }

    pub fn get_versions_from_hashes(
        &self,
        hashes: &[String],
        algorithm: &str,
    ) -> Result<HashMap<String, ProjectVersion>, ModrinthError> {
        let prepared_hashes = prepare_hashes(hashes);
        if prepared_hashes.is_empty() {
            return Ok(HashMap::new());
        }

        let algorithm = normalize_hash_algorithm(algorithm)?;
        debug!(
            target: "vertexlauncher/modrinth",
            hashes = prepared_hashes.len(),
            algorithm,
            "looking up Modrinth versions from file hashes"
        );

        let response: HashMap<String, ProjectVersionRecord> = self.post_json(
            "/version_files",
            &VersionFileLookupRequest {
                hashes: prepared_hashes.as_slice(),
                algorithm,
            },
        )?;
        if response.is_empty() {
            warn!(
                target: "vertexlauncher/modrinth",
                algorithm,
                hashes = prepared_hashes.len(),
                "Modrinth version-file lookup returned no matches"
            );
        }

        Ok(response
            .into_iter()
            .map(|(hash, version)| (hash, version.into_project_version()))
            .collect())
    }

    pub fn get_version_from_hash(
        &self,
        hash: &str,
        algorithm: &str,
    ) -> Result<Option<ProjectVersion>, ModrinthError> {
        let normalized_hash = normalize_hash(hash);
        if normalized_hash.is_empty() {
            return Ok(None);
        }

        let mut versions = self.get_versions_from_hashes(&[normalized_hash.clone()], algorithm)?;
        if versions.is_empty() {
            warn!(
                target: "vertexlauncher/modrinth",
                hash = normalized_hash,
                algorithm,
                "Modrinth hash lookup returned no version"
            );
        } else if versions.len() > 1 {
            warn!(
                target: "vertexlauncher/modrinth",
                hash = normalized_hash,
                matches = versions.len(),
                "Modrinth hash lookup returned multiple versions; using the requested hash key"
            );
        }

        Ok(versions.remove(normalized_hash.as_str()))
    }

    pub fn get_latest_versions_from_hashes(
        &self,
        hashes: &[String],
        algorithm: &str,
        loaders: &[String],
        game_versions: &[String],
    ) -> Result<HashMap<String, ProjectVersion>, ModrinthError> {
        let prepared_hashes = prepare_hashes(hashes);
        if prepared_hashes.is_empty() {
            return Ok(HashMap::new());
        }

        let algorithm = normalize_hash_algorithm(algorithm)?;
        debug!(
            target: "vertexlauncher/modrinth",
            hashes = prepared_hashes.len(),
            algorithm,
            loaders = loaders.len(),
            game_versions = game_versions.len(),
            "looking up latest compatible Modrinth versions from file hashes"
        );

        let response: HashMap<String, ProjectVersionRecord> = self.post_json(
            "/version_files/update",
            &VersionFileUpdateRequest {
                hashes: prepared_hashes.as_slice(),
                algorithm,
                loaders,
                game_versions,
            },
        )?;
        if response.is_empty() {
            warn!(
                target: "vertexlauncher/modrinth",
                algorithm,
                hashes = prepared_hashes.len(),
                "Modrinth version-file update lookup returned no matches"
            );
        }

        Ok(response
            .into_iter()
            .map(|(hash, version)| (hash, version.into_project_version()))
            .collect())
    }

    pub fn get_latest_version_from_hash(
        &self,
        hash: &str,
        algorithm: &str,
        loaders: &[String],
        game_versions: &[String],
    ) -> Result<Option<ProjectVersion>, ModrinthError> {
        let normalized_hash = normalize_hash(hash);
        if normalized_hash.is_empty() {
            return Ok(None);
        }

        let mut versions = self.get_latest_versions_from_hashes(
            &[normalized_hash.clone()],
            algorithm,
            loaders,
            game_versions,
        )?;
        Ok(versions.remove(normalized_hash.as_str()))
    }

    /// Executes a GET request and deserializes the JSON body.
    ///
    /// `path` is appended to the configured `base_url`.
    fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, ModrinthError> {
        self.acquire_rate_limit_slot(path);
        debug!(
            target: "vertexlauncher/modrinth",
            path,
            query_count = query.len(),
            "sending Modrinth GET request"
        );

        let mut request = self
            .agent
            .get(&format!("{}{}", self.base_url, path))
            .header("User-Agent", &self.user_agent);

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
    ) -> Result<T, ModrinthError> {
        self.acquire_rate_limit_slot(path);
        debug!(
            target: "vertexlauncher/modrinth",
            path,
            "sending Modrinth POST request"
        );

        let request = self
            .agent
            .post(&format!("{}{}", self.base_url, path))
            .header("User-Agent", &self.user_agent);
        self.read_json_response(
            path,
            request
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
    ) -> Result<T, ModrinthError> {
        let mut response = match response_result {
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
        let retry_after_secs = parse_retry_after_seconds(response.headers());
        self.note_response_budget(response.headers());
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
        if status == 429 {
            self.note_rate_limit(path, retry_after_secs);
            return Err(ModrinthError::rate_limited(
                retry_after_secs.or(Some(DEFAULT_RATE_LIMIT_COOLDOWN.as_secs())),
            ));
        }
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

    fn acquire_rate_limit_slot(&self, path: &str) {
        loop {
            let Some(wait) = self.reserve_next_request_slot() else {
                return;
            };
            debug!(
                target: "vertexlauncher/modrinth",
                path,
                wait_ms = wait.as_millis() as u64,
                "waiting for Modrinth rate-limit slot"
            );
            thread::sleep(wait);
        }
    }

    fn reserve_next_request_slot(&self) -> Option<Duration> {
        let Ok(mut guard) = rate_limit_store().lock() else {
            warn!(target: "vertexlauncher/modrinth", "rate_limit_store mutex was poisoned");
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
        let cooldown = Duration::from_secs(retry_after_secs.unwrap_or(60).max(1));
        let until = Instant::now() + cooldown.max(DEFAULT_RATE_LIMIT_COOLDOWN);
        let Ok(mut guard) = rate_limit_store().lock() else {
            warn!(target: "vertexlauncher/modrinth", "rate_limit_store mutex was poisoned");
            return;
        };
        if guard.cooldown_until.is_none_or(|existing| existing < until) {
            guard.cooldown_until = Some(until);
        }
        guard.next_request_at = guard.next_request_at.max(until);
        warn!(
            target: "vertexlauncher/modrinth",
            path,
            retry_after_secs = retry_after_secs.unwrap_or(DEFAULT_RATE_LIMIT_COOLDOWN.as_secs()),
            "Modrinth rate limited request; backing off further API calls"
        );
    }

    fn note_response_budget(&self, headers: &ureq::http::HeaderMap) {
        let Some(reset_secs) = parse_ratelimit_reset_seconds(headers) else {
            return;
        };
        let Some(remaining) = parse_ratelimit_remaining(headers) else {
            return;
        };
        let wait = rate_limit_wait_from_budget(reset_secs, remaining);
        let Ok(mut guard) = rate_limit_store().lock() else {
            warn!(target: "vertexlauncher/modrinth", "rate_limit_store mutex was poisoned");
            return;
        };
        let next_request_at = Instant::now() + wait;
        if guard.next_request_at < next_request_at {
            guard.next_request_at = next_request_at;
        }
        if remaining == 0 {
            let cooldown_until = Instant::now() + Duration::from_secs(reset_secs.max(1));
            if guard
                .cooldown_until
                .is_none_or(|existing| existing < cooldown_until)
            {
                guard.cooldown_until = Some(cooldown_until);
            }
        }
    }
}

#[derive(Serialize)]
struct VersionFileLookupRequest<'a> {
    hashes: &'a [String],
    algorithm: &'a str,
}

#[derive(Serialize)]
struct VersionFileUpdateRequest<'a> {
    hashes: &'a [String],
    algorithm: &'a str,
    #[serde(skip_serializing_if = "string_slice_is_empty")]
    loaders: &'a [String],
    #[serde(skip_serializing_if = "string_slice_is_empty")]
    game_versions: &'a [String],
}

fn string_slice_is_empty(values: &[String]) -> bool {
    values.is_empty()
}

// HTTP header names are case-insensitive; HeaderMap::get normalises to
// lowercase internally, so there is no need for a case-variant fallback.
fn parse_retry_after_seconds(headers: &ureq::http::HeaderMap) -> Option<u64> {
    headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn parse_ratelimit_remaining(headers: &ureq::http::HeaderMap) -> Option<u64> {
    headers
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn parse_ratelimit_reset_seconds(headers: &ureq::http::HeaderMap) -> Option<u64> {
    headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn rate_limit_wait_from_budget(reset_secs: u64, remaining: u64) -> Duration {
    if remaining == 0 {
        return Duration::from_secs(reset_secs.max(1));
    }

    let window_millis = u128::from(reset_secs) * 1000;
    if window_millis == 0 {
        return DEFAULT_MIN_REQUEST_SPACING;
    }

    let min_spacing_millis = DEFAULT_MIN_REQUEST_SPACING.as_millis();
    let per_request_millis = window_millis.div_ceil(u128::from(remaining));
    Duration::from_millis(per_request_millis.max(min_spacing_millis) as u64)
}

fn normalize_hash_algorithm(value: &str) -> Result<&'static str, ModrinthError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "sha1" => Ok("sha1"),
        "sha512" => Ok("sha512"),
        invalid => Err(ModrinthError::InvalidHashAlgorithm(invalid.to_owned())),
    }
}

fn prepare_hashes(hashes: &[String]) -> Vec<String> {
    hashes
        .iter()
        .map(String::as_str)
        .map(normalize_hash)
        .filter(|hash| !hash.is_empty())
        .collect()
}

fn prepare_string_ids(values: &[String]) -> Vec<String> {
    let mut prepared = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() || prepared.iter().any(|existing| existing == trimmed) {
            continue;
        }
        prepared.push(trimmed.to_owned());
    }
    prepared
}

fn normalize_hash(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_budget_keeps_subsecond_spacing() {
        let wait = rate_limit_wait_from_budget(60, 299);
        assert_eq!(wait, DEFAULT_MIN_REQUEST_SPACING);
    }

    #[test]
    fn rate_limit_budget_expands_spacing_when_budget_is_tight() {
        let wait = rate_limit_wait_from_budget(60, 10);
        assert_eq!(wait, Duration::from_secs(6));
    }

    #[test]
    fn rate_limit_budget_uses_reset_window_for_empty_budget() {
        let wait = rate_limit_wait_from_budget(17, 0);
        assert_eq!(wait, Duration::from_secs(17));
    }
}
