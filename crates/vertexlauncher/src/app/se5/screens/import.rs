//! The import dialog (a pack manifest or another launcher's instance folder) and the CurseForge manual-download
//! dialog that either flow can raise.

use std::collections::HashMap;

use crate::app::se5::file_dialog::{Dialog as PickerDialog, Pick};
use vertex_shell::sturdy::ui::{Sizing, TextStyle, TextWrap, Ui};
use vertex_shell::sturdy::ui_kit::ModalStyle;

use super::super::super::import_flow::ManualDownloadContinuation;
use super::super::super::import_instance_modal::{
    ImportMode, ImportPackageKind, ImportPreviewKind, ImportRequest, ImportSource,
    LAUNCHER_KIND_OPTIONS, load_preview_from_state, loader_label_for_display, path_input_string,
    selected_import_mode, selected_launcher_hint, update_path_from_input,
};
use super::super::AppState;
use super::super::kit::{self, ButtonKind, Kit, TextRole};

const FILE_SLOT: &str = "import::file";
const FOLDER_SLOT: &str = "import::folder";

/// Returns false once the dialog should close.
pub fn dialog(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) -> bool {
    let curseforge_key_configured = !state.config.curseforge_api_key().trim().is_empty();
    // Pickers report here; a finished one fills in the path and inspects it.
    if let Some(path) = state
        .file_dialogs
        .take(FILE_SLOT)
        .and_then(|paths| paths.into_iter().next())
    {
        state.flows.import.package_path = path;
        load_preview_from_state(&mut state.flows.import);
    }
    if let Some(path) = state
        .file_dialogs
        .take(FOLDER_SLOT)
        .and_then(|paths| paths.into_iter().next())
    {
        state.flows.import.launcher_path = path;
        load_preview_from_state(&mut state.flows.import);
    }

    let style = ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(640.0),
        ..ModalStyle::default()
    };
    let in_flight = state.flows.import.import_in_flight;
    let mut cancel = false;
    let mut submit: Option<ImportRequest> = None;
    let mut pick_file = false;
    let mut pick_folder = false;
    let import = &mut state.flows.import;
    let response = kit::modal(ui, "import-instance", &style, !in_flight, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text(ui, TextRole::Heading, "Import Profile");
        kit.text(
            ui,
            TextRole::Muted,
            "Import from a pack manifest or copy an instance out of another launcher.",
        );

        let modes = ImportMode::options().map(str::to_owned).to_vec();
        let previous_mode = import.source_mode_index;
        kit.setting_row(ui, "import::mode-row", "Import source", Some("Choose whether to import from a pack manifest or an existing launcher instance folder."), |kit, ui| {
            if let Some(index) = kit.dropdown(ui, "import::mode", import.source_mode_index, &modes, 280.0) {
                import.source_mode_index = index;
            }
        });
        if import.source_mode_index != previous_mode {
            import.preview = None;
            import.error = None;
        }

        match selected_import_mode(import) {
            ImportMode::ManifestFile => {
                let mut text = path_input_string(import.package_path.as_path());
                kit.setting_row(
                    ui,
                    "import::file-row",
                    "Manifest file",
                    Some("Select a .vtmpack, .mrpack, or CurseForge modpack .zip file."),
                    |kit, ui| {
                        kit.text_field(
                            ui,
                            "import::file-path",
                            &mut text,
                            "Path to a .vtmpack, .mrpack or .zip",
                            false,
                            Sizing::Fixed(300.0),
                        );
                    },
                );
                if update_path_from_input(&mut import.package_path, &text) {
                    import.preview = None;
                    import.error = None;
                }
                let _row = kit.row(ui, kit::SPACE_SM);
                pick_file = kit.button(
                    ui,
                    "import::choose-file",
                    "Choose manifest",
                    ButtonKind::Neutral,
                    true,
                );
                if kit.button(
                    ui,
                    "import::inspect-file",
                    "Inspect manifest",
                    ButtonKind::Neutral,
                    true,
                ) {
                    load_preview_from_state(import);
                }
                drop(_row);
                let is_zip = import
                    .package_path
                    .to_string_lossy()
                    .trim()
                    .to_ascii_lowercase()
                    .ends_with(".zip");
                let highlight = !curseforge_key_configured
                    && (matches!(
                        import.preview.as_ref().map(|p| p.kind),
                        Some(ImportPreviewKind::Manifest(
                            ImportPackageKind::CurseForgePack
                        ))
                    ) || is_zip);
                let notice = "CurseForge modpack zips are supported, but they only work if you have a CurseForge API key in Settings. Vertex will fall back to Modrinth downloads when it can resolve an exact compatible match.";
                let role = if highlight {
                    TextRole::Danger
                } else {
                    TextRole::Muted
                };
                kit::label(
                    ui,
                    notice,
                    &TextStyle {
                        wrap: TextWrap::Words,
                        ..kit.style(role)
                    },
                );
            }
            ImportMode::LauncherDirectory => {
                let launchers: Vec<String> = LAUNCHER_KIND_OPTIONS
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect();
                let previous_kind = import.launcher_kind_index;
                kit.setting_row(
                    ui,
                    "import::launcher-row",
                    "Launcher",
                    Some("Use Auto-detect unless you know which launcher produced the instance."),
                    |kit, ui| {
                        if let Some(index) = kit.dropdown(
                            ui,
                            "import::launcher",
                            import.launcher_kind_index,
                            &launchers,
                            280.0,
                        ) {
                            import.launcher_kind_index = index;
                        }
                    },
                );
                let mut text = path_input_string(import.launcher_path.as_path());
                kit.setting_row(ui, "import::folder-row", "Instance folder", Some("Choose the instance directory from Modrinth, CurseForge, Prism, ATLauncher, or another launcher."), |kit, ui| {
                    kit.text_field(ui, "import::folder-path", &mut text, "Path to the instance folder", false, Sizing::Fixed(300.0));
                });
                if update_path_from_input(&mut import.launcher_path, &text)
                    || import.launcher_kind_index != previous_kind
                {
                    import.preview = None;
                    import.error = None;
                }
                let _row = kit.row(ui, kit::SPACE_SM);
                pick_folder = kit.button(
                    ui,
                    "import::choose-folder",
                    "Choose folder",
                    ButtonKind::Neutral,
                    true,
                );
                if kit.button(
                    ui,
                    "import::inspect-folder",
                    "Inspect folder",
                    ButtonKind::Neutral,
                    true,
                ) {
                    load_preview_from_state(import);
                }
            }
        }

        kit.setting_row(
            ui,
            "import::name-row",
            "Imported profile name",
            Some("Defaults to the package name, but you can override it."),
            |kit, ui| {
                kit.text_field(
                    ui,
                    "import::name",
                    &mut import.instance_name,
                    "Profile name",
                    false,
                    Sizing::Fixed(300.0),
                );
            },
        );

        if let Some(preview) = import.preview.as_ref() {
            let _card = kit.card(ui, "import::preview", kit::SPACE_XS);
            kit.text(ui, TextRole::Heading, "Detected package");
            kit.text(ui, TextRole::Body, preview.kind.label());
            let versions = format!(
                "Minecraft {} • {}",
                preview.game_version,
                loader_label_for_display(
                    preview.modloader.as_str(),
                    preview.modloader_version.as_str()
                )
            );
            kit.text(ui, TextRole::Body, &versions);
            kit::label(
                ui,
                &preview.summary,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Muted)
                },
            );
        }
        if let Some(error) = import.error.as_deref() {
            kit::label(
                ui,
                error,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Danger)
                },
            );
        }
        if import.preview_in_flight {
            let message = import
                .preview_status_message
                .as_deref()
                .unwrap_or("Inspecting import source in the background...");
            kit.text(ui, TextRole::Muted, message);
        }
        if import.import_in_flight {
            let progress = import.import_latest_progress.as_ref();
            let fraction = progress.and_then(|p| {
                (p.total_steps > 0)
                    .then(|| (p.completed_steps as f32 / p.total_steps as f32).clamp(0.0, 1.0))
            });
            kit.text(
                ui,
                TextRole::Body,
                progress.map_or("Importing profile in the background...", |p| {
                    p.message.as_str()
                }),
            );
            let counts = progress
                .map(|p| {
                    format!(
                        "{} of {} steps complete",
                        p.completed_steps.min(p.total_steps),
                        p.total_steps
                    )
                })
                .unwrap_or_else(|| "Preparing import task...".to_owned());
            kit.text(ui, TextRole::Small, &counts);
            kit.progress(ui, "import::progress", fraction, kit::uptime());
        }

        let _actions = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        if kit.button(
            ui,
            "import::cancel",
            "Cancel",
            ButtonKind::Neutral,
            !import.import_in_flight,
        ) {
            cancel = true;
        }
        let missing_path = match selected_import_mode(import) {
            ImportMode::ManifestFile => import.package_path.as_os_str().is_empty(),
            ImportMode::LauncherDirectory => import.launcher_path.as_os_str().is_empty(),
        };
        if kit.button(
            ui,
            "import::confirm",
            "Import profile",
            ButtonKind::Primary,
            !import.import_in_flight && !missing_path,
        ) {
            if import.preview.is_none() {
                load_preview_from_state(import);
            }
            if let Some(preview) = import.preview.as_ref() {
                let instance_name = instances::normalize_optional(import.instance_name.as_str())
                    .unwrap_or_else(|| preview.detected_name.clone());
                submit = Some(ImportRequest {
                    source: match selected_import_mode(import) {
                        ImportMode::ManifestFile => {
                            ImportSource::ManifestFile(import.package_path.clone())
                        }
                        ImportMode::LauncherDirectory => ImportSource::LauncherDirectory {
                            path: import.launcher_path.clone(),
                            launcher: selected_launcher_hint(import),
                        },
                    },
                    instance_name,
                    manual_curseforge_files: HashMap::new(),
                    manual_curseforge_staging_dir: None,
                    max_concurrent_downloads: 4,
                });
            }
        }
    });

    if pick_file {
        state.file_dialogs.open(
            FILE_SLOT,
            Pick::File,
            PickerDialog::new()
                .filter("Launcher profiles", &["vtmpack", "mrpack", "zip"])
                .filter("Vertex packs", &["vtmpack"])
                .filter("Modrinth packs", &["mrpack"])
                .filter("CurseForge packs", &["zip"]),
        );
    }
    if pick_folder {
        state
            .file_dialogs
            .open(FOLDER_SLOT, Pick::Folder, PickerDialog::new());
    }
    if let Some(request) = submit {
        state.start_import(request);
    }
    if (response.dismissed || kit.back_pressed()) || cancel {
        state.flows.import.reset();
        kit.forget("import::");
        return false;
    }
    true
}

/// Waiting for the user to download CurseForge files by hand. Shown whenever a manual download is pending, over
/// whatever else is open.
pub fn manual_download_dialog(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let Some(pending) = state.flows.pending_manual.as_ref() else {
        return;
    };
    let cancel_label = match pending.continuation {
        ManualDownloadContinuation::Import(_) => "Cancel Import",
        ManualDownloadContinuation::DiscoverInstall(_) => "Cancel Install",
    };
    let requirements = pending.pending_files.clone();
    let downloads_dir = pending.downloads_dir.clone();
    let staged = pending.staged_files.len();
    let error = pending.error.clone();
    let style = ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(620.0),
        z_index: 2100,
        ..ModalStyle::default()
    };
    let mut cancel = false;
    let mut open_folder = false;
    let response = kit::modal(ui, "curseforge-manual", &style, true, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text(ui, TextRole::Heading, "Manual CurseForge downloads needed");
        kit::label(
            ui,
            "CurseForge does not allow some files to be downloaded by other launchers. Open each file's page, download it in your browser, and Vertex will pick it up from your downloads folder automatically.",
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Body)
            },
        );
        kit::label(
            ui,
            &format!("Watching: {}", downloads_dir.display()),
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
        if staged > 0 {
            kit.text(
                ui,
                TextRole::Muted,
                &format!("{staged} file(s) already found."),
            );
        }
        {
            let _list = ui.element(
                &vertex_shell::sturdy::ui::ElementDesc::new()
                    .id("curseforge-manual::list".into())
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(240.0))
                    .direction(vertex_shell::sturdy::ui::Direction::TopToBottom)
                    .child_gap(kit::SPACE_SM)
                    .clip(vertex_shell::sturdy::ui::Clip::vertical()),
            );
            for requirement in &requirements {
                let _card = kit.card(
                    ui,
                    &format!("curseforge-manual::file::{}", requirement.file_id),
                    kit::SPACE_XS,
                );
                kit.text(ui, TextRole::Body, &requirement.project_name);
                kit.text(
                    ui,
                    TextRole::Muted,
                    &format!("Expected file: {}", requirement.file_name),
                );
                kit.text(
                    ui,
                    TextRole::Small,
                    &format!(
                        "CurseForge project {}, file {}",
                        requirement.project_id, requirement.file_id
                    ),
                );
                if kit.button(
                    ui,
                    &format!("curseforge-manual::page::{}", requirement.file_id),
                    "Open CurseForge file page",
                    ButtonKind::Neutral,
                    true,
                ) && let Err(err) =
                    launcher_ui::desktop::open_url(requirement.download_page_url.as_str())
                {
                    launcher_ui::notification::error!(
                        "curseforge",
                        "Could not open the browser: {}",
                        err
                    );
                }
            }
        }
        if let Some(error) = &error {
            kit::label(
                ui,
                error,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Danger)
                },
            );
        }
        let _row = kit.row(ui, kit::SPACE_SM);
        if kit.button(
            ui,
            "curseforge-manual::folder",
            "Open downloads folder",
            ButtonKind::Neutral,
            true,
        ) {
            open_folder = true;
        }
        kit.fill(ui);
        if kit.button(
            ui,
            "curseforge-manual::cancel",
            cancel_label,
            ButtonKind::Neutral,
            true,
        ) {
            cancel = true;
        }
    });
    if open_folder && let Err(err) = launcher_ui::desktop::open_in_file_manager(&downloads_dir) {
        launcher_ui::notification::error!(
            "curseforge",
            "Could not open the downloads folder: {}",
            err
        );
    }
    if (response.dismissed || kit.back_pressed()) || cancel {
        state.cancel_manual_downloads();
        kit.forget("curseforge-manual::");
    }
}
