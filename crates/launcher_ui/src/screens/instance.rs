use vertex_i18n::tr;
use config::{
    Config, INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN,
    JavaRuntimeVersion,
};
use content_resolver::{
    InstalledContentFile, InstalledContentHashCache, InstalledContentKind,
    InstalledContentResolver, ResolveInstalledContentRequest,
};
use directories::UserDirs;
use flate2::read::GzDecoder;
use installation::{
    DownloadPolicy, InstallProgress, InstallProgressCallback, InstallStage, LaunchRequest,
    LoaderSupportIndex, LoaderVersionIndex, MinecraftVersionEntry, VersionCatalog,
    VersionCatalogFilter, display_user_path, ensure_game_files_async, ensure_openjdk_runtime_async,
    fetch_loader_versions_for_game, fetch_version_catalog_with_refresh, launch_instance, normalize_path_key,
};
use instances::{
    InstanceStore, record_instance_launch_usage, set_instance_settings, set_instance_versions,
};
use managed_content::load_managed_content_identities;
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsStr,
    fs,
    hash::Hasher,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use vtmpack::{
    VtmpackInstanceMetadata, apply_vtmpatch_to_instance, export_instance_as_vtmpack_with_progress,
    export_instance_as_vtmpatch_with_progress, list_exportable_root_entries,
};

use crate::app::tokio_runtime;
use crate::desktop;
use crate::{console, install_activity, notification};

pub mod content;
mod content_lookup_result;
#[path = "instance/instance_api.rs"]
pub mod api;
mod installed_content_cache;
mod installed_entry_render_result;
#[path = "instance_exports.rs"]
mod instance_exports;
#[path = "instance_logs.rs"]
mod instance_logs;
#[path = "instance_move_modal.rs"]
mod instance_move_modal;
#[path = "instance/instance_presence_section.rs"]
mod instance_presence_section;
mod instance_screen_output;
mod instance_screen_state;
#[path = "instance_screenshots.rs"]
mod instance_screenshots;
#[path = "instance_settings_modal.rs"]
mod instance_settings_modal;
mod instance_version_detection;
mod move_instance;
mod platform;
mod root_entries;
mod runtime;
mod runtime_prepare_operation;
mod runtime_prepare_outcome;
mod server_export_environment;

pub use self::instance_presence_section::InstancePresenceSection;
pub use instance_version_detection::{DetectedInstanceVersions, detect_instance_versions};

use content::ContentApplyResult;
use content::poll_content_lookup_results;
use content_lookup_result::ContentLookupResult;
use installed_content_cache::{InstalledContentCache, InstalledContentScanResult};
use instance_exports::*;
use instance_logs::*;
use instance_move_modal::*;
use instance_screen_state::{
    InstanceLogEntry, InstanceScreenState,
    InstanceScreenshotEntry, MoveInstanceResult,
    ServerExportOutcome, VtmpackExportOutcome,
};
use instance_screenshots::*;
use move_instance::{
    poll_move_instance_progress, poll_move_instance_results, request_move_instance,
};
use runtime::*;
use platform::effective_linux_graphics_settings_for_state;
use platform::linux_instance_driver_settings_for_save;
use runtime_prepare_operation::RuntimePrepareOperation;
use runtime_prepare_outcome::RuntimePrepareOutcome;

use super::platform as screen_platform;

const RESERVED_SYSTEM_MEMORY_MIB: u128 = 4 * 1024;
const FALLBACK_TOTAL_MEMORY_MIB: u128 = 20 * 1024;
const MODLOADER_OPTIONS: [&str; 6] = ["Vanilla", "Fabric", "Forge", "NeoForge", "Quilt", "Custom"];
const CUSTOM_MODLOADER_INDEX: usize = MODLOADER_OPTIONS.len() - 1;

const INSTALLED_CONTENT_PAGE_SIZES: [usize; 4] = [10, 25, 50, 100];

const INSTANCE_SCREENSHOT_SCAN_INTERVAL: Duration = Duration::from_secs(10);
const INSTANCE_LOG_SCAN_INTERVAL: Duration = Duration::from_secs(3);







const MAX_INSTANCE_LOG_LINES: usize = 12_000;



































fn export_instance_as_server_zip_with_progress<F>(
    instance: &instances::InstanceRecord,
    instance_root: &Path,
    output_path: &Path,
    included_root_entries: &BTreeMap<String, bool>,
    force_java_21_minimum: bool,
    mut progress: F,
) -> Result<String, String>
where
    F: FnMut(vtmpack::VtmpackExportProgress),
{
    progress(vtmpack::VtmpackExportProgress {
        message: tr!("Scanning instance files...").into_owned(),
        completed_steps: 0,
        total_steps: 1,
    });
    let mut included_files = collect_server_export_files(instance_root, included_root_entries)?;
    let manifest = managed_content::load_content_manifest(instance_root);
    progress(vtmpack::VtmpackExportProgress {
        message: tr!("Checking which mods are client-only...").into_owned(),
        completed_steps: 0,
        total_steps: 1,
    });
    let client_only =
        server_export_environment::find_client_only_mods(instance_root, &manifest, &included_files);
    included_files.retain(|file| !client_only.excluded.contains(file));
    let unknowns = classify_curseforge_unknown_mods_for_server_export(
        instance_root,
        &manifest,
        &mut progress,
        included_files.len(),
    );

    let required_java = config::JavaRuntimeVersion::required_major_for_game(
        instance.game_version.as_str(),
    )
    .map(|major| {
        if force_java_21_minimum && major < 21 {
            21
        } else {
            major
        }
    });

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|err| {
            tr!("failed to create server export directory {parent}: {err}", parent = parent.display(), err = err).into_owned()
        })?;
    }

    let output_file = fs::File::create(output_path)
        .map_err(|err| tr!("failed to create {output_path}: {err}", output_path = output_path.display(), err = err).into_owned())?;
    let mut zip = zip::ZipWriter::new(output_file);
    let file_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let script_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    let mut completed_steps = 0usize;
    let total_steps = included_files.len() + 5;
    for file in &included_files {
        let relative = file.strip_prefix(instance_root).unwrap_or(file.as_path());
        let zip_path = normalize_zip_path(relative);
        progress(vtmpack::VtmpackExportProgress {
            message: tr!("Adding {relative}", relative = relative.display()).into_owned(),
            completed_steps,
            total_steps,
        });
        zip.start_file(zip_path.as_str(), file_options)
            .map_err(|err| tr!("failed to add {zip_path} to zip: {err}", zip_path = zip_path, err = err).into_owned())?;
        let mut input = fs::File::open(file.as_path())
            .map_err(|err| tr!("failed to open {file}: {err}", file = file.display(), err = err).into_owned())?;
        std::io::copy(&mut input, &mut zip)
            .map_err(|err| tr!("failed to write {file} to zip: {err}", file = file.display(), err = err).into_owned())?;
        completed_steps += 1;
    }

    progress(vtmpack::VtmpackExportProgress {
        message: tr!("Writing launch scripts...").into_owned(),
        completed_steps,
        total_steps,
    });
    zip.start_file("start-server.sh", script_options)
        .map_err(|err| tr!("failed to add start-server.sh: {err}", err = err).into_owned())?;
    zip.write_all(build_server_start_script_sh(required_java).as_bytes())
        .map_err(|err| tr!("failed to write start-server.sh: {err}", err = err).into_owned())?;
    completed_steps += 1;

    zip.start_file("start-server.bat", script_options)
        .map_err(|err| tr!("failed to add start-server.bat: {err}", err = err).into_owned())?;
    zip.write_all(build_server_start_script_bat(required_java).as_bytes())
        .map_err(|err| tr!("failed to write start-server.bat: {err}", err = err).into_owned())?;
    completed_steps += 1;

    progress(vtmpack::VtmpackExportProgress {
        message: tr!("Writing server build report...").into_owned(),
        completed_steps,
        total_steps,
    });
    zip.start_file("VERTEX_SERVER_BUILD_REPORT.txt", file_options)
        .map_err(|err| tr!("failed to add VERTEX_SERVER_BUILD_REPORT.txt: {err}", err = err).into_owned())?;
    zip.write_all(
        build_server_export_report(
            instance,
            included_root_entries,
            required_java,
            &unknowns,
            &client_only,
        )
        .as_bytes(),
    )
    .map_err(|err| tr!("failed to write server build report: {err}", err = err).into_owned())?;
    completed_steps += 1;

    progress(vtmpack::VtmpackExportProgress {
        message: tr!("Writing EULA placeholder...").into_owned(),
        completed_steps,
        total_steps,
    });
    zip.start_file("eula.txt", file_options)
        .map_err(|err| tr!("failed to add eula.txt: {err}", err = err).into_owned())?;
    zip.write_all(b"eula=false\n")
        .map_err(|err| tr!("failed to write eula.txt: {err}", err = err).into_owned())?;
    completed_steps += 1;

    progress(vtmpack::VtmpackExportProgress {
        message: tr!("Finalizing zip archive...").into_owned(),
        completed_steps,
        total_steps,
    });
    let _ = zip
        .finish()
        .map_err(|err| tr!("failed to finalize zip archive: {err}", err = err).into_owned())?;
    completed_steps += 1;

    progress(vtmpack::VtmpackExportProgress {
        message: tr!("Server export complete.").into_owned(),
        completed_steps,
        total_steps,
    });
    Ok(tr!("{count} files included, {count2} client-only mods left out, {count3} CurseForge-managed unknown mods flagged.", count = included_files.len(), count2 = client_only.excluded.len(), count3 = unknowns.len()).into_owned())
}

fn collect_server_export_files(
    instance_root: &Path,
    included_root_entries: &BTreeMap<String, bool>,
) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for (entry, included) in included_root_entries {
        if !*included {
            continue;
        }
        let path = instance_root.join(entry);
        if !path.exists() {
            continue;
        }
        if path.is_file() {
            files.push(path);
            continue;
        }
        collect_regular_files_recursive(path.as_path(), &mut files)
            .map_err(|err| tr!("failed to collect files under {path}: {err}", path = path.display(), err = err).into_owned())?;
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn collect_regular_files_recursive(root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let entries = fs::read_dir(root)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_regular_files_recursive(path.as_path(), out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn classify_curseforge_unknown_mods_for_server_export<F>(
    instance_root: &Path,
    manifest: &managed_content::ContentInstallManifest,
    progress: &mut F,
    initial_step_offset: usize,
) -> Vec<String>
where
    F: FnMut(vtmpack::VtmpackExportProgress),
{
    let candidates = manifest
        .projects
        .values()
        .filter(|project| {
            project.selected_source == Some(managed_content::ManagedContentSource::CurseForge)
                && normalize_path_key(project.file_path.as_path()).starts_with("mods/")
        })
        .map(|project| {
            let display_name = project.name.trim().to_owned();
            let name = if display_name.is_empty() {
                project.file_path.to_string_lossy().into_owned()
            } else {
                display_name
            };
            (
                name,
                instance_root.join(project.file_path.as_path()),
                project.file_path.to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>();
    let mut unknowns = Vec::new();
    if candidates.is_empty() {
        return unknowns;
    }

    let modrinth = modrinth::Client::default();
    let total_steps = initial_step_offset + candidates.len() + 1;
    let mut completed = initial_step_offset;
    for (name, path, relative) in candidates {
        progress(vtmpack::VtmpackExportProgress {
            message: tr!("Classifying CurseForge mod {relative}...", relative = relative).into_owned(),
            completed_steps: completed,
            total_steps,
        });
        completed += 1;

        if !path.is_file() {
            unknowns.push(tr!("{name} ({relative}) - file missing", name = name, relative = relative).into_owned());
            continue;
        }
        let hashes = match modrinth::hash_file_sha1_and_sha512_hex(path.as_path()) {
            Ok(values) => values,
            Err(error) => {
                unknowns.push(tr!("{name} ({relative}) - hash failed: {error}", name = name, relative = relative, error = error).into_owned());
                continue;
            }
        };
        let (sha1, sha512) = hashes;
        let matched = modrinth
            .get_version_from_hash(sha512.as_str(), "sha512")
            .ok()
            .flatten()
            .is_some()
            || modrinth
                .get_version_from_hash(sha1.as_str(), "sha1")
                .ok()
                .flatten()
                .is_some();
        if !matched {
            unknowns.push(tr!("{name} ({relative})", name = name, relative = relative).into_owned());
        }
    }
    progress(vtmpack::VtmpackExportProgress {
        message: tr!("CurseForge unknown-classification complete.").into_owned(),
        completed_steps: completed,
        total_steps,
    });
    unknowns
}

fn build_server_export_report(
    instance: &instances::InstanceRecord,
    included_root_entries: &BTreeMap<String, bool>,
    required_java: Option<u8>,
    unknowns: &[String],
    client_only: &server_export_environment::ClientOnlyMods,
) -> String {
    let mut report = String::new();
    report.push_str("Vertex Auto-Generated Server Package\n");
    report.push_str("===================================\n\n");
    report.push_str(format!("Instance: {}\n", instance.name).as_str());
    report.push_str(format!("Minecraft: {}\n", instance.game_version).as_str());
    report.push_str(format!("Modloader: {}\n", instance.modloader).as_str());
    if let Some(version) = normalize_optional(instance.modloader_version.as_str()) {
        report.push_str(format!("Modloader version: {version}\n").as_str());
    }
    if let Some(java_major) = required_java {
        report.push_str(format!("Recommended Java: {java_major}\n").as_str());
    } else {
        report.push_str("Recommended Java: unknown\n");
    }
    report.push('\n');
    report.push_str("Included top-level entries:\n");
    for entry in included_root_entries
        .iter()
        .filter_map(|(entry, enabled)| enabled.then_some(entry.as_str()))
    {
        report.push_str(format!("- {entry}\n").as_str());
    }
    report.push('\n');
    if unknowns.is_empty() {
        report.push_str("CurseForge unknowns:\n- none\n\n");
    } else {
        report.push_str("CurseForge unknowns (not found on Modrinth by hash):\n");
        for entry in unknowns {
            report.push_str(format!("- {entry}\n").as_str());
        }
        report.push('\n');
    }
    if let Some(error) = &client_only.lookup_error {
        report.push_str(format!(
            "Client-only mod check skipped (Modrinth unreachable: {error}). All mods were included.\n\n"
        ).as_str());
    } else if client_only.descriptions.is_empty() {
        report.push_str("Client-only mods left out:\n- none\n\n");
    } else {
        report.push_str(
            "Client-only mods left out (Modrinth marks them as not running on servers):\n",
        );
        for entry in &client_only.descriptions {
            report.push_str(format!("- {entry}\n").as_str());
        }
        report.push('\n');
    }
    report.push_str("How to start:\n");
    report.push_str("- Linux/macOS: ./start-server.sh\n");
    report.push_str("- Windows: start-server.bat\n");
    report.push_str(
        "- If startup fails, check unknowns above first. Some mods may still be required or incompatible on dedicated server.\n",
    );
    report
}

fn build_server_start_script_sh(required_java: Option<u8>) -> String {
    let java_note = required_java
        .map(|major| tr!("echo \"Recommended Java major: {major}\"", major = major).into_owned())
        .unwrap_or_else(|| tr!("echo \"Recommended Java major: unknown\"").into_owned());
    tr!("#!/usr/bin/env bash
set -euo pipefail
{java_note}

jar=\"$(find . -maxdepth 6 -type f \\( -iname '*server*.jar' -o -iname '*forge*.jar' -o -iname '*fabric*.jar' -o -iname '*quilt*.jar' -o -iname '*neoforge*.jar' \\) | head -n 1)\"
if [ -z \"$jar\" ]; then
  jar=\"$(find . -maxdepth 6 -type f -iname '*.jar' | head -n 1)\"
fi
jar=\"${{jar#./}}\"

if [ -z \"$jar\" ]; then
  echo \"No server jar found in this directory.\"
  echo \"Check VERTEX_SERVER_BUILD_REPORT.txt for guidance.\"
  exit 1
fi

echo \"Starting server using $jar\"
java -Xms2G -Xmx4G -jar \"$jar\" nogui
", java_note = java_note).into_owned()
}

fn build_server_start_script_bat(required_java: Option<u8>) -> String {
    let java_note = required_java
        .map(|major| tr!("echo Recommended Java major: {major}", major = major).into_owned())
        .unwrap_or_else(|| tr!("echo Recommended Java major: unknown").into_owned());
    tr!("@echo off
setlocal EnableDelayedExpansion
{java_note}

set \"jar=\"
for /r %%f in (*server*.jar) do (
  if not defined jar set \"jar=%%f\"
)
if not defined jar (
  for /r %%f in (*forge*.jar *fabric*.jar *quilt*.jar *neoforge*.jar) do (
    if not defined jar set \"jar=%%f\"
  )
)
if not defined jar (
  for /r %%f in (*.jar) do (
  if not defined jar set \"jar=%%f\"
  )
)

if not defined jar (
  echo No server jar found in this directory.
  echo Check VERTEX_SERVER_BUILD_REPORT.txt for guidance.
  exit /b 1
)

echo Starting server using !jar!
java -Xms2G -Xmx4G -jar \"!jar!\" nogui
", java_note = java_note).into_owned()
}

fn normalize_zip_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn modified_millis(path: &Path) -> Option<u64> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(duration.as_millis() as u64)
}

fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn format_time_ago(timestamp_ms: Option<u64>, now_ms: u64) -> String {
    let Some(timestamp_ms) = timestamp_ms else {
        return "unknown".to_owned();
    };
    let elapsed_ms = now_ms.saturating_sub(timestamp_ms);
    let elapsed_seconds = elapsed_ms / 1_000;
    if elapsed_seconds < 60 {
        return tr!("just now").into_owned();
    }
    let elapsed_minutes = elapsed_seconds / 60;
    if elapsed_minutes < 60 {
        return tr!("{elapsed_minutes}m ago", elapsed_minutes = elapsed_minutes).into_owned();
    }
    let elapsed_hours = elapsed_minutes / 60;
    if elapsed_hours < 24 {
        return tr!("{elapsed_hours}h ago", elapsed_hours = elapsed_hours).into_owned();
    }
    let elapsed_days = elapsed_hours / 24;
    if elapsed_days < 30 {
        return tr!("{elapsed_days}d ago", elapsed_days = elapsed_days).into_owned();
    }
    let elapsed_months = elapsed_days / 30;
    if elapsed_months < 12 {
        return tr!("{elapsed_months}mo ago", elapsed_months = elapsed_months).into_owned();
    }
    let elapsed_years = elapsed_days / 365;
    tr!("{elapsed_years}y ago", elapsed_years = elapsed_years).into_owned()
}

fn ensure_selected_modloader_is_supported(state: &mut InstanceScreenState, game_version: &str) {
    if !support_catalog_ready(state) {
        state.incompatible_modloader_version_warning_key = None;
        return;
    }
    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
        state.incompatible_modloader_version_warning_key = None;
        return;
    }

    let selected_label = MODLOADER_OPTIONS
        .get(state.selected_modloader)
        .copied()
        .unwrap_or(MODLOADER_OPTIONS[0]);
    let entered_modloader_version = state.modloader_version_input.trim();
    if entered_modloader_version.is_empty()
        || is_latest_modloader_version_alias(entered_modloader_version)
    {
        state.incompatible_modloader_version_warning_key = None;
        return;
    }
    let Some(known_versions) = state
        .loader_versions
        .versions_for_loader(selected_label, game_version)
    else {
        state.incompatible_modloader_version_warning_key = None;
        return;
    };
    if known_versions
        .iter()
        .any(|version| version.eq_ignore_ascii_case(entered_modloader_version))
    {
        state.incompatible_modloader_version_warning_key = None;
        return;
    }

    let warning_key = format!(
        "{}\n{}\n{}",
        selected_label, game_version, entered_modloader_version
    );
    if state.incompatible_modloader_version_warning_key.as_deref() == Some(warning_key.as_str()) {
        return;
    }
    state.incompatible_modloader_version_warning_key = Some(warning_key);

    tracing::warn!(
        target: "vertexlauncher/ui/instance",
        selected_modloader = %selected_label,
        game_version = %game_version,
        selected_modloader_version = %entered_modloader_version,
        "Selected modloader version is not currently marked compatible for this game version; keeping user selection."
    );
}

fn support_catalog_ready(state: &InstanceScreenState) -> bool {
    state.version_catalog_filter.is_some() && state.version_catalog_error.is_none()
}
