/// Errors returned by Modrinth API requests.
#[derive(Debug, thiserror::Error)]
pub enum ModrinthError {
    #[error("Modrinth API rate limited by upstream{retry_after_suffix}")]
    RateLimited { retry_after_suffix: String },
    #[error("HTTP status {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("HTTP transport error: {0}")]
    Transport(String),
    #[error("Response read error: {0}")]
    Read(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid Modrinth hash algorithm: {0}")]
    InvalidHashAlgorithm(String),
}

impl ModrinthError {
    #[must_use]
    pub fn rate_limited(retry_after_secs: Option<u64>) -> Self {
        let retry_after_suffix = retry_after_secs
            .map(|secs| format!("; retry after {secs}s"))
            .unwrap_or_default();
        Self::RateLimited { retry_after_suffix }
    }

    #[must_use]
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited { .. })
    }
}
