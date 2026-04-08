use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Read, Write};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(target_os = "windows")]
use vertex_constants::installation::CREATE_NO_WINDOW;
use vertex_constants::installation::{
    CACHE_LOADER_VERSIONS_DIR_NAME, CACHE_VERSION_CATALOG_ALL_FILE,
    CACHE_VERSION_CATALOG_RELEASES_FILE, FABRIC_GAME_VERSIONS_URL, FABRIC_VERSION_MATRIX_URL,
    FORGE_MAVEN_METADATA_URL, HTTP_RETRY_ATTEMPTS, HTTP_RETRY_BASE_DELAY_MS, HTTP_TIMEOUT_CONNECT,
    HTTP_TIMEOUT_GLOBAL, HTTP_TIMEOUT_RECV_BODY, HTTP_TIMEOUT_RECV_RESPONSE,
    MAX_CONTENT_LENGTH_PROBES_PER_BATCH, MOJANG_VERSION_MANIFEST_URL,
    NEOFORGE_LEGACY_FORGE_METADATA_URL, NEOFORGE_MAVEN_METADATA_URL, OPENJDK_USER_AGENT,
    QUILT_GAME_VERSIONS_URL, QUILT_VERSION_MATRIX_URL, USER_AGENT as DEFAULT_USER_AGENT,
    VERSION_CATALOG_CACHE_TTL,
};

mod fs_support;
mod installation_core;
mod launch_engine;
mod process_control;

pub use fs_support::*;
pub use installation_core::*;
pub(crate) use launch_engine::*;
pub use process_control::*;
