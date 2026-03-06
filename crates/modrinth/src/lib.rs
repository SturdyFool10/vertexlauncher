use serde::Deserialize;
use serde::de::DeserializeOwned;

const DEFAULT_MODRINTH_API_BASE_URL: &str = "https://api.modrinth.com/v2";
const DEFAULT_USER_AGENT: &str =
    "VertexLauncher/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";

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

#[derive(Clone, Debug)]
pub struct Client {
    agent: ureq::Agent,
    base_url: String,
    user_agent: String,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            agent: ureq::Agent::new(),
            base_url: DEFAULT_MODRINTH_API_BASE_URL.to_owned(),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
        }
    }
}

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
}

impl Client {
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn list_project_types(&self) -> Result<Vec<String>, ModrinthError> {
        self.get_json("/tag/project_type", &[])
    }

    pub fn search_projects(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SearchProject>, ModrinthError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let limit = limit.clamp(1, 100);
        let response: SearchResponse = self.get_json(
            "/search",
            &[
                ("query", trimmed.to_owned()),
                ("limit", limit.to_string()),
                ("offset", offset.to_string()),
            ],
        )?;

        Ok(response
            .hits
            .into_iter()
            .map(SearchHit::into_search_project)
            .collect())
    }

    fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, ModrinthError> {
        let mut request = self
            .agent
            .get(&format!("{}{}", self.base_url, path))
            .set("User-Agent", &self.user_agent);

        for (key, value) in query {
            request = request.query(key, value);
        }

        let response = match request.call() {
            Ok(ok) => ok,
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                return Err(ModrinthError::HttpStatus { status, body });
            }
            Err(ureq::Error::Transport(transport)) => {
                return Err(ModrinthError::Transport(transport.to_string()));
            }
        };

        let raw = response.into_string()?;
        Ok(serde_json::from_str(&raw)?)
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
}

impl SearchHit {
    fn into_search_project(self) -> SearchProject {
        let canonical_slug = self
            .slug
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(self.project_id.as_str());
        let canonical_type = self.project_type.trim();
        let project_url = format!("https://modrinth.com/{canonical_type}/{canonical_slug}");

        SearchProject {
            project_id: self.project_id,
            slug: self.slug,
            title: self.title,
            description: self.description,
            project_type: self.project_type,
            icon_url: self.icon_url,
            author: self.author,
            project_url,
        }
    }
}
