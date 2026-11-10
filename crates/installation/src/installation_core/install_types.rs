use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameSetupResult {
    pub version_json_path: PathBuf,
    pub client_jar_path: PathBuf,
    pub downloaded_files: u32,
    pub resolved_modloader_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    pub instance_root: PathBuf,
    pub game_version: String,
    pub modloader: String,
    pub modloader_version: Option<String>,
    pub account_key: Option<String>,
    pub java_executable: Option<String>,
    pub max_memory_mib: u128,
    pub extra_jvm_args: Option<String>,
    pub extra_env_vars: Option<String>,
    pub player_name: Option<String>,
    pub player_uuid: Option<String>,
    pub auth_access_token: Option<String>,
    pub auth_xuid: Option<String>,
    pub auth_user_type: Option<String>,
    pub quick_play_singleplayer: Option<String>,
    pub quick_play_multiplayer: Option<String>,
    pub linux_set_opengl_driver: bool,
    pub linux_use_zink_driver: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchResult {
    pub pid: u32,
    pub profile_id: String,
    pub launch_log_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishedInstanceProcess {
    pub instance_root: String,
    pub account_key: Option<String>,
    pub pid: u32,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadPolicy {
    pub max_concurrent_downloads: u32,
    pub max_download_bps: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadBatchTask {
    pub url: String,
    pub destination: PathBuf,
    pub expected_size: Option<u64>,
}

static SHARED_DOWNLOAD_POLICY: std::sync::Mutex<Option<DownloadPolicy>> = std::sync::Mutex::new(None);

/// Sets the download limits (concurrent transfers, speed limit) from the user's settings for code that has no config
/// of its own at hand, such as the importers. The app refreshes it whenever the settings are read.
pub fn set_shared_download_policy(policy: DownloadPolicy) {
    *SHARED_DOWNLOAD_POLICY.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(policy);
}

/// The limits last given to [`set_shared_download_policy`], or the defaults before that.
pub fn shared_download_policy() -> DownloadPolicy {
    SHARED_DOWNLOAD_POLICY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .unwrap_or_default()
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: 8,
            max_download_bps: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallStage {
    PreparingFolders,
    ResolvingMetadata,
    DownloadingCore,
    InstallingModloader,
    Complete,
}

#[derive(Clone, Debug)]
pub struct InstallProgress {
    pub stage: InstallStage,
    pub message: String,
    pub downloaded_files: u32,
    pub total_files: u32,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub bytes_per_second: f64,
    pub eta_seconds: Option<u64>,
}

pub type InstallProgressCallback = Arc<dyn Fn(InstallProgress) + Send + Sync + 'static>;

pub(crate) type InstallProgressSink = dyn Fn(InstallProgress) + Send + Sync + 'static;
