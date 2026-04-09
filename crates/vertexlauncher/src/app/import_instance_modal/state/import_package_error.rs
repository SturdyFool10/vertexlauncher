use super::*;

#[derive(Clone, Debug)]
pub enum ImportPackageError {
    Message(String),
    ManualCurseForgeDownloads {
        requirements: Vec<CurseForgeManualDownloadRequirement>,
        staged_files: HashMap<u64, PathBuf>,
    },
}

impl ImportPackageError {
    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl std::fmt::Display for ImportPackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
            Self::ManualCurseForgeDownloads { requirements, .. } => write!(
                f,
                "{} CurseForge files require manual download",
                requirements.len()
            ),
        }
    }
}

impl std::error::Error for ImportPackageError {}

impl From<String> for ImportPackageError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for ImportPackageError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_owned())
    }
}
