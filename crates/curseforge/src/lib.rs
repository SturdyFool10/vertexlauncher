use serde::Deserialize;
use serde::de::DeserializeOwned;
use tracing::{debug, warn};

const DEFAULT_CURSEFORGE_API_BASE_URL: &str = "https://api.curseforge.com";
const DEFAULT_USER_AGENT: &str =
    "VertexLauncher/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";
pub const MINECRAFT_GAME_ID: u32 = 432;

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
            agent: ureq::Agent::new(),
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
            "searching CurseForge projects"
        );
        let response: DataResponse<Vec<ModRecord>> = self.get_json(
            "/v1/mods/search",
            &[
                ("gameId", game_id.to_string()),
                ("searchFilter", trimmed.to_owned()),
                ("index", index.to_string()),
                ("pageSize", page_size.to_string()),
            ],
        )?;

        let projects: Vec<SearchProject> = response
            .data
            .into_iter()
            .map(|record| SearchProject {
                id: record.id,
                name: record.name,
                summary: record.summary.unwrap_or_default(),
                slug: record.slug,
                class_id: record.class_id,
                primary_category_id: record.primary_category_id,
                website_url: record.links.and_then(|links| links.website_url),
                icon_url: record.logo.and_then(|logo| logo.thumbnail_url.or(logo.url)),
            })
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

    /// Executes a GET request and deserializes the JSON body.
    ///
    /// `path` is appended to the configured `base_url`.
    fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, CurseForgeError> {
        debug!(
            target: "vertexlauncher/curseforge",
            path,
            query_count = query.len(),
            "sending CurseForge request"
        );

        let mut request = self
            .agent
            .get(&format!("{}{}", self.base_url, path))
            .set("User-Agent", &self.user_agent)
            .set("x-api-key", &self.api_key);

        for (key, value) in query {
            request = request.query(key, value);
        }

        let response = match request.call() {
            Ok(ok) => ok,
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                warn!(
                    target: "vertexlauncher/curseforge",
                    path,
                    status,
                    body_len = body.len(),
                    "CurseForge returned non-success status"
                );
                return Err(CurseForgeError::HttpStatus { status, body });
            }
            Err(ureq::Error::Transport(transport)) => {
                warn!(
                    target: "vertexlauncher/curseforge",
                    path,
                    error = %transport,
                    "CurseForge transport error"
                );
                return Err(CurseForgeError::Transport(transport.to_string()));
            }
        };

        let raw = response.into_string().map_err(|err| {
            warn!(
                target: "vertexlauncher/curseforge",
                path,
                error = %err,
                "failed to read CurseForge response body"
            );
            CurseForgeError::Read(err)
        })?;

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
}

#[derive(Debug, Deserialize)]
struct DataResponse<T> {
    data: T,
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
