use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use content_resolver::{InstalledContentKind, detect_installed_content_kind};
use curseforge::Client as CurseForgeClient;
use eframe::egui;
use instances::{
    InstanceRecord, InstanceStore, NewInstanceSpec, create_instance, delete_instance,
    instance_root_path,
};
use launcher_runtime as tokio_runtime;
use launcher_ui::{ui::components::settings_widgets, ui::style};
use managed_content::{
    CONTENT_MANIFEST_FILE_NAME, ContentInstallManifest, InstalledContentProject,
    ManagedContentSource, ModpackInstallState, load_content_manifest, load_modpack_install_state,
    remove_modpack_install_state, save_content_manifest, save_modpack_install_state,
};
use modrinth::Client as ModrinthClient;
use serde::Deserialize;
use serde_json::Value;
use textui::TextUi;
use textui_egui::prelude::*;
use ui_foundation::{DialogPreset, dialog_options, primary_button, secondary_button, show_dialog};
use vtmpack::{VtmpackDownloadableEntry, VtmpackManifest, read_vtmpack_manifest};

const MODAL_GAP_SM: f32 = 6.0;
const MODAL_GAP_MD: f32 = 8.0;
const MODAL_GAP_LG: f32 = 10.0;
const ACTION_BUTTON_MAX_WIDTH: f32 = 260.0;
const MODRINTH_DOWNLOAD_MIN_SPACING: Duration = Duration::from_millis(250);
const CURSEFORGE_DOWNLOAD_MIN_SPACING: Duration = Duration::from_millis(500);

#[track_caller]
fn fs_create_dir_all_logged(path: &Path) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display());
    let result = fs::create_dir_all(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_read_dir_logged(path: &Path) -> std::io::Result<fs::ReadDir> {
    tracing::debug!(target: "vertexlauncher/io", op = "read_dir", path = %path.display());
    let result = fs::read_dir(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_dir", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_read_to_string_logged(path: &Path) -> std::io::Result<String> {
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    let result = fs::read_to_string(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_copy_logged(source: &Path, destination: &Path) -> std::io::Result<u64> {
    tracing::debug!(target: "vertexlauncher/io", op = "copy", from = %source.display(), to = %destination.display());
    let result = fs::copy(source, destination);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "copy", from = %source.display(), to = %destination.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_write_logged(path: &Path, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "write", path = %path.display());
    let result = fs::write(path, bytes);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "write", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_remove_dir_all_logged(path: &Path) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "remove_dir_all", path = %path.display());
    let result = fs::remove_dir_all(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "remove_dir_all", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_rename_logged(source: &Path, destination: &Path) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "rename", from = %source.display(), to = %destination.display());
    let result = fs::rename(source, destination);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "rename", from = %source.display(), to = %destination.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_file_open_logged(path: &Path) -> std::io::Result<fs::File> {
    tracing::debug!(target: "vertexlauncher/io", op = "file_open", path = %path.display());
    let result = fs::File::open(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "file_open", path = %path.display(), error = %err);
    }
    result
}

mod inspection;
mod package_import;
mod render_impl;
mod state;

pub use package_import::{
    attach_curseforge_modpack_install_state,
    format_curseforge_download_url_error,
    prepare_curseforge_manual_download_for_file,
    prepare_curseforge_manual_downloads,
    CurseForgeManualDownloadRequirement,
};
pub use render_impl::import_package_with_progress;
pub use render_impl::render;
pub use state::{
    ImportInstanceState,
    ImportPackageError,
    ImportProgress,
    ImportRequest,
    ImportSource,
    ImportTaskResult,
    ModalAction,
};

use self::{inspection::*, package_import::*, state::*};

