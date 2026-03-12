use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use curseforge::Client as CurseForgeClient;
use eframe::egui;
use instances::{
    InstanceRecord, InstanceStore, NewInstanceSpec, create_instance, delete_instance,
    instance_root_path,
};
use launcher_ui::{
    ui::style,
    ui::{components::settings_widgets, modal},
};
use modrinth::Client as ModrinthClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use textui::{ButtonOptions, LabelOptions, TextUi};

const MODAL_GAP_SM: f32 = 6.0;
const MODAL_GAP_MD: f32 = 8.0;
const MODAL_GAP_LG: f32 = 10.0;
const ACTION_BUTTON_MAX_WIDTH: f32 = 260.0;
const MANAGED_CONTENT_MANIFEST_FILE_NAME: &str = ".vertex-content-manifest.toml";

#[derive(Debug, Default)]
pub struct ImportInstanceState {
    pub source_mode_index: usize,
    pub package_path: String,
    pub launcher_path: String,
    pub launcher_kind_index: usize,
    pub instance_name: String,
    pub error: Option<String>,
    preview: Option<ImportPreview>,
}

impl ImportInstanceState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Debug)]
pub struct ImportRequest {
    pub source: ImportSource,
    pub instance_name: String,
}

#[derive(Clone, Debug)]
pub enum ImportSource {
    ManifestFile(PathBuf),
    LauncherDirectory {
        path: PathBuf,
        launcher: Option<LauncherKind>,
    },
}

#[derive(Clone, Debug)]
pub enum ModalAction {
    None,
    Cancel,
    Import(ImportRequest),
}

#[derive(Clone, Debug)]
struct ImportPreview {
    kind: ImportPreviewKind,
    detected_name: String,
    game_version: String,
    modloader: String,
    modloader_version: String,
    summary: String,
}

#[derive(Clone, Copy, Debug)]
enum ImportPreviewKind {
    Manifest(ImportPackageKind),
    Launcher(LauncherKind),
}

impl ImportPreviewKind {
    fn label(self) -> &'static str {
        match self {
            ImportPreviewKind::Manifest(kind) => kind.label(),
            ImportPreviewKind::Launcher(kind) => kind.label(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportPackageKind {
    VertexPack,
    ModrinthPack,
}

impl ImportPackageKind {
    fn label(self) -> &'static str {
        match self {
            ImportPackageKind::VertexPack => "Vertex .vtmpack",
            ImportPackageKind::ModrinthPack => "Modrinth .mrpack",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportMode {
    ManifestFile,
    LauncherDirectory,
}

impl ImportMode {
    fn from_index(index: usize) -> Self {
        match index {
            1 => Self::LauncherDirectory,
            _ => Self::ManifestFile,
        }
    }

    fn options() -> [&'static str; 2] {
        ["From manifest file", "Import from another launcher"]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LauncherKind {
    Modrinth,
    CurseForge,
    Prism,
    ATLauncher,
    Unknown,
}

impl LauncherKind {
    fn label(self) -> &'static str {
        match self {
            Self::Modrinth => "Modrinth launcher instance",
            Self::CurseForge => "CurseForge instance",
            Self::Prism => "Prism / MultiMC / PolyMC instance",
            Self::ATLauncher => "ATLauncher instance",
            Self::Unknown => "Generic launcher instance",
        }
    }
}

const LAUNCHER_KIND_OPTIONS: [&str; 5] = [
    "Auto-detect",
    "Modrinth",
    "CurseForge",
    "Prism / MultiMC",
    "ATLauncher",
];

fn selected_import_mode(state: &ImportInstanceState) -> ImportMode {
    ImportMode::from_index(state.source_mode_index)
}

fn selected_launcher_hint(state: &ImportInstanceState) -> Option<LauncherKind> {
    match state.launcher_kind_index {
        1 => Some(LauncherKind::Modrinth),
        2 => Some(LauncherKind::CurseForge),
        3 => Some(LauncherKind::Prism),
        4 => Some(LauncherKind::ATLauncher),
        _ => None,
    }
}

pub fn render(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut ImportInstanceState,
) -> ModalAction {
    let mut action = ModalAction::None;
    let viewport_rect = ctx.input(|i| i.content_rect());
    let modal_max_width = (viewport_rect.width() * 0.85).max(1.0);
    let modal_max_height = (viewport_rect.height() * 0.82).max(1.0);
    let modal_pos = egui::pos2(
        (viewport_rect.center().x - modal_max_width * 0.5).clamp(
            viewport_rect.left(),
            viewport_rect.right() - modal_max_width,
        ),
        (viewport_rect.center().y - modal_max_height * 0.5).clamp(
            viewport_rect.top(),
            viewport_rect.bottom() - modal_max_height,
        ),
    );

    modal::show_scrim(ctx, "import_instance_modal_scrim", viewport_rect);
    egui::Window::new("Import Profile")
        .id(egui::Id::new("import_instance_modal_window"))
        .order(egui::Order::Foreground)
        .fixed_pos(modal_pos)
        .fixed_size(egui::vec2(modal_max_width, modal_max_height))
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .title_bar(false)
        .hscroll(false)
        .vscroll(true)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(modal::window_frame(ctx))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(MODAL_GAP_MD, MODAL_GAP_MD);
            let text_color = ui.visuals().text_color();
            let heading_style = LabelOptions {
                font_size: 34.0,
                line_height: 38.0,
                weight: 700,
                color: text_color,
                wrap: false,
                ..LabelOptions::default()
            };
            let body_style = LabelOptions {
                font_size: 18.0,
                line_height: 24.0,
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            };

            let _ = text_ui.label(
                ui,
                "instance_import_heading",
                "Import Profile",
                &heading_style,
            );
            let _ = text_ui.label(
                ui,
                "instance_import_subheading",
                "Import from a pack manifest or copy an instance out of another launcher.",
                &body_style,
            );

            let previous_mode = state.source_mode_index;
            let _ = settings_widgets::full_width_dropdown_row(
                text_ui,
                ui,
                "instance_import_mode",
                "Import source",
                Some("Choose whether to import from a pack manifest or an existing launcher instance folder."),
                &mut state.source_mode_index,
                &ImportMode::options(),
            );
            if state.source_mode_index != previous_mode {
                state.preview = None;
                state.error = None;
            }
            ui.add_space(MODAL_GAP_SM);

            match selected_import_mode(state) {
                ImportMode::ManifestFile => {
                    let previous_path = state.package_path.clone();
                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        "instance_import_package_path",
                        "Manifest file",
                        Some("Select a .vtmpack or .mrpack file."),
                        &mut state.package_path,
                    );
                    if state.package_path != previous_path {
                        state.preview = None;
                        state.error = None;
                    }

                    ui.horizontal(|ui| {
                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_choose_file",
                            "Choose manifest",
                            (ui.available_width() * 0.5).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            if let Some(path) = pick_import_file() {
                                state.package_path = path.display().to_string();
                                load_preview_from_state(state);
                            }
                        }

                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_inspect_file",
                            "Inspect manifest",
                            (ui.available_width()).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            load_preview_from_state(state);
                        }
                    });
                }
                ImportMode::LauncherDirectory => {
                    let previous_path = state.launcher_path.clone();
                    let previous_launcher_kind = state.launcher_kind_index;
                    let _ = settings_widgets::full_width_dropdown_row(
                        text_ui,
                        ui,
                        "instance_import_launcher_kind",
                        "Launcher",
                        Some("Use Auto-detect unless you know which launcher produced the instance."),
                        &mut state.launcher_kind_index,
                        &LAUNCHER_KIND_OPTIONS,
                    );
                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        "instance_import_launcher_path",
                        "Instance folder",
                        Some("Choose the instance directory from Modrinth, CurseForge, Prism, ATLauncher, or another launcher."),
                        &mut state.launcher_path,
                    );
                    if state.launcher_path != previous_path
                        || state.launcher_kind_index != previous_launcher_kind
                    {
                        state.preview = None;
                        state.error = None;
                    }

                    ui.horizontal(|ui| {
                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_choose_folder",
                            "Choose folder",
                            (ui.available_width() * 0.5).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            if let Some(path) = pick_import_directory() {
                                state.launcher_path = path.display().to_string();
                                load_preview_from_state(state);
                            }
                        }

                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_inspect_launcher",
                            "Inspect folder",
                            (ui.available_width()).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            load_preview_from_state(state);
                        }
                    });
                }
            }

            ui.add_space(MODAL_GAP_SM);
            let _ = settings_widgets::full_width_text_input_row(
                text_ui,
                ui,
                "instance_import_name",
                "Imported profile name",
                Some("Defaults to the package name, but you can override it."),
                &mut state.instance_name,
            );

            if let Some(preview) = state.preview.as_ref() {
                ui.add_space(MODAL_GAP_SM);
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_title",
                        "Detected package",
                        &LabelOptions {
                            font_size: 20.0,
                            line_height: 24.0,
                            weight: 600,
                            color: ui.visuals().text_color(),
                            wrap: false,
                            ..LabelOptions::default()
                        },
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_kind",
                        preview.kind.label(),
                        &body_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_versions",
                        format!(
                            "Minecraft {} • {}",
                            preview.game_version,
                            format_loader_label(
                                preview.modloader.as_str(),
                                preview.modloader_version.as_str()
                            )
                        )
                        .as_str(),
                        &body_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_summary",
                        preview.summary.as_str(),
                        &body_style,
                    );
                });
            }

            if let Some(error) = state.error.as_deref() {
                let _ = text_ui.label(
                    ui,
                    "instance_import_error",
                    error,
                    &LabelOptions {
                        color: ui.visuals().error_fg_color,
                        wrap: true,
                        ..LabelOptions::default()
                    },
                );
            }

            ui.add_space(MODAL_GAP_LG);
            ui.horizontal(|ui| {
                let button_style = ButtonOptions {
                    min_size: egui::vec2(160.0, style::CONTROL_HEIGHT),
                    text_color: ui.visuals().text_color(),
                    fill: ui.visuals().widgets.inactive.bg_fill,
                    fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                    fill_active: ui.visuals().widgets.active.bg_fill,
                    fill_selected: ui.visuals().selection.bg_fill,
                    stroke: ui.visuals().widgets.inactive.bg_stroke,
                    ..ButtonOptions::default()
                };
                if text_ui
                    .button(ui, "instance_import_cancel", "Cancel", &button_style)
                    .clicked()
                {
                    action = ModalAction::Cancel;
                }

                let import_disabled = match selected_import_mode(state) {
                    ImportMode::ManifestFile => state.package_path.trim().is_empty(),
                    ImportMode::LauncherDirectory => state.launcher_path.trim().is_empty(),
                };
                if ui
                    .add_enabled_ui(!import_disabled, |ui| {
                        text_ui.button(
                            ui,
                            "instance_import_confirm",
                            "Import profile",
                            &button_style,
                        )
                    })
                    .inner
                    .clicked()
                {
                    if state.preview.is_none() {
                        load_preview_from_state(state);
                    }
                    if let Some(preview) = state.preview.as_ref() {
                        let instance_name = non_empty(state.instance_name.as_str())
                            .unwrap_or_else(|| preview.detected_name.clone());
                        action = ModalAction::Import(ImportRequest {
                            source: match selected_import_mode(state) {
                                ImportMode::ManifestFile => {
                                    ImportSource::ManifestFile(PathBuf::from(
                                        state.package_path.trim(),
                                    ))
                                }
                                ImportMode::LauncherDirectory => {
                                    ImportSource::LauncherDirectory {
                                        path: PathBuf::from(state.launcher_path.trim()),
                                        launcher: selected_launcher_hint(state),
                                    }
                                }
                            },
                            instance_name,
                        });
                    }
                }
            });
        });

    action
}

pub fn import_package(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: ImportRequest,
) -> Result<InstanceRecord, String> {
    match &request.source {
        ImportSource::ManifestFile(path) => {
            let preview = inspect_package(path.as_path())?;
            match preview.kind {
                ImportPreviewKind::Manifest(ImportPackageKind::VertexPack) => {
                    import_vtmpack(store, installations_root, &request)
                }
                ImportPreviewKind::Manifest(ImportPackageKind::ModrinthPack) => {
                    import_mrpack(store, installations_root, &request)
                }
                ImportPreviewKind::Launcher(_) => {
                    Err("Launcher previews are not valid for manifest imports.".to_owned())
                }
            }
        }
        ImportSource::LauncherDirectory { .. } => {
            import_launcher_instance(store, installations_root, &request)
        }
    }
}

fn load_preview_from_state(state: &mut ImportInstanceState) {
    let preview_result = match selected_import_mode(state) {
        ImportMode::ManifestFile => {
            let path = PathBuf::from(state.package_path.trim());
            if path.as_os_str().is_empty() {
                state.preview = None;
                state.error = Some("Choose a .vtmpack or .mrpack file first.".to_owned());
                return;
            }
            inspect_package(path.as_path())
        }
        ImportMode::LauncherDirectory => {
            let path = PathBuf::from(state.launcher_path.trim());
            if path.as_os_str().is_empty() {
                state.preview = None;
                state.error = Some("Choose an instance folder first.".to_owned());
                return;
            }
            inspect_launcher_instance(path.as_path(), selected_launcher_hint(state))
        }
    };

    match preview_result {
        Ok(preview) => {
            if state.instance_name.trim().is_empty() {
                state.instance_name = preview.detected_name.clone();
            }
            state.preview = Some(preview);
            state.error = None;
        }
        Err(err) => {
            state.preview = None;
            state.error = Some(err);
        }
    }
}

fn pick_import_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Launcher profiles", &["vtmpack", "mrpack"])
        .add_filter("Vertex packs", &["vtmpack"])
        .add_filter("Modrinth packs", &["mrpack"])
        .pick_file()
}

fn pick_import_directory() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

fn inspect_package(path: &Path) -> Result<ImportPreview, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "vtmpack" => inspect_vtmpack(path),
        "mrpack" => inspect_mrpack(path),
        _ => Err(format!(
            "Unsupported import file {}. Expected .vtmpack or .mrpack.",
            path.display()
        )),
    }
}

fn inspect_vtmpack(path: &Path) -> Result<ImportPreview, String> {
    let manifest = read_vtmpack_manifest(path)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Manifest(ImportPackageKind::VertexPack),
        detected_name: manifest.instance.name.clone(),
        game_version: manifest.instance.game_version.clone(),
        modloader: manifest.instance.modloader.clone(),
        modloader_version: manifest.instance.modloader_version.clone(),
        summary: format!(
            "{} for Minecraft {} ({}) with {} downloadable items, {} bundled mods, {} config files.",
            manifest.instance.name,
            manifest.instance.game_version,
            format_loader_label(
                manifest.instance.modloader.as_str(),
                manifest.instance.modloader_version.as_str()
            ),
            manifest.downloadable_content.len(),
            manifest.bundled_mods.len(),
            manifest.configs.len()
        ),
    })
}

fn inspect_mrpack(path: &Path) -> Result<ImportPreview, String> {
    let manifest = read_mrpack_manifest(path)?;
    let dependency_info = resolve_mrpack_dependencies(&manifest.dependencies)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Manifest(ImportPackageKind::ModrinthPack),
        detected_name: non_empty(manifest.name.as_str())
            .unwrap_or_else(|| "Imported Modrinth Pack".to_owned()),
        game_version: dependency_info.game_version.clone(),
        modloader: dependency_info.modloader.clone(),
        modloader_version: dependency_info.modloader_version.clone(),
        summary: format!(
            "{} {} for Minecraft {} ({}) with {} packaged files.",
            non_empty(manifest.name.as_str()).unwrap_or_else(|| "Modrinth pack".to_owned()),
            non_empty(manifest.version_id.as_str()).unwrap_or_default(),
            dependency_info.game_version,
            format_loader_label(
                dependency_info.modloader.as_str(),
                dependency_info.modloader_version.as_str()
            ),
            manifest.files.len()
        )
        .trim()
        .to_owned(),
    })
}

#[derive(Clone, Debug)]
struct LauncherInspection {
    launcher: LauncherKind,
    name: String,
    description: Option<String>,
    game_version: String,
    modloader: String,
    modloader_version: String,
    summary: String,
    source_root: PathBuf,
    managed_manifest: ManagedContentManifest,
}

fn inspect_launcher_instance(
    path: &Path,
    launcher_hint: Option<LauncherKind>,
) -> Result<ImportPreview, String> {
    let inspection = inspect_launcher_details(path, launcher_hint)?;
    Ok(ImportPreview {
        kind: ImportPreviewKind::Launcher(inspection.launcher),
        detected_name: inspection.name,
        game_version: inspection.game_version,
        modloader: inspection.modloader,
        modloader_version: inspection.modloader_version,
        summary: inspection.summary,
    })
}

fn inspect_launcher_details(
    path: &Path,
    launcher_hint: Option<LauncherKind>,
) -> Result<LauncherInspection, String> {
    if !path.exists() {
        return Err(format!(
            "Instance folder {} does not exist.",
            path.display()
        ));
    }
    if !path.is_dir() {
        return Err(format!(
            "Import source {} is not a directory.",
            path.display()
        ));
    }

    let launcher = launcher_hint.unwrap_or_else(|| detect_launcher_kind(path));
    match launcher {
        LauncherKind::Modrinth => inspect_modrinth_launcher_instance(path),
        LauncherKind::CurseForge => inspect_curseforge_launcher_instance(path),
        LauncherKind::Prism => inspect_prism_launcher_instance(path),
        LauncherKind::ATLauncher => inspect_atlauncher_instance(path),
        LauncherKind::Unknown => inspect_generic_launcher_instance(path),
    }
}

fn import_launcher_instance(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
) -> Result<InstanceRecord, String> {
    let ImportSource::LauncherDirectory { path, launcher } = &request.source else {
        return Err("Launcher import requires an instance directory source.".to_owned());
    };

    let inspection = inspect_launcher_details(path.as_path(), *launcher)?;
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: inspection.description.clone(),
            thumbnail_path: None,
            modloader: default_if_blank(inspection.modloader.as_str(), "Vanilla".to_owned()),
            game_version: default_if_blank(inspection.game_version.as_str(), "latest".to_owned()),
            modloader_version: inspection.modloader_version.clone(),
        },
    )
    .map_err(|err| format!("failed to create imported profile: {err}"))?;
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) = copy_launcher_instance_content(
        inspection.source_root.as_path(),
        instance_root.as_path(),
        &inspection.managed_manifest,
    ) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    Ok(instance)
}

fn detect_launcher_kind(path: &Path) -> LauncherKind {
    if path.join(MANAGED_CONTENT_MANIFEST_FILE_NAME).is_file() {
        LauncherKind::Unknown
    } else if path.join("profile.json").is_file() || looks_like_modrinth_profile_path(path) {
        LauncherKind::Modrinth
    } else if path.join("minecraftinstance.json").is_file() {
        LauncherKind::CurseForge
    } else if path.join("instance.cfg").is_file() || path.join("mmc-pack.json").is_file() {
        LauncherKind::Prism
    } else if path.join("instance.json").is_file() {
        LauncherKind::ATLauncher
    } else {
        LauncherKind::Unknown
    }
}

fn inspect_modrinth_launcher_instance(path: &Path) -> Result<LauncherInspection, String> {
    if !path.join("profile.json").is_file() {
        let mut inspection =
            inspect_generic_launcher_instance_with_launcher(path, LauncherKind::Modrinth)?;
        let inferred = infer_modrinth_profile_metadata(path);
        if let Some(game_version) = inferred.game_version {
            inspection.game_version = game_version;
        }
        if let Some(modloader) = inferred.modloader {
            inspection.modloader = modloader;
        }
        if let Some(modloader_version) = inferred.modloader_version {
            inspection.modloader_version = modloader_version;
        }
        inspection.description = Some(
            "Imported from a Modrinth instance folder without profile.json metadata.".to_owned(),
        );
        inspection.summary = format!(
            "Detected {} by location. No profile.json was present, so Minecraft and loader metadata were inferred from profile contents where possible; files will still be copied from the instance root.",
            inspection.launcher.label()
        );
        return Ok(inspection);
    }

    let profile = read_json_file(path.join("profile.json").as_path())?;
    let source_root = path.to_path_buf();
    let name = first_non_empty([
        json_string_at_path(&profile, &["metadata", "name"]),
        json_string_at_path(&profile, &["name"]),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported Modrinth Instance".to_owned());
    let game_version = first_non_empty([
        json_string_at_path(&profile, &["metadata", "game_version"]),
        json_string_at_path(&profile, &["game_version"]),
        json_string_at_path(&profile, &["metadata", "minecraft_version"]),
        json_string_at_path(&profile, &["minecraft_version"]),
    ])
    .unwrap_or_else(|| "latest".to_owned());
    let (modloader, modloader_version) = infer_loader_pair(
        first_non_empty([
            json_string_at_path(&profile, &["metadata", "loader"]),
            json_string_at_path(&profile, &["loader"]),
            json_string_at_path(&profile, &["loader_type"]),
        ]),
        first_non_empty([
            json_string_at_path(&profile, &["metadata", "loader_version"]),
            json_string_at_path(&profile, &["loader_version"]),
            json_string_at_path(&profile, &["loaderVersion"]),
        ]),
    );
    let mut managed_manifest =
        load_existing_managed_manifest(path).unwrap_or_else(|_| ManagedContentManifest::default());
    if managed_manifest.projects.is_empty() {
        managed_manifest = extract_managed_manifest_from_json(
            &profile,
            source_root.as_path(),
            ManagedContentSourceHint::Modrinth,
        );
    }
    Ok(build_launcher_inspection(
        LauncherKind::Modrinth,
        name,
        Some("Imported from an existing Modrinth launcher instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

fn looks_like_modrinth_profile_path(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(parent_name) = parent.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    if parent_name != "profiles" {
        return false;
    }
    path.ancestors().any(|ancestor| {
        ancestor
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name == "ModrinthApp")
    })
}

#[derive(Default)]
struct ModrinthProfileMetadata {
    game_version: Option<String>,
    modloader: Option<String>,
    modloader_version: Option<String>,
}

fn infer_modrinth_profile_metadata(path: &Path) -> ModrinthProfileMetadata {
    let mut metadata = ModrinthProfileMetadata::default();
    metadata.game_version = infer_modrinth_game_version_from_telemetry(path)
        .or_else(|| infer_modrinth_game_version_from_filenames(path));

    let (modloader, modloader_version) = infer_modrinth_loader_from_profile(path);
    metadata.modloader = modloader;
    metadata.modloader_version = modloader_version;

    if let Some(app_root) = modrinth_app_root(path) {
        refine_modrinth_metadata_from_meta_cache(app_root.as_path(), &mut metadata);
    }

    metadata
}

fn infer_modrinth_game_version_from_telemetry(path: &Path) -> Option<String> {
    let telemetry_dir = path.join("logs").join("telemetry");
    let mut files = fs::read_dir(telemetry_dir)
        .ok()?
        .flatten()
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| entry.file_name());
    files.reverse();

    for entry in files {
        let raw = fs::read_to_string(entry.path()).ok()?;
        for line in raw.lines().rev() {
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && let Some(game_version) = value.get("game_version").and_then(Value::as_str)
            {
                let trimmed = game_version.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_owned());
                }
            }
        }
    }

    None
}

fn infer_modrinth_loader_from_profile(path: &Path) -> (Option<String>, Option<String>) {
    if let Some((loader, version)) = infer_modrinth_loader_from_dependencies_file(
        path.join("config/fabric_loader_dependencies.json")
            .as_path(),
    ) {
        return (Some(loader), Some(version));
    }

    if let Some((loader, version)) = infer_modrinth_loader_from_mod_filenames(path) {
        return (Some(loader), version);
    }

    (None, None)
}

fn infer_modrinth_loader_from_dependencies_file(path: &Path) -> Option<(String, String)> {
    let value = read_json_file_optional(path).ok()??;
    let fabric_requirement = value
        .get("overrides")
        .and_then(|value| value.get("fabricloader"))
        .and_then(|value| value.get("+depends"))
        .and_then(|value| value.get("fabricloader"))
        .and_then(Value::as_str)
        .and_then(clean_version_requirement)?;
    Some(("Fabric".to_owned(), fabric_requirement))
}

fn clean_version_requirement(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut out = String::new();
    let mut started = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            started = true;
            out.push(ch);
            continue;
        }
        if started && ch == '.' {
            out.push(ch);
            continue;
        }
        if started {
            break;
        }
    }

    if out.is_empty() { None } else { Some(out) }
}

fn infer_modrinth_loader_from_mod_filenames(path: &Path) -> Option<(String, Option<String>)> {
    let mods_dir = path.join("mods");
    let entries = fs::read_dir(mods_dir).ok()?;
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if file_name.contains("fabric") {
            return Some(("Fabric".to_owned(), None));
        }
        if file_name.contains("quilt") {
            return Some(("Quilt".to_owned(), None));
        }
        if file_name.contains("neoforge") {
            return Some(("NeoForge".to_owned(), None));
        }
        if file_name.contains("forge") {
            return Some(("Forge".to_owned(), None));
        }
    }
    None
}

fn infer_modrinth_game_version_from_filenames(path: &Path) -> Option<String> {
    let mods_dir = path.join("mods");
    let entries = fs::read_dir(mods_dir).ok()?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if let Some(version) = find_minecraft_version_in_text(file_name.as_ref()) {
            return Some(version);
        }
    }
    None
}

fn find_minecraft_version_in_text(text: &str) -> Option<String> {
    let chars = text.chars().collect::<Vec<_>>();
    for start in 0..chars.len() {
        if !chars[start].is_ascii_digit() {
            continue;
        }
        let mut end = start;
        let mut dot_count = 0usize;
        while end < chars.len() && (chars[end].is_ascii_digit() || chars[end] == '.') {
            if chars[end] == '.' {
                dot_count += 1;
            }
            end += 1;
        }
        if dot_count >= 2 {
            let candidate = chars[start..end].iter().collect::<String>();
            if candidate.split('.').all(|segment| !segment.is_empty()) {
                return Some(candidate);
            }
        }
    }
    None
}

fn modrinth_app_root(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        ancestor
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name == "ModrinthApp")
            .then(|| ancestor.to_path_buf())
    })
}

fn refine_modrinth_metadata_from_meta_cache(
    app_root: &Path,
    metadata: &mut ModrinthProfileMetadata,
) {
    let versions_dir = app_root.join("meta").join("versions");
    let Ok(entries) = fs::read_dir(versions_dir) else {
        return;
    };

    let game_version = metadata.game_version.clone();
    for entry in entries.flatten() {
        let version_name = entry.file_name().to_string_lossy().to_string();
        let Some(version_json) =
            read_meta_version_file(entry.path().as_path(), version_name.as_str())
        else {
            continue;
        };

        if let Some(expected_game_version) = game_version.as_deref()
            && !version_name.starts_with(expected_game_version)
        {
            continue;
        }

        if metadata.modloader.is_none() || metadata.modloader_version.is_none() {
            if let Some((loader, loader_version)) = infer_loader_from_meta_version(&version_json) {
                metadata.modloader.get_or_insert(loader);
                metadata.modloader_version.get_or_insert(loader_version);
            }
        }

        if metadata.game_version.is_none()
            && let Some(id) = version_json.get("id").and_then(Value::as_str)
            && let Some(version) = id.split('-').next()
            && !version.trim().is_empty()
        {
            metadata.game_version = Some(version.to_owned());
        }

        if metadata.game_version.is_some()
            && metadata.modloader.is_some()
            && metadata.modloader_version.is_some()
        {
            break;
        }
    }
}

fn read_meta_version_file(dir: &Path, dir_name: &str) -> Option<Value> {
    let path = dir.join(format!("{dir_name}.json"));
    read_json_file_optional(path.as_path()).ok().flatten()
}

fn infer_loader_from_meta_version(value: &Value) -> Option<(String, String)> {
    let libraries = value.get("libraries")?.as_array()?;
    for library in libraries {
        let name = library
            .get("name")
            .and_then(Value::as_str)?
            .to_ascii_lowercase();
        if let Some(version) = name.strip_prefix("net.fabricmc:fabric-loader:") {
            return Some(("Fabric".to_owned(), version.to_owned()));
        }
        if let Some(version) = name.strip_prefix("org.quiltmc:quilt-loader:") {
            return Some(("Quilt".to_owned(), version.to_owned()));
        }
        if let Some(version) = name.strip_prefix("net.neoforged:neoforge:") {
            return Some(("NeoForge".to_owned(), version.to_owned()));
        }
        if let Some(version) = name.strip_prefix("net.minecraftforge:forge:") {
            return Some(("Forge".to_owned(), version.to_owned()));
        }
    }
    None
}

fn inspect_curseforge_launcher_instance(path: &Path) -> Result<LauncherInspection, String> {
    let manifest = read_json_file(path.join("minecraftinstance.json").as_path())?;
    let source_root = path.to_path_buf();
    let name = first_non_empty([
        json_string_at_path(&manifest, &["name"]),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported CurseForge Instance".to_owned());
    let game_version = first_non_empty([
        json_string_at_path(&manifest, &["gameVersion"]),
        json_string_at_path(&manifest, &["minecraftVersion"]),
        json_string_at_path(&manifest, &["baseModLoader", "minecraftVersion"]),
    ])
    .unwrap_or_else(|| "latest".to_owned());
    let loader_hint = first_non_empty([
        json_string_at_path(&manifest, &["baseModLoader", "name"]),
        json_string_at_path(&manifest, &["baseModLoader", "modLoader"]),
        json_string_at_path(&manifest, &["modLoader"]),
    ]);
    let loader_version_hint = first_non_empty([
        json_string_at_path(&manifest, &["baseModLoader", "forgeVersion"]),
        json_string_at_path(&manifest, &["baseModLoader", "version"]),
        json_string_at_path(&manifest, &["modLoaderVersion"]),
    ]);
    let (modloader, modloader_version) = infer_loader_pair(loader_hint, loader_version_hint);
    let mut managed_manifest =
        load_existing_managed_manifest(path).unwrap_or_else(|_| ManagedContentManifest::default());
    if managed_manifest.projects.is_empty() {
        managed_manifest = extract_managed_manifest_from_json(
            &manifest,
            source_root.as_path(),
            ManagedContentSourceHint::CurseForge,
        );
    }
    Ok(build_launcher_inspection(
        LauncherKind::CurseForge,
        name,
        Some("Imported from an existing CurseForge instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

fn inspect_prism_launcher_instance(path: &Path) -> Result<LauncherInspection, String> {
    let source_root = if path.join(".minecraft").is_dir() {
        path.join(".minecraft")
    } else {
        path.to_path_buf()
    };
    let cfg = read_key_value_file(path.join("instance.cfg").as_path()).unwrap_or_default();
    let pack_json = read_json_file_optional(path.join("mmc-pack.json").as_path())?;
    let name = first_non_empty([
        cfg.get("name").cloned(),
        pack_json
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["name"])),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported Prism Instance".to_owned());
    let (game_version, modloader, modloader_version) =
        parse_prism_versions(pack_json.as_ref(), cfg.get("MCVersion").cloned());
    let managed_manifest =
        load_existing_managed_manifest(source_root.as_path()).unwrap_or_default();
    Ok(build_launcher_inspection(
        LauncherKind::Prism,
        name,
        Some("Imported from a Prism / MultiMC style instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

fn inspect_atlauncher_instance(path: &Path) -> Result<LauncherInspection, String> {
    let manifest = read_json_file_optional(path.join("instance.json").as_path())?;
    let source_root = path.to_path_buf();
    let name = first_non_empty([
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["name"])),
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
    ])
    .unwrap_or_else(|| "Imported ATLauncher Instance".to_owned());
    let game_version = first_non_empty([
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["minecraft", "version"])),
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["minecraftVersion"])),
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["version"])),
    ])
    .unwrap_or_else(|| "latest".to_owned());
    let (modloader, modloader_version) = infer_loader_pair(
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["loader"])),
        manifest
            .as_ref()
            .and_then(|value| json_string_at_path(value, &["loaderVersion"])),
    );
    let mut managed_manifest =
        load_existing_managed_manifest(path).unwrap_or_else(|_| ManagedContentManifest::default());
    if managed_manifest.projects.is_empty()
        && let Some(value) = manifest.as_ref()
    {
        managed_manifest = extract_managed_manifest_from_json(
            value,
            source_root.as_path(),
            ManagedContentSourceHint::Auto,
        );
    }
    Ok(build_launcher_inspection(
        LauncherKind::ATLauncher,
        name,
        Some("Imported from an existing ATLauncher instance.".to_owned()),
        game_version,
        modloader,
        modloader_version,
        source_root,
        managed_manifest,
    ))
}

fn inspect_generic_launcher_instance(path: &Path) -> Result<LauncherInspection, String> {
    inspect_generic_launcher_instance_with_launcher(path, LauncherKind::Unknown)
}

fn inspect_generic_launcher_instance_with_launcher(
    path: &Path,
    launcher: LauncherKind,
) -> Result<LauncherInspection, String> {
    if !path.is_dir() {
        return Err(format!("{} is not a directory.", path.display()));
    }
    let managed_manifest = load_existing_managed_manifest(path).unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| "Imported Instance".to_owned());
    Ok(build_launcher_inspection(
        launcher,
        name,
        Some(format!(
            "Imported by copying files from {}.",
            launcher.label()
        )),
        "latest".to_owned(),
        "Vanilla".to_owned(),
        String::new(),
        path.to_path_buf(),
        managed_manifest,
    ))
}

fn build_launcher_inspection(
    launcher: LauncherKind,
    name: String,
    description: Option<String>,
    game_version: String,
    modloader: String,
    modloader_version: String,
    source_root: PathBuf,
    managed_manifest: ManagedContentManifest,
) -> LauncherInspection {
    let mods_count = count_regular_files(source_root.join("mods").as_path());
    let config_count = count_regular_files(source_root.join("config").as_path());
    let managed_count = managed_manifest.projects.len();
    LauncherInspection {
        launcher,
        name,
        description,
        game_version: default_if_blank(game_version.as_str(), "latest".to_owned()),
        modloader: default_if_blank(modloader.as_str(), "Vanilla".to_owned()),
        modloader_version,
        summary: format!(
            "Detected {} with {} managed projects, {} mods, and {} config files.",
            launcher.label(),
            managed_count,
            mods_count,
            config_count
        ),
        source_root,
        managed_manifest,
    }
}

fn copy_launcher_instance_content(
    source_root: &Path,
    destination_root: &Path,
    managed_manifest: &ManagedContentManifest,
) -> Result<(), String> {
    copy_dir_recursive(source_root, source_root, destination_root)?;
    if !managed_manifest.projects.is_empty() {
        let raw = toml::to_string_pretty(managed_manifest)
            .map_err(|err| format!("failed to serialize managed import manifest: {err}"))?;
        fs::write(
            destination_root.join(MANAGED_CONTENT_MANIFEST_FILE_NAME),
            raw,
        )
        .map_err(|err| {
            format!(
                "failed to write managed import manifest into {}: {err}",
                destination_root.display()
            )
        })?;
    }
    Ok(())
}

fn copy_dir_recursive(root: &Path, current: &Path, destination_root: &Path) -> Result<(), String> {
    let entries = fs::read_dir(current)
        .map_err(|err| format!("failed to read {}: {err}", current.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|err| format!("failed to normalize {}: {err}", path.display()))?;
        if should_skip_import_path(relative) {
            continue;
        }
        let destination = destination_root.join(relative);
        if path.is_dir() {
            fs::create_dir_all(destination.as_path())
                .map_err(|err| format!("failed to create {}: {err}", destination.display()))?;
            copy_dir_recursive(root, path.as_path(), destination_root)?;
        } else if path.is_file() {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
            }
            fs::copy(path.as_path(), destination.as_path()).map_err(|err| {
                format!(
                    "failed to copy {} to {}: {err}",
                    path.display(),
                    destination.display()
                )
            })?;
        }
    }
    Ok(())
}

fn should_skip_import_path(relative: &Path) -> bool {
    let normalized = relative.to_string_lossy().replace('\\', "/");
    if normalized.is_empty() {
        return false;
    }
    let skip_exact = [
        "instance.cfg",
        "mmc-pack.json",
        "profile.json",
        "minecraftinstance.json",
        "instance.json",
        MANAGED_CONTENT_MANIFEST_FILE_NAME,
    ];
    if skip_exact
        .iter()
        .any(|candidate| normalized.eq_ignore_ascii_case(candidate))
    {
        return true;
    }
    let skip_prefixes = [
        "logs/",
        "crash-reports/",
        "versions/",
        "libraries/",
        "natives/",
        ".cache/",
        "cache/",
        "downloads/",
    ];
    skip_prefixes
        .iter()
        .any(|prefix| normalized.to_ascii_lowercase().starts_with(prefix))
}

fn read_json_file(path: &Path) -> Result<Value, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn read_json_file_optional(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    read_json_file(path).map(Some)
}

fn read_key_value_file(path: &Path) -> Result<HashMap<String, String>, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut values = HashMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            values.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    Ok(values)
}

fn parse_prism_versions(
    pack_json: Option<&Value>,
    cfg_game_version: Option<String>,
) -> (String, String, String) {
    let mut game_version = cfg_game_version.unwrap_or_else(|| "latest".to_owned());
    let mut loader = "Vanilla".to_owned();
    let mut loader_version = String::new();

    if let Some(Value::Array(components)) =
        pack_json.and_then(|value| value.get("components")).cloned()
    {
        for component in components {
            let uid = component
                .get("uid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let version = component
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if uid.contains("minecraft") && game_version == "latest" && !version.trim().is_empty() {
                game_version = version.clone();
            }
            if uid.contains("fabric") {
                loader = "Fabric".to_owned();
                loader_version = version;
            } else if uid.contains("neoforge") {
                loader = "NeoForge".to_owned();
                loader_version = version;
            } else if uid.contains("forge") {
                loader = "Forge".to_owned();
                loader_version = version;
            } else if uid.contains("quilt") {
                loader = "Quilt".to_owned();
                loader_version = version;
            }
        }
    }

    (game_version, loader, loader_version)
}

fn json_string_at_path(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn first_non_empty<const N: usize>(values: [Option<String>; N]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
}

fn infer_loader_pair(
    loader_hint: Option<String>,
    version_hint: Option<String>,
) -> (String, String) {
    let loader_hint = loader_hint.unwrap_or_else(|| "Vanilla".to_owned());
    let loader_hint_trimmed = loader_hint.trim().to_owned();
    let loader_hint_lower = loader_hint_trimmed.to_ascii_lowercase();
    let version_hint = version_hint.unwrap_or_default();
    if loader_hint_lower.contains("neoforge") {
        return (
            "NeoForge".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    if loader_hint_lower.contains("fabric") {
        return (
            "Fabric".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    if loader_hint_lower.contains("quilt") {
        return (
            "Quilt".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    if loader_hint_lower.contains("forge") {
        return (
            "Forge".to_owned(),
            trailing_loader_version(&loader_hint_trimmed, &version_hint),
        );
    }
    (
        default_if_blank(loader_hint_trimmed.as_str(), "Vanilla".to_owned()),
        version_hint,
    )
}

fn trailing_loader_version(loader_hint: &str, explicit_version: &str) -> String {
    let explicit = explicit_version.trim();
    if !explicit.is_empty() {
        return explicit.to_owned();
    }
    loader_hint
        .split_once('-')
        .map(|(_, version)| version.trim().to_owned())
        .unwrap_or_default()
}

fn load_existing_managed_manifest(path: &Path) -> Result<ManagedContentManifest, String> {
    let manifest_path = path.join(MANAGED_CONTENT_MANIFEST_FILE_NAME);
    if !manifest_path.exists() {
        return Ok(ManagedContentManifest::default());
    }
    let raw = fs::read_to_string(manifest_path.as_path())
        .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;
    toml::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", manifest_path.display()))
}

#[derive(Clone, Copy)]
enum ManagedContentSourceHint {
    Auto,
    Modrinth,
    CurseForge,
}

fn extract_managed_manifest_from_json(
    value: &Value,
    source_root: &Path,
    source_hint: ManagedContentSourceHint,
) -> ManagedContentManifest {
    let mut manifest = ManagedContentManifest::default();
    walk_json_for_projects(value, source_root, source_hint, &mut manifest);
    manifest
}

fn walk_json_for_projects(
    value: &Value,
    source_root: &Path,
    source_hint: ManagedContentSourceHint,
    manifest: &mut ManagedContentManifest,
) {
    maybe_add_project_from_json(value, source_root, source_hint, manifest);
    match value {
        Value::Object(map) => {
            for child in map.values() {
                walk_json_for_projects(child, source_root, source_hint, manifest);
            }
        }
        Value::Array(values) => {
            for child in values {
                walk_json_for_projects(child, source_root, source_hint, manifest);
            }
        }
        _ => {}
    }
}

fn maybe_add_project_from_json(
    value: &Value,
    source_root: &Path,
    source_hint: ManagedContentSourceHint,
    manifest: &mut ManagedContentManifest,
) {
    let Value::Object(map) = value else {
        return;
    };

    let modrinth_project_id = json_object_string(
        map,
        &[
            "project_id",
            "projectId",
            "modrinth_project_id",
            "modrinthProjectId",
        ],
    );
    let curseforge_project_id = json_object_u64(
        map,
        &[
            "addonID",
            "addonId",
            "projectID",
            "projectId",
            "curseforge_project_id",
            "curseforgeProjectId",
        ],
    );
    let source = match source_hint {
        ManagedContentSourceHint::Modrinth if modrinth_project_id.is_some() => Some("modrinth"),
        ManagedContentSourceHint::CurseForge if curseforge_project_id.is_some() => {
            Some("curseforge")
        }
        ManagedContentSourceHint::Auto => {
            if modrinth_project_id.is_some() {
                Some("modrinth")
            } else if curseforge_project_id.is_some() {
                Some("curseforge")
            } else {
                None
            }
        }
        _ => None,
    };
    if source.is_none() {
        return;
    }

    let version_id = first_non_empty([
        json_object_string(
            map,
            &[
                "version_id",
                "versionId",
                "fileId",
                "fileID",
                "gameVersionFileID",
            ],
        ),
        map.get("installedFile")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_u64)
            .map(|value| value.to_string()),
    ])
    .unwrap_or_default();
    let metadata_file_name = json_object_string(map, &["fileName", "filename", "file_name"])
        .or_else(|| {
            map.get("installedFile")
                .and_then(|value| value.get("fileName"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let version_name = json_object_string(map, &["version_name", "versionName"])
        .or_else(|| metadata_file_name.clone())
        .unwrap_or_default();
    let name = json_object_string(map, &["name", "title"])
        .or_else(|| {
            map.get("installedFile")
                .and_then(|value| value.get("displayName"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| version_name.clone());

    let Some(metadata_file_name) = metadata_file_name.as_deref() else {
        return;
    };

    let file_path = first_non_empty([
        json_object_string(map, &["path", "file_path", "filePath"]).and_then(|value| {
            resolve_existing_relative_file_path(source_root, value.as_str(), metadata_file_name)
        }),
        Some(metadata_file_name.to_owned()).and_then(|value| {
            resolve_existing_relative_file_path(source_root, value.as_str(), metadata_file_name)
        }),
    ]);
    let Some(file_path) = file_path else {
        return;
    };

    let project_key = if let Some(id) = modrinth_project_id.as_ref() {
        format!("modrinth:{id}")
    } else if let Some(id) = curseforge_project_id {
        format!("curseforge:{id}")
    } else {
        normalize_project_key(file_path.as_str())
    };
    manifest.projects.insert(
        project_key.clone(),
        ManagedContentManifestProject {
            project_key,
            name,
            file_path,
            modrinth_project_id,
            curseforge_project_id,
            selected_source: source.map(str::to_owned),
            selected_version_id: non_empty(version_id.as_str()),
            selected_version_name: non_empty(version_name.as_str()),
        },
    );
}

fn json_object_string(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn json_object_u64(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        map.get(*key).and_then(|value| match value {
            Value::Number(number) => number.as_u64(),
            Value::String(raw) => raw.trim().parse::<u64>().ok(),
            _ => None,
        })
    })
}

fn resolve_existing_relative_file_path(
    source_root: &Path,
    raw: &str,
    expected_file_name: &str,
) -> Option<String> {
    let normalized = normalize_project_key(raw);
    if normalized.is_empty() {
        return None;
    }

    let direct = source_root.join(normalized.as_str());
    if direct.is_file() && file_name_matches(direct.as_path(), expected_file_name) {
        return Some(normalized);
    }

    let known_dirs = ["mods", "resourcepacks", "shaderpacks", "datapacks"];
    for dir in known_dirs {
        let candidate = source_root.join(dir).join(raw);
        if candidate.is_file() && file_name_matches(candidate.as_path(), expected_file_name) {
            return Some(format!("{dir}/{}", raw.trim()));
        }
    }

    None
}

fn file_name_matches(path: &Path, expected_file_name: &str) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == expected_file_name.trim())
}

fn normalize_project_key(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_lowercase()
}

fn count_regular_files(path: &Path) -> usize {
    if !path.exists() {
        return 0;
    }
    count_regular_files_recursive(path).unwrap_or(0)
}

fn count_regular_files_recursive(path: &Path) -> Result<usize, String> {
    let mut count = 0usize;
    let entries =
        fs::read_dir(path).map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            count += count_regular_files_recursive(entry_path.as_path())?;
        } else if entry_path.is_file() {
            count += 1;
        }
    }
    Ok(count)
}

fn import_vtmpack(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
) -> Result<InstanceRecord, String> {
    let ImportSource::ManifestFile(package_path) = &request.source else {
        return Err("Vertex pack import requires a manifest file source.".to_owned());
    };
    let manifest = read_vtmpack_manifest(package_path.as_path())?;
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: None,
            thumbnail_path: None,
            modloader: default_if_blank(manifest.instance.modloader.as_str(), "Vanilla".to_owned()),
            game_version: default_if_blank(
                manifest.instance.game_version.as_str(),
                "latest".to_owned(),
            ),
            modloader_version: manifest.instance.modloader_version.clone(),
        },
    )
    .map_err(|err| format!("failed to create imported profile: {err}"))?;
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) =
        populate_vtmpack_instance(package_path.as_path(), manifest, instance_root.as_path())
    {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    Ok(instance)
}

fn import_mrpack(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
) -> Result<InstanceRecord, String> {
    let ImportSource::ManifestFile(package_path) = &request.source else {
        return Err("Modrinth pack import requires a manifest file source.".to_owned());
    };
    let manifest = read_mrpack_manifest(package_path.as_path())?;
    let dependency_info = resolve_mrpack_dependencies(&manifest.dependencies)?;
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: non_empty(manifest.summary.as_deref().unwrap_or_default()),
            thumbnail_path: None,
            modloader: dependency_info.modloader.clone(),
            game_version: dependency_info.game_version.clone(),
            modloader_version: dependency_info.modloader_version.clone(),
        },
    )
    .map_err(|err| format!("failed to create imported profile: {err}"))?;
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) =
        populate_mrpack_instance(package_path.as_path(), manifest, instance_root.as_path())
    {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    Ok(instance)
}

fn populate_vtmpack_instance(
    package_path: &Path,
    manifest: VtmpackManifest,
    instance_root: &Path,
) -> Result<(), String> {
    extract_vtmpack_payload(package_path, instance_root)?;

    for downloadable in &manifest.downloadable_content {
        if downloadable.file_path.trim().is_empty() {
            continue;
        }
        let destination = join_safe(instance_root, downloadable.file_path.as_str())?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create import directory {}: {err}",
                    parent.display()
                )
            })?;
        }
        download_vtmpack_entry(downloadable, destination.as_path())?;
    }

    Ok(())
}

fn extract_vtmpack_payload(package_path: &Path, instance_root: &Path) -> Result<(), String> {
    let file = fs::File::open(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let decoder = xz2::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive
        .entries()
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?
    {
        let mut entry = entry.map_err(|err| {
            format!(
                "failed to read archive entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_path = entry
            .path()
            .map_err(|err| format!("failed to decode archive path: {err}"))?
            .to_path_buf();
        let entry_string = entry_path.to_string_lossy().replace('\\', "/");

        if entry_string == "manifest.toml" {
            continue;
        }
        if entry_string == format!("metadata/{MANAGED_CONTENT_MANIFEST_FILE_NAME}") {
            let destination = instance_root.join(MANAGED_CONTENT_MANIFEST_FILE_NAME);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    format!(
                        "failed to create metadata directory {}: {err}",
                        parent.display()
                    )
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                format!(
                    "failed to restore managed metadata into {}: {err}",
                    destination.display()
                )
            })?;
            continue;
        }
        if let Some(relative) = entry_string.strip_prefix("bundled_mods/") {
            let destination = join_safe(&instance_root.join("mods"), relative)?;
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    format!(
                        "failed to create bundled mod directory {}: {err}",
                        parent.display()
                    )
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                format!(
                    "failed to import bundled mod {}: {err}",
                    destination.display()
                )
            })?;
            continue;
        }
        if let Some(relative) = entry_string.strip_prefix("configs/") {
            let destination = join_safe(&instance_root.join("config"), relative)?;
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    format!(
                        "failed to create config directory {}: {err}",
                        parent.display()
                    )
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                format!("failed to import config {}: {err}", destination.display())
            })?;
            continue;
        }
        if let Some(relative) = entry_string.strip_prefix("root_entries/") {
            let destination = join_safe(instance_root, relative)?;
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    format!(
                        "failed to create imported root entry directory {}: {err}",
                        parent.display()
                    )
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                format!(
                    "failed to import extra root entry {}: {err}",
                    destination.display()
                )
            })?;
        }
    }
    Ok(())
}

fn download_vtmpack_entry(
    entry: &VtmpackDownloadableEntry,
    destination: &Path,
) -> Result<(), String> {
    match normalize_source_name(entry.selected_source.as_deref()) {
        Some(ManagedSource::Modrinth) => {
            let version_id = entry
                .selected_version_id
                .as_deref()
                .ok_or_else(|| format!("missing Modrinth version id for {}", entry.name))?;
            let version = ModrinthClient::default()
                .get_version(version_id)
                .map_err(|err| format!("failed to fetch Modrinth version {version_id}: {err}"))?;
            let file = version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())
                .ok_or_else(|| {
                    format!("no downloadable file found for Modrinth version {version_id}")
                })?;
            download_file(file.url.as_str(), destination)
        }
        Some(ManagedSource::CurseForge) => {
            let project_id = entry
                .curseforge_project_id
                .ok_or_else(|| format!("missing CurseForge project id for {}", entry.name))?;
            let file_id = entry
                .selected_version_id
                .as_deref()
                .ok_or_else(|| format!("missing CurseForge file id for {}", entry.name))?
                .parse::<u64>()
                .map_err(|err| format!("invalid CurseForge file id for {}: {err}", entry.name))?;
            let client = CurseForgeClient::from_env().ok_or_else(|| {
                "CurseForge API key missing; set VERTEX_CURSEFORGE_API_KEY or CURSEFORGE_API_KEY to import this pack."
                    .to_owned()
            })?;
            let file = find_curseforge_file(&client, project_id, file_id)?;
            let download_url = file.download_url.ok_or_else(|| {
                format!("CurseForge file {file_id} for project {project_id} has no download URL")
            })?;
            download_file(download_url.as_str(), destination)
        }
        None => {
            if let Some(version_id) = entry.selected_version_id.as_deref() {
                let version = ModrinthClient::default()
                    .get_version(version_id)
                    .map_err(|err| {
                        format!("failed to fetch Modrinth fallback version {version_id}: {err}")
                    })?;
                let file = version
                    .files
                    .iter()
                    .find(|file| file.primary)
                    .or_else(|| version.files.first())
                    .ok_or_else(|| {
                        format!("no downloadable file found for Modrinth version {version_id}")
                    })?;
                return download_file(file.url.as_str(), destination);
            }
            Err(format!(
                "download source for {} could not be determined from the pack metadata",
                entry.name
            ))
        }
    }
}

fn find_curseforge_file(
    client: &CurseForgeClient,
    project_id: u64,
    file_id: u64,
) -> Result<curseforge::File, String> {
    let mut index = 0u32;
    loop {
        let files = client
            .list_mod_files(project_id, None, None, index, 50)
            .map_err(|err| format!("failed to list CurseForge files for {project_id}: {err}"))?;
        if files.is_empty() {
            break;
        }
        if let Some(found) = files.into_iter().find(|file| file.id == file_id) {
            return Ok(found);
        }
        index += 50;
    }
    Err(format!(
        "CurseForge file {file_id} was not found for project {project_id}"
    ))
}

fn populate_mrpack_instance(
    package_path: &Path,
    manifest: MrpackManifest,
    instance_root: &Path,
) -> Result<(), String> {
    extract_mrpack_overrides(package_path, instance_root)?;
    for file in manifest.files {
        if matches!(
            file.env.as_ref().and_then(|env| env.client.as_deref()),
            Some("unsupported")
        ) {
            continue;
        }
        let destination = join_safe(instance_root, file.path.as_str())?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create import directory {}: {err}",
                    parent.display()
                )
            })?;
        }
        let download_url = file
            .downloads
            .first()
            .cloned()
            .ok_or_else(|| format!("Modrinth pack entry {} has no download URL", file.path))?;
        download_file(download_url.as_str(), destination.as_path())?;
    }
    Ok(())
}

fn extract_mrpack_overrides(package_path: &Path, instance_root: &Path) -> Result<(), String> {
    let file = fs::File::open(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|err| {
            format!(
                "failed to read zip entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_name = entry.name().replace('\\', "/");
        let Some(relative) = entry_name
            .strip_prefix("overrides/")
            .or_else(|| entry_name.strip_prefix("client-overrides/"))
        else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }
        let destination = join_safe(instance_root, relative)?;
        if entry.is_dir() {
            fs::create_dir_all(destination.as_path()).map_err(|err| {
                format!(
                    "failed to create override directory {}: {err}",
                    destination.display()
                )
            })?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create override parent {}: {err}",
                    parent.display()
                )
            })?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|err| {
            format!(
                "failed to read override {} from {}: {err}",
                entry_name,
                package_path.display()
            )
        })?;
        fs::write(destination.as_path(), bytes)
            .map_err(|err| format!("failed to write override {}: {err}", destination.display()))?;
    }

    Ok(())
}

fn read_vtmpack_manifest(path: &Path) -> Result<VtmpackManifest, String> {
    let file =
        fs::File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let decoder = xz2::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive
        .entries()
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?
    {
        let mut entry = entry.map_err(|err| format!("failed to read archive entry: {err}"))?;
        let entry_path = entry
            .path()
            .map_err(|err| format!("failed to decode archive path: {err}"))?;
        if entry_path == Path::new("manifest.toml") {
            let mut raw = String::new();
            entry
                .read_to_string(&mut raw)
                .map_err(|err| format!("failed to read manifest.toml: {err}"))?;
            return toml::from_str(&raw)
                .map_err(|err| format!("failed to parse vtmpack manifest: {err}"));
        }
    }

    Err(format!(
        "No manifest.toml found in Vertex pack {}",
        path.display()
    ))
}

fn read_mrpack_manifest(path: &Path) -> Result<MrpackManifest, String> {
    let file =
        fs::File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut manifest = archive
        .by_name("modrinth.index.json")
        .map_err(|err| format!("missing modrinth.index.json in {}: {err}", path.display()))?;
    let mut raw = String::new();
    manifest
        .read_to_string(&mut raw)
        .map_err(|err| format!("failed to read modrinth.index.json: {err}"))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse modrinth.index.json: {err}"))
}

fn resolve_mrpack_dependencies(
    dependencies: &HashMap<String, String>,
) -> Result<MrpackDependencyInfo, String> {
    let game_version = dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(|| "Modrinth pack is missing the required minecraft dependency.".to_owned())?;

    let loader_candidates = [
        ("neoforge", "NeoForge"),
        ("forge", "Forge"),
        ("fabric-loader", "Fabric"),
        ("quilt-loader", "Quilt"),
    ];
    for (key, label) in loader_candidates {
        if let Some(version) = dependencies.get(key) {
            return Ok(MrpackDependencyInfo {
                game_version,
                modloader: label.to_owned(),
                modloader_version: version.clone(),
            });
        }
    }

    Ok(MrpackDependencyInfo {
        game_version,
        modloader: "Vanilla".to_owned(),
        modloader_version: String::new(),
    })
}

fn normalize_source_name(source: Option<&str>) -> Option<ManagedSource> {
    match source?.trim().to_ascii_lowercase().as_str() {
        "modrinth" => Some(ManagedSource::Modrinth),
        "curseforge" => Some(ManagedSource::CurseForge),
        _ => None,
    }
}

fn join_safe(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    let mut clean = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "unsafe path in import package: {}",
                    relative.display()
                ));
            }
        }
    }
    Ok(root.join(clean))
}

fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    let response = ureq::get(url)
        .call()
        .map_err(|err| format!("download request failed for {url}: {err}"))?;
    let mut reader = response.into_body().into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read download body from {url}: {err}"))?;
    fs::write(destination, bytes)
        .map_err(|err| format!("failed to write {}: {err}", destination.display()))
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn default_if_blank(value: &str, fallback: String) -> String {
    non_empty(value).unwrap_or(fallback)
}

fn format_loader_label(modloader: &str, version: &str) -> String {
    let version = version.trim();
    if version.is_empty() {
        modloader.trim().to_owned()
    } else {
        format!("{} {}", modloader.trim(), version)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedSource {
    Modrinth,
    CurseForge,
}

#[derive(Debug)]
struct MrpackDependencyInfo {
    game_version: String,
    modloader: String,
    modloader_version: String,
}

#[derive(Debug, Clone, Deserialize)]
struct VtmpackManifest {
    instance: VtmpackInstanceMetadata,
    #[serde(default)]
    downloadable_content: Vec<VtmpackDownloadableEntry>,
    #[serde(default)]
    bundled_mods: Vec<String>,
    #[serde(default)]
    configs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct VtmpackInstanceMetadata {
    name: String,
    game_version: String,
    modloader: String,
    #[serde(default)]
    modloader_version: String,
}

#[derive(Debug, Clone, Deserialize)]
struct VtmpackDownloadableEntry {
    #[serde(default)]
    name: String,
    file_path: String,
    #[serde(default)]
    curseforge_project_id: Option<u64>,
    #[serde(default)]
    selected_source: Option<String>,
    #[serde(default)]
    selected_version_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MrpackManifest {
    #[serde(default)]
    name: String,
    #[serde(rename = "versionId", default)]
    version_id: String,
    #[serde(default)]
    summary: Option<String>,
    dependencies: HashMap<String, String>,
    #[serde(default)]
    files: Vec<MrpackFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct MrpackFile {
    path: String,
    #[serde(default)]
    downloads: Vec<String>,
    #[serde(default)]
    env: Option<MrpackFileEnv>,
}

#[derive(Debug, Clone, Deserialize)]
struct MrpackFileEnv {
    #[serde(default)]
    client: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ManagedContentManifest {
    #[serde(default)]
    projects: BTreeMap<String, ManagedContentManifestProject>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ManagedContentManifestProject {
    #[serde(default)]
    project_key: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    file_path: String,
    #[serde(default)]
    modrinth_project_id: Option<String>,
    #[serde(default)]
    curseforge_project_id: Option<u64>,
    #[serde(default)]
    selected_source: Option<String>,
    #[serde(default)]
    selected_version_id: Option<String>,
    #[serde(default)]
    selected_version_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_mrpack_dependencies_for_fabric() {
        let dependencies = HashMap::from([
            ("minecraft".to_owned(), "1.21.1".to_owned()),
            ("fabric-loader".to_owned(), "0.16.10".to_owned()),
        ]);

        let resolved = resolve_mrpack_dependencies(&dependencies).expect("expected dependencies");
        assert_eq!(resolved.game_version, "1.21.1");
        assert_eq!(resolved.modloader, "Fabric");
        assert_eq!(resolved.modloader_version, "0.16.10");
    }

    #[test]
    fn safe_join_rejects_parent_traversal() {
        let result = join_safe(Path::new("/tmp/root"), "../mods/evil.jar");
        assert!(result.is_err());
    }
}
