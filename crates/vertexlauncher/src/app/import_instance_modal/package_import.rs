use vertex_i18n::tr;
use super::*;

#[path = "package_import/download_support.rs"]
mod download_support;
#[path = "package_import/game_version.rs"]
mod game_version;
#[path = "package_import/instance_root_swap.rs"]
mod instance_root_swap;
#[path = "package_import/managed_source.rs"]
mod managed_source;
#[path = "package_import/mrpack_dependency_info.rs"]
mod mrpack_dependency_info;
#[path = "package_import/mrpack_file.rs"]
mod mrpack_file;
#[path = "package_import/mrpack_file_env.rs"]
mod mrpack_file_env;
#[path = "package_import/mrpack_import.rs"]
mod mrpack_import;
#[path = "package_import/mrpack_manifest.rs"]
mod mrpack_manifest;
#[path = "package_import_curseforge.rs"]
mod package_import_curseforge;
#[path = "package_import/resolved_modrinth_download_source.rs"]
mod resolved_modrinth_download_source;
#[path = "package_import/resolved_mrpack_source.rs"]
mod resolved_mrpack_source;
#[cfg(test)]
#[path = "package_import/tests.rs"]
mod tests;
#[path = "package_import/vtmpack_import.rs"]
mod vtmpack_import;

pub(super) use self::download_support::*;
pub(super) use self::game_version::*;
pub(super) use self::instance_root_swap::*;
use self::managed_source::ManagedSource;
use self::mrpack_dependency_info::MrpackDependencyInfo;
use self::mrpack_file::MrpackFile;
pub(super) use self::mrpack_import::*;
use self::mrpack_manifest::MrpackManifest;
use self::package_import_curseforge::find_curseforge_file;
pub use self::package_import_curseforge::{
    CurseForgeManualDownloadRequirement, attach_curseforge_modpack_install_state,
    format_curseforge_download_url_error, prepare_curseforge_manual_download_for_file,
    prepare_curseforge_manual_downloads,
};
use self::package_import_curseforge::{CurseForgePackManifest, CurseForgePackMinecraft};
#[cfg(test)]
use self::package_import_curseforge::{
    curseforge_file_has_api_download, modrinth_fallback_queries, select_modrinth_backup_file,
};
use self::resolved_modrinth_download_source::ResolvedModrinthDownloadSource;
use self::resolved_mrpack_source::ResolvedMrpackSource;
pub(super) use self::vtmpack_import::*;

pub(super) fn import_curseforge_pack(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<InstanceRecord, ImportPackageError> {
    package_import_curseforge::import_curseforge_pack(store, installations_root, request, progress)
}

pub(super) fn read_curseforge_pack_manifest(path: &Path) -> Result<CurseForgePackManifest, String> {
    package_import_curseforge::read_curseforge_pack_manifest(path)
}

pub(super) fn resolve_curseforge_pack_dependencies(
    minecraft: &CurseForgePackMinecraft,
) -> Result<MrpackDependencyInfo, String> {
    package_import_curseforge::resolve_curseforge_pack_dependencies(minecraft)
}

pub(super) fn import_progress(
    message: &str,
    completed_steps: usize,
    total_steps: usize,
) -> ImportProgress {
    ImportProgress {
        message: vertex_i18n::t(message).into_owned(),
        completed_steps,
        total_steps,
    }
}

pub(super) fn normalize_source_name(source: Option<&str>) -> Option<ManagedSource> {
    match source?.trim().to_ascii_lowercase().as_str() {
        "modrinth" => Some(ManagedSource::Modrinth),
        "curseforge" => Some(ManagedSource::CurseForge),
        _ => None,
    }
}

pub(super) fn join_safe(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut clean = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(tr!("unsafe path in import package: {relative}", relative = relative.display()).into_owned());
            }
        }
    }
    Ok(root.join(clean))
}

pub(super) fn default_if_blank(value: &str, fallback: String) -> String {
    instances::normalize_optional(value).unwrap_or(fallback)
}
