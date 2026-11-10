//! The "Create instance" dialog: name, Minecraft version, modloader and loader version. Creating registers the
//! instance and starts downloading its game files, exactly as the egui dialog does.

use vertex_i18n::tr;
use installation::{VersionCatalog, VersionCatalogFilter, fetch_version_catalog_with_refresh};
use instances::{InstanceRecord, InstanceStore, NewInstanceSpec, create_instance};
use vertex_shell::sturdy::ui::{Sizing, TextStyle, TextWrap, Ui};
use vertex_shell::sturdy::ui_kit::ModalStyle;

use super::super::startup::start_initial_instance_install_with;
use super::AppState;
use super::kit::{self, ButtonKind, Kit, TextRole};

const LOADERS: [&str; 5] = ["Vanilla", "Fabric", "Forge", "NeoForge", "Quilt"];
const LATEST_LOADER_VERSION: &str = vertex_i18n::src!("Latest");

#[derive(Default)]
pub struct CreateState {
    pub name: String,
    version: usize,
    loader: usize,
    loader_version: usize,
    catalog: Option<VersionCatalog>,
    catalog_filter: Option<VersionCatalogFilter>,
    catalog_error: Option<String>,
    catalog_results:
        launcher_runtime::WorkerChannel<(VersionCatalogFilter, Result<VersionCatalog, String>)>,
    catalog_in_flight: bool,
    create_results:
        launcher_runtime::WorkerChannel<Result<(InstanceStore, InstanceRecord), String>>,
    create_in_flight: bool,
    error: Option<String>,
}

impl CreateState {
    fn reset(&mut self) {
        let catalog = self.catalog.take();
        let filter = self.catalog_filter.take();
        *self = Self::default();
        self.catalog = catalog;
        self.catalog_filter = filter;
    }

    fn request_catalog(&mut self, filter: VersionCatalogFilter, force_refresh: bool) {
        if self.catalog_in_flight {
            return;
        }
        self.catalog_in_flight = true;
        self.catalog_error = None;
        let tx = self.catalog_results.sender();
        launcher_runtime::spawn_blocking_detached(move || {
            let result = fetch_version_catalog_with_refresh(filter, force_refresh)
                .map_err(|err| err.to_string());
            let _ = tx.send((filter, result));
        });
    }
}

fn version_filter(state: &AppState) -> VersionCatalogFilter {
    VersionCatalogFilter {
        include_snapshots_and_betas: state.config.include_snapshots_and_betas(),
        include_alpha: state.config.include_alpha_versions(),
        include_experimental: state.config.include_experimental_versions(),
    }
}

/// Collects catalog and create results. Runs every frame, also while the dialog is closed.
pub fn poll(state: &mut AppState) {
    for (filter, result) in state.create.catalog_results.drain().items {
        state.create.catalog_in_flight = false;
        match result {
            Ok(catalog) => {
                state.create.catalog = Some(catalog);
                state.create.catalog_filter = Some(filter);
            }
            Err(err) => state.create.catalog_error = Some(err),
        }
    }
    let results = state.create.create_results.drain().items;
    for result in results {
        state.create.create_in_flight = false;
        match result {
            Ok((store, instance)) => {
                state.instances = store;
                let root = state.installations_root();
                let config = state.config.clone();
                let tx = state.initial_install_sender();
                start_initial_instance_install_with(Some(tx), &instance, root.as_path(), &config);
                state.open_instance(&instance.id);
                state.create.reset();
                if state.dialog == Some(super::Dialog::CreateInstance) {
                    state.dialog = None;
                }
            }
            Err(err) => state.create.error = Some(err),
        }
    }
}

/// Returns false once the dialog should close.
pub fn dialog(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) -> bool {
    let filter = version_filter(state);
    if state.create.catalog_filter != Some(filter) && state.create.catalog_error.is_none() {
        state.create.request_catalog(filter, false);
    }
    let style = ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(560.0),
        ..ModalStyle::default()
    };
    let mut close = false;
    let mut create = None;
    let mut refresh = false;
    let busy = state.create.create_in_flight;
    let response = kit::modal(ui, "create-instance", &style, !busy, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text(ui, TextRole::Heading, &tr!("Create instance"));
        kit.text(
            ui,
            TextRole::Muted,
            &tr!("Choose a name, a Minecraft version and a modloader."),
        );

        let create_state = &mut state.create;
        kit.setting_row(ui, "create::name-row", &tr!("Name"), None, |kit, ui| {
            kit.text_field(
                ui,
                "create::name",
                &mut create_state.name,
                &tr!("My instance"),
                false,
                Sizing::Fixed(300.0),
            );
        });

        let versions: Vec<String> = create_state
            .catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .game_versions
                    .iter()
                    .map(|entry| entry.display_label())
                    .collect()
            })
            .unwrap_or_default();
        if versions.is_empty() {
            let message = match (&create_state.catalog_error, create_state.catalog_in_flight) {
                (Some(err), _) => tr!("Could not load Minecraft versions: {err}", err = err).into_owned(),
                (None, true) => tr!("Loading Minecraft versions…").into_owned(),
                (None, false) => tr!("No Minecraft versions are available.").into_owned(),
            };
            kit::label(
                ui,
                &message,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Muted)
                },
            );
            if create_state.catalog_error.is_some()
                && kit.button(ui, "create::retry", &tr!("Retry"), ButtonKind::Neutral, true)
            {
                refresh = true;
            }
        } else {
            kit.setting_row(
                ui,
                "create::version-row",
                &tr!("Minecraft version"),
                None,
                |kit, ui| {
                    if let Some(index) = kit.dropdown(
                        ui,
                        "create::version",
                        create_state.version,
                        &versions,
                        300.0,
                    ) {
                        create_state.version = index;
                        create_state.loader_version = 0;
                    }
                },
            );
        }
        let game_version = create_state
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.game_versions.get(create_state.version))
            .map(|entry| entry.id.clone())
            .unwrap_or_default();

        let loader_labels: Vec<String> = LOADERS
            .iter()
            .map(|loader| {
                let unavailable = create_state.catalog.as_ref().and_then(|catalog| {
                    catalog
                        .loader_support
                        .unavailable_reason(loader, &game_version)
                });
                match unavailable {
                    Some(_) if *loader != "Vanilla" => tr!("{loader} (not available)", loader = loader).into_owned(),
                    _ => (*loader).to_owned(),
                }
            })
            .collect();
        kit.setting_row(ui, "create::loader-row", &tr!("Modloader"), None, |kit, ui| {
            if let Some(index) = kit.dropdown(
                ui,
                "create::loader",
                create_state.loader,
                &loader_labels,
                300.0,
            ) {
                create_state.loader = index;
                create_state.loader_version = 0;
            }
        });

        let loader = LOADERS[create_state.loader];
        let loader_supported = loader == "Vanilla"
            || create_state.catalog.as_ref().is_none_or(|catalog| {
                catalog
                    .loader_support
                    .supports_loader(loader, &game_version)
            });
        let mut loader_versions = vec![LATEST_LOADER_VERSION.to_owned()];
        if loader != "Vanilla"
            && let Some(versions) = create_state.catalog.as_ref().and_then(|catalog| {
                catalog
                    .loader_versions
                    .versions_for_loader(loader, &game_version)
            })
        {
            loader_versions.extend(versions.iter().cloned());
        }
        if loader != "Vanilla" {
            kit.setting_row(
                ui,
                "create::loader-version-row",
                &tr!("Loader version"),
                None,
                |kit, ui| {
                    if let Some(index) = kit.dropdown(
                        ui,
                        "create::loader-version",
                        create_state.loader_version,
                        &loader_versions,
                        300.0,
                    ) {
                        create_state.loader_version = index;
                    }
                },
            );
        }
        if !loader_supported {
            kit.text(
                ui,
                TextRole::Danger,
                &tr!("{loader} does not support Minecraft {game_version}.", loader = loader, game_version = game_version),
            );
        }
        if let Some(error) = &create_state.error {
            kit::label(
                ui,
                error,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Danger)
                },
            );
        }
        if busy {
            kit.progress(
                ui,
                "create::progress",
                None,
                state.started.elapsed().as_secs_f64(),
            );
        }

        let _actions = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        if kit.button(ui, "create::cancel", &tr!("Cancel"), ButtonKind::Neutral, !busy) {
            close = true;
        }
        let can_create = !busy
            && !create_state.name.trim().is_empty()
            && !game_version.is_empty()
            && loader_supported;
        if kit.button(
            ui,
            "create::submit",
            &tr!("Create"),
            ButtonKind::Primary,
            can_create,
        ) {
            let loader_version = loader_versions
                .get(create_state.loader_version)
                .filter(|version| *version != LATEST_LOADER_VERSION)
                .cloned()
                .unwrap_or_default();
            create = Some(NewInstanceSpec {
                name: create_state.name.trim().to_owned(),
                description: None,
                thumbnail_path: None,
                modloader: loader.to_owned(),
                game_version,
                modloader_version: loader_version,
            });
        }
    });

    if refresh {
        state.create.catalog_error = None;
        state.create.request_catalog(filter, true);
    }
    if let Some(spec) = create {
        start_create(state, spec);
    }
    if (response.dismissed || kit.back_pressed()) || close {
        state.create.reset();
        kit.forget("create::");
        return false;
    }
    true
}

fn start_create(state: &mut AppState, spec: NewInstanceSpec) {
    if state.create.create_in_flight {
        return;
    }
    state.create.error = None;
    state.create.create_in_flight = true;
    let tx = state.create.create_results.sender();
    let mut store = state.instances.clone();
    let root = state.installations_root();
    launcher_runtime::spawn_blocking_detached(move || {
        let result = create_instance(&mut store, &root, spec)
            .map(|instance| (store, instance))
            .map_err(|err| err.to_string());
        let _ = tx.send(result);
    });
}
