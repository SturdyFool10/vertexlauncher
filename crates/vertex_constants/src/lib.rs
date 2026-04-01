use std::time::Duration;

pub mod app_paths {
    pub const APP_DIR_NAME: &str = "vertexlauncher";
    pub const LEGACY_APP_DIR_NAME: &str = "vertex-launcher";
    pub const CONFIG_DIR_NAME: &str = "config";
    pub const INSTANCES_FILENAME: &str = "instances.json";
    pub const INSTANCES_DIR_NAME: &str = "instances";
    pub const CACHE_DIR_NAME: &str = "cache";
    pub const LOGS_DIR_NAME: &str = "logs";
    pub const THEMES_DIR_NAME: &str = "themes";
}

pub mod auth {
    use super::Duration;

    /// Built-in client ID for device code sign-in. Overridden by `VERTEX_DEVICE_CODE_CLIENT_ID`.
    pub const BUILTIN_DEVICE_CODE_CLIENT_ID: &str = "2a674004-0bc7-4136-b863-def55befdfa2";
    /// Built-in Microsoft OAuth client id used when `VERTEX_MSA_CLIENT_ID` is not set.
    /// Leave empty to force env-based configuration.
    pub const BUILTIN_MICROSOFT_CLIENT_ID: &str = "00000000402b5328";
    /// Built-in OAuth tenant used when `VERTEX_MSA_TENANT` is not set.
    pub const BUILTIN_MICROSOFT_TENANT: &str = "consumers";
    /// Built-in tenant for device code sign-in. Overridden by `VERTEX_DEVICE_CODE_TENANT`.
    pub const BUILTIN_DEVICE_CODE_TENANT: &str = "consumers";

    pub const OAUTH_BASE_URL: &str = "https://login.microsoftonline.com";
    pub const LIVE_AUTHORIZE_URL: &str = "https://login.live.com/oauth20_authorize.srf";
    pub const LIVE_TOKEN_URL: &str = "https://login.live.com/oauth20_token.srf";
    pub const LIVE_REDIRECT_URI: &str = "https://login.live.com/oauth20_desktop.srf";
    pub const LIVE_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL offline_access";
    pub const DEVICE_CODE_SCOPE: &str = "XboxLive.signin offline_access";

    pub const XBOX_USER_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
    pub const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
    pub const MINECRAFT_LOGIN_URL: &str = "https://api.minecraftservices.com/launcher/login";
    pub const MINECRAFT_LOGIN_LEGACY_URL: &str =
        "https://api.minecraftservices.com/authentication/login_with_xbox";
    pub const MINECRAFT_ENTITLEMENTS_URL: &str =
        "https://api.minecraftservices.com/entitlements/mcstore";
    pub const MINECRAFT_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
    pub const MINECRAFT_PROFILE_SKINS_URL: &str =
        "https://api.minecraftservices.com/minecraft/profile/skins";
    pub const MINECRAFT_PROFILE_CAPE_ACTIVE_URL: &str =
        "https://api.minecraftservices.com/minecraft/profile/capes/active";

    pub const ACCOUNT_CACHE_FILENAME: &str = "account_cache.json";
    pub const ACCOUNT_CACHE_APP_DIR: &str = "vertexlauncher";
    pub const LEGACY_ACCOUNT_CACHE_APP_DIR: &str = "vertex-launcher";
    pub const LEGACY_ACCOUNT_CACHE_PATH: &str = "account_cache.json";

    pub const AUTH_REQUEST_MIN_INTERVAL: Duration = Duration::from_secs(2);
    pub const MAX_LOG_MESSAGE_CHARS: usize = 240;

    pub const ACCOUNTS_STATE_SERVICE: &str = "vertexlauncher.accounts_state.v1";
    pub const ACCOUNTS_STATE_ACCOUNT: &str = "cached_accounts";
    pub const REFRESH_TOKEN_SERVICE: &str = "vertexlauncher.microsoft_refresh_token.v2";
    pub const LEGACY_REFRESH_TOKEN_SERVICE: &str = "vertexlauncher.microsoft_refresh_token";
    pub const SECURE_STORE_RETRY_ATTEMPTS: usize = 5;
    pub const SECURE_STORE_RETRY_DELAY: Duration = Duration::from_millis(75);
    pub const REFRESH_TOKEN_VERIFY_ATTEMPTS: usize = 5;
    pub const REFRESH_TOKEN_STORE_ATTEMPTS: usize = 3;
}

pub mod branding {
    #[cfg(target_os = "linux")]
    pub const DESKTOP_APP_ID: &str = "io.github.SturdyFool10.VertexLauncher";
    #[cfg(not(target_os = "linux"))]
    pub const DESKTOP_APP_ID: &str = "vertexlauncher";

    pub const DISCORD_APPLICATION_ID: &str = "1486469547073601627";
}

pub mod instances {
    pub const STORE_FILENAME: &str = "instances.json";
    pub const DEFAULT_INSTANCE_NAME: &str = "Instance";
    pub const DEFAULT_MODLOADER: &str = "Vanilla";
    pub const DEFAULT_GAME_VERSION: &str = "latest";
}

pub mod content_resolver {
    pub const HASH_CACHE_DIR_NAME: &str = "cache";
    pub const HASH_CACHE_FILE_NAME: &str = "content_hash_cache.json";
    pub const LOOKUP_CACHE_KEY_PREFIX: &str = "lookup::";
    pub const HEURISTIC_WARNING_MESSAGE: &str =
        "Resolved from filename search. This match is heuristic and may be wrong.";
}

pub mod managed_content {
    pub const CONTENT_MANIFEST_FILE_NAME: &str = ".vertex-content-manifest.toml";
    pub const MODPACK_STATE_FILE_NAME: &str = ".vertex-modpack-state.toml";
}

pub mod vtmpack {
    pub const VTMPACK_EXTENSION: &str = "vtmpack";
    pub const VTMPACK_MANIFEST_VERSION: u32 = 1;
}

pub mod modrinth {
    use super::Duration;

    pub const API_BASE_URL: &str = "https://api.modrinth.com/v2";
    pub const USER_AGENT: &str =
        "VertexLauncher/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";
    pub const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(60);
    pub const MIN_REQUEST_SPACING: Duration = Duration::from_millis(250);
}

pub mod curseforge {
    use super::Duration;

    pub const API_BASE_URL: &str = "https://api.curseforge.com";
    pub const USER_AGENT: &str =
        "VertexLauncher/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";
    pub const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(60);
    pub const MIN_REQUEST_SPACING: Duration = Duration::from_millis(500);
    pub const DOWNLOAD_URL_LOOKUP_MAX_ATTEMPTS: usize = 3;
    pub const DOWNLOAD_URL_LOOKUP_RETRY_BASE_DELAY: Duration = Duration::from_millis(750);
    pub const MINECRAFT_GAME_ID: u32 = 432;
}

pub mod installation {
    use super::Duration;

    pub const USER_AGENT: &str =
        "VertexLauncher/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";
    pub const MOJANG_VERSION_MANIFEST_URL: &str =
        "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
    pub const FABRIC_VERSION_MATRIX_URL: &str = "https://meta.fabricmc.net/v2/versions/loader";
    pub const FABRIC_GAME_VERSIONS_URL: &str = "https://meta.fabricmc.net/v2/versions/game";
    pub const QUILT_VERSION_MATRIX_URL: &str = "https://meta.quiltmc.org/v3/versions/loader";
    pub const QUILT_GAME_VERSIONS_URL: &str = "https://meta.quiltmc.org/v3/versions/game";
    pub const FORGE_MAVEN_METADATA_URL: &str =
        "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml";
    pub const NEOFORGE_MAVEN_METADATA_URL: &str =
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml";
    pub const NEOFORGE_LEGACY_FORGE_METADATA_URL: &str =
        "https://maven.neoforged.net/releases/net/neoforged/forge/maven-metadata.xml";
    pub const CACHE_VERSION_CATALOG_RELEASES_FILE: &str = "version_catalog_release_only.json";
    pub const CACHE_VERSION_CATALOG_ALL_FILE: &str = "version_catalog_with_snapshots.json";
    pub const CACHE_LOADER_VERSIONS_DIR_NAME: &str = "loader_versions";
    pub const VERSION_CATALOG_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
    pub const HTTP_RETRY_ATTEMPTS: u32 = 4;
    pub const HTTP_RETRY_BASE_DELAY_MS: u64 = 350;
    pub const HTTP_TIMEOUT_GLOBAL: Duration = Duration::from_secs(45);
    pub const HTTP_TIMEOUT_CONNECT: Duration = Duration::from_secs(15);
    pub const HTTP_TIMEOUT_RECV_RESPONSE: Duration = Duration::from_secs(20);
    pub const HTTP_TIMEOUT_RECV_BODY: Duration = Duration::from_secs(45);
    pub const MAX_CONTENT_LENGTH_PROBES_PER_BATCH: usize = 32;
    pub const OPENJDK_USER_AGENT: &str =
        "VertexLauncher-JavaProvisioner/0.1 (+https://github.com/SturdyFool10/vertexlauncher)";

    #[cfg(target_os = "windows")]
    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
}

pub mod launcher {
    use super::Duration;

    pub mod single_instance {
        use super::Duration;

        pub const PORT: u16 = 38457;
        pub const HELLO_MESSAGE: &[u8] = b"vertexlauncher:hello:v1";
        pub const PRESENT_MESSAGE: &[u8] = b"vertexlauncher:present:v1";
        pub const PROBE_TIMEOUT: Duration = Duration::from_millis(250);
        pub const PROBE_ATTEMPTS: usize = 3;
        pub const RETRY_DELAY: Duration = Duration::from_millis(50);
    }

    pub mod system_browser_sign_in {
        use super::Duration;

        pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(15 * 60);
        pub const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
        pub const AUTO_CLOSE_DELAY_SECS: u64 = 4;
    }
}
