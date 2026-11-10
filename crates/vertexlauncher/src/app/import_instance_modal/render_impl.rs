use vertex_i18n::tr;
use super::*;

pub fn import_package_with_progress<F>(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: ImportRequest,
    mut progress: F,
) -> Result<InstanceRecord, ImportPackageError>
where
    F: FnMut(ImportProgress),
{
    match &request.source {
        ImportSource::ManifestFile(path) => {
            let preview = inspect_package(path.as_path()).map_err(ImportPackageError::message)?;
            match preview.kind {
                ImportPreviewKind::Manifest(ImportPackageKind::VertexPack) => {
                    import_vtmpack(store, installations_root, &request, &mut progress)
                        .map_err(ImportPackageError::message)
                }
                ImportPreviewKind::Manifest(ImportPackageKind::ModrinthPack) => {
                    import_mrpack(store, installations_root, &request, &mut progress)
                        .map_err(ImportPackageError::message)
                }
                ImportPreviewKind::Manifest(ImportPackageKind::CurseForgePack) => {
                    import_curseforge_pack(store, installations_root, &request, &mut progress)
                }
                ImportPreviewKind::Launcher(_) => Err(ImportPackageError::message(
                    vertex_i18n::src!("Launcher previews are not valid for manifest imports."),
                )),
            }
        }
        ImportSource::LauncherDirectory { .. } => {
            progress(import_progress(vertex_i18n::src!("Copying launcher instance files..."), 0, 0));
            import_launcher_instance(store, installations_root, &request)
                .map_err(ImportPackageError::message)
        }
    }
}

#[allow(dead_code)]
pub fn update_mrpack_instance_with_progress<F>(
    store: &mut InstanceStore,
    installations_root: &Path,
    instance_id: &str,
    package_path: &Path,
    mut progress: F,
) -> Result<InstanceRecord, String>
where
    F: FnMut(ImportProgress),
{
    let existing_instance = store
        .find(instance_id)
        .cloned()
        .ok_or_else(|| tr!("instance {instance_id} was not found", instance_id = instance_id).into_owned())?;
    let existing_root = instance_root_path(installations_root, &existing_instance);
    let current_modpack_state = load_modpack_install_state(existing_root.as_path())
        .ok_or_else(|| tr!("This instance is not tied to an updatable modpack yet.").into_owned())?;

    progress(import_progress(vertex_i18n::src!("Reading updated .mrpack manifest..."), 0, 1));
    let manifest = read_mrpack_manifest(package_path)?;
    let dependency_info = resolve_mrpack_dependencies(&manifest.dependencies)?;
    let override_steps = count_mrpack_override_entries(package_path)?;
    let total_steps = 6 + override_steps + manifest.files.len();
    let temp_root = unique_temp_instance_root(installations_root, existing_instance.id.as_str());

    fs_create_dir_all_logged(temp_root.as_path()).map_err(|err| {
        tr!("failed to create temp instance root {temp_root}: {err}", temp_root = temp_root.display(), err = err).into_owned()
    })?;
    fs_create_dir_all_logged(temp_root.join("mods").as_path())
        .map_err(|err| tr!("failed to create temp mods directory: {err}", err = err).into_owned())?;

    progress(import_progress(
        vertex_i18n::src!("Building updated modpack files..."),
        1,
        total_steps,
    ));
    if let Err(err) = populate_mrpack_instance(
        package_path,
        manifest.clone(),
        temp_root.as_path(),
        total_steps,
        &mut progress,
    ) {
        let _ = fs_remove_dir_all_logged(temp_root.as_path());
        return Err(err);
    }

    let new_base_manifest = build_mrpack_base_manifest(temp_root.as_path(), &manifest)?;
    let new_modpack_state = build_mrpack_install_state(package_path, &manifest, new_base_manifest);
    let current_manifest = load_content_manifest(existing_root.as_path());
    let mut final_manifest = new_modpack_state.base_manifest.clone();
    let pack_managed_paths =
        pack_managed_path_keys(&current_manifest, &current_modpack_state.base_manifest);

    progress(import_progress(
        vertex_i18n::src!("Preserving user-added content..."),
        total_steps.saturating_sub(4),
        total_steps,
    ));
    preserve_non_pack_managed_content(
        existing_root.as_path(),
        temp_root.as_path(),
        &pack_managed_paths,
    )?;
    for (project_key, project) in current_manifest.projects {
        if !project.pack_managed {
            final_manifest.projects.insert(project_key, project);
        }
    }

    progress(import_progress(
        vertex_i18n::src!("Preserving worlds and servers..."),
        total_steps.saturating_sub(3),
        total_steps,
    ));
    preserve_instance_user_state(existing_root.as_path(), temp_root.as_path())?;
    save_content_manifest(temp_root.as_path(), &final_manifest)?;
    save_modpack_install_state(temp_root.as_path(), &new_modpack_state)?;

    progress(import_progress(
        vertex_i18n::src!("Finalizing updated instance..."),
        total_steps.saturating_sub(2),
        total_steps,
    ));
    swap_instance_root(existing_root.as_path(), temp_root.as_path())?;

    let instance = store
        .find_mut(instance_id)
        .ok_or_else(|| tr!("instance {instance_id} disappeared during update", instance_id = instance_id).into_owned())?;
    instance.game_version = dependency_info.game_version;
    instance.modloader = dependency_info.modloader;
    instance.modloader_version = dependency_info.modloader_version;

    progress(import_progress(
        vertex_i18n::src!("Update complete."),
        total_steps,
        total_steps,
    ));
    Ok(instance.clone())
}

pub(crate) fn load_preview_from_state(state: &mut ImportInstanceState) {
    let tx = state.preview_results.sender();
    let request = match selected_import_mode(state) {
        ImportMode::ManifestFile => {
            let path = state.package_path.clone();
            if path.as_os_str().is_empty() {
                state.preview = None;
                state.error = Some(
                    tr!("Choose a .vtmpack, .mrpack, or CurseForge modpack .zip file first.").into_owned(),
                );
                return;
            }
            (path, selected_launcher_hint(state), true)
        }
        ImportMode::LauncherDirectory => {
            let path = state.launcher_path.clone();
            if path.as_os_str().is_empty() {
                state.preview = None;
                state.error = Some(tr!("Choose an instance folder first.").into_owned());
                return;
            }
            (path, selected_launcher_hint(state), false)
        }
    };

    state.preview_request_serial = state.preview_request_serial.saturating_add(1);
    let request_serial = state.preview_request_serial;
    state.preview_in_flight = true;
    state.preview_status_message = Some(tr!("Inspecting import source in the background...").into_owned());
    state.error = None;
    let progress_tx = Some(state.preview_progress.sender());
    let _ = tokio_runtime::spawn_detached(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            let (path, launcher_hint, manifest_mode) = request;
            if manifest_mode {
                inspect_package_with_progress(path.as_path(), |message| {
                    if let Some(tx) = progress_tx.as_ref() {
                        let _ = tx.send((request_serial, message));
                    }
                })
            } else {
                inspect_launcher_instance(path.as_path(), launcher_hint)
            }
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);
        if let Err(err) = tx.send((request_serial, result)) {
            tracing::error!(
                target: "vertexlauncher/import_instance",
                request_serial,
                error = %err,
                "Failed to deliver import preview result."
            );
        }
    });
}

/// File-dialog slots for the import modal's pickers.
const IMPORT_FILE_DIALOG: &str = "import_instance_pick_file";
const IMPORT_FOLDER_DIALOG: &str = "import_instance_pick_folder";
