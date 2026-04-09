use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::Duration,
};

use curseforge::{Client as CurseForgeClient, MINECRAFT_GAME_ID};
use egui::Ui;
use installation::{MinecraftVersionEntry, fetch_version_catalog};
use modrinth::Client as ModrinthClient;
use textui::TextUi;
use textui_egui::{apply_gamepad_scroll_to_registered_id, make_gamepad_scrollable, prelude::*};
use ui_foundation::{UiMetrics, responsive_columns, themed_text_input};

use crate::{
    app::tokio_runtime,
    assets,
    screens::AppScreen,
    ui::{
        components::{remote_tiled_image, settings_widgets},
        style,
    },
};

const DISCOVER_PROVIDER_LIMIT: u32 = 36;
const DISCOVER_CARD_GAP: f32 = 12.0;
const VERSION_CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(75);
const DETAIL_VERSIONS_FETCH_TIMEOUT: Duration = Duration::from_secs(45);
const DISCOVER_SEARCH_CACHE_MAX_ENTRIES: usize = 10;

#[path = "discover_detail.rs"]
mod discover_detail;

use self::discover_detail::{
    DiscoverVersionEntry, DiscoverVersionsResult, format_compact_number, format_short_date,
    open_detail_page, render_discover_detail_content,
};

#[derive(Clone, Copy, Debug)]
struct DiscoverUiMetrics {
    card_min_width: f32,
    card_image_height: f32,
    estimated_card_base_height: f32,
}

#[derive(Clone, Copy, Debug)]
struct DiscoverMasonryItem {
    entry_index: usize,
    offset_y: f32,
    height: f32,
}

#[derive(Clone, Debug)]
struct DiscoverMasonryColumn {
    items: Vec<DiscoverMasonryItem>,
    total_height: f32,
}

#[derive(Clone, Debug)]
struct DiscoverMasonryLayout {
    columns: Vec<DiscoverMasonryColumn>,
    content_width: f32,
    column_width: f32,
}

#[derive(Clone, Debug)]
struct CachedDiscoverMasonryLayout {
    width_bucket: u32,
    entries_fingerprint: u64,
    height_cache_revision: u64,
    layout: DiscoverMasonryLayout,
}

impl DiscoverUiMetrics {
    fn from_ui(ui: &Ui) -> Self {
        let metrics = UiMetrics::from_ui(ui, 860.0);
        Self {
            card_min_width: metrics.scaled_width(0.18, 220.0, 300.0),
            card_image_height: metrics.scaled_height(0.15, 112.0, 160.0),
            estimated_card_base_height: metrics.scaled_height(0.24, 188.0, 236.0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiscoverState {
    query_input: String,
    search_tags: Vec<String>,
    game_version_filter: String,
    provider_filter: DiscoverProviderFilter,
    loader_filter: DiscoverLoaderFilter,
    sort_mode: DiscoverSortMode,
    page: u32,
    search_in_flight: bool,
    search_request_serial: u64,
    initial_search_requested: bool,
    status_message: Option<String>,
    warnings: Vec<String>,
    entries: Vec<DiscoverEntry>,
    tile_height_cache: HashMap<(String, u32), f32>,
    tile_height_cache_revision: u64,
    has_more_results: bool,
    masonry_layout_cache: Option<CachedDiscoverMasonryLayout>,
    cached_snapshots: HashMap<DiscoverSearchRequest, DiscoverSearchSnapshot>,
    available_game_versions: Vec<MinecraftVersionEntry>,
    version_catalog_error: Option<String>,
    version_catalog_in_flight: bool,
    version_catalog_tx: Option<mpsc::Sender<Result<Vec<MinecraftVersionEntry>, String>>>,
    version_catalog_rx:
        Option<Arc<Mutex<mpsc::Receiver<Result<Vec<MinecraftVersionEntry>, String>>>>>,
    search_results_tx: Option<mpsc::Sender<DiscoverSearchResult>>,
    search_results_rx: Option<Arc<Mutex<mpsc::Receiver<DiscoverSearchResult>>>>,
    detail_entry: Option<DiscoverEntry>,
    detail_selected_source: Option<DiscoverSource>,
    detail_versions: Vec<DiscoverVersionEntry>,
    detail_versions_error: Option<String>,
    detail_versions_in_flight: bool,
    detail_version_request_serial: u64,
    detail_version_results_tx: Option<mpsc::Sender<DiscoverVersionsResult>>,
    detail_version_results_rx: Option<Arc<Mutex<mpsc::Receiver<DiscoverVersionsResult>>>>,
    install_in_flight: bool,
    install_message: Option<String>,
    install_completed_steps: usize,
    install_total_steps: usize,
    install_error: Option<String>,
}

impl Default for DiscoverState {
    fn default() -> Self {
        Self {
            query_input: String::new(),
            search_tags: Vec::new(),
            game_version_filter: String::new(),
            provider_filter: DiscoverProviderFilter::default(),
            loader_filter: DiscoverLoaderFilter::default(),
            sort_mode: DiscoverSortMode::default(),
            page: 1,
            search_in_flight: false,
            search_request_serial: 0,
            initial_search_requested: false,
            status_message: None,
            warnings: Vec::new(),
            entries: Vec::new(),
            tile_height_cache: HashMap::new(),
            tile_height_cache_revision: 0,
            has_more_results: true,
            masonry_layout_cache: None,
            cached_snapshots: HashMap::new(),
            available_game_versions: Vec::new(),
            version_catalog_error: None,
            version_catalog_in_flight: false,
            version_catalog_tx: None,
            version_catalog_rx: None,
            search_results_tx: None,
            search_results_rx: None,
            detail_entry: None,
            detail_selected_source: None,
            detail_versions: Vec::new(),
            detail_versions_error: None,
            detail_versions_in_flight: false,
            detail_version_request_serial: 0,
            detail_version_results_tx: None,
            detail_version_results_rx: None,
            install_in_flight: false,
            install_message: None,
            install_completed_steps: 0,
            install_total_steps: 0,
            install_error: None,
        }
    }
}

impl DiscoverState {
    pub fn begin_install(&mut self, message: impl Into<String>) {
        self.install_in_flight = true;
        self.install_error = None;
        self.install_message = Some(message.into());
        self.install_completed_steps = 0;
        self.install_total_steps = 0;
    }

    pub fn apply_install_progress(
        &mut self,
        message: impl Into<String>,
        completed_steps: usize,
        total_steps: usize,
    ) {
        self.install_in_flight = true;
        self.install_error = None;
        self.install_message = Some(message.into());
        self.install_completed_steps = completed_steps;
        self.install_total_steps = total_steps;
    }

    pub fn finish_install(&mut self, result: Result<String, String>) {
        self.install_in_flight = false;
        match result {
            Ok(message) => {
                self.install_error = None;
                self.install_message = Some(message);
            }
            Err(error) => {
                self.install_error = Some(error);
            }
        }
    }

    pub fn purge_inactive_state(&mut self) {
        self.search_in_flight = false;
        self.initial_search_requested = false;
        self.status_message = None;
        self.warnings.clear();
        self.entries.clear();
        self.tile_height_cache.clear();
        self.tile_height_cache_revision = self.tile_height_cache_revision.saturating_add(1);
        self.masonry_layout_cache = None;
        self.cached_snapshots.clear();
        self.has_more_results = true;
        self.search_results_tx = None;
        self.search_results_rx = None;
        self.detail_entry = None;
        self.detail_selected_source = None;
        self.detail_versions.clear();
        self.detail_versions_error = None;
        self.detail_versions_in_flight = false;
        self.detail_version_request_serial = 0;
        self.detail_version_results_tx = None;
        self.detail_version_results_rx = None;
        self.install_in_flight = false;
        self.install_message = None;
        self.install_completed_steps = 0;
        self.install_total_steps = 0;
        self.install_error = None;
    }
}

#[derive(Debug, Clone, Default)]
pub struct DiscoverOutput {
    pub requested_screen: Option<AppScreen>,
    pub install_requested: Option<DiscoverInstallRequest>,
}

#[derive(Debug, Clone)]
pub struct DiscoverInstallRequest {
    pub instance_name: String,
    pub project_summary: Option<String>,
    pub icon_url: Option<String>,
    pub version_name: String,
    pub source: DiscoverInstallSource,
}

#[derive(Debug, Clone)]
pub enum DiscoverInstallSource {
    Modrinth {
        project_id: String,
        version_id: String,
        file_url: String,
        file_name: String,
    },
    CurseForge {
        project_id: u64,
        file_id: u64,
        file_name: String,
        download_url: Option<String>,
        manual_download_path: Option<std::path::PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum DiscoverProviderFilter {
    #[default]
    All,
    Modrinth,
    CurseForge,
}

impl DiscoverProviderFilter {
    const ALL: [Self; 3] = [Self::All, Self::Modrinth, Self::CurseForge];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All Sources",
            Self::Modrinth => "Modrinth",
            Self::CurseForge => "CurseForge",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum DiscoverLoaderFilter {
    #[default]
    Any,
    Fabric,
    Forge,
    NeoForge,
    Quilt,
}

impl DiscoverLoaderFilter {
    const ALL: [Self; 5] = [
        Self::Any,
        Self::Fabric,
        Self::Forge,
        Self::NeoForge,
        Self::Quilt,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Any => "Any Loader",
            Self::Fabric => "Fabric",
            Self::Forge => "Forge",
            Self::NeoForge => "NeoForge",
            Self::Quilt => "Quilt",
        }
    }

    fn modrinth_slug(self) -> Option<&'static str> {
        match self {
            Self::Any => None,
            Self::Fabric => Some("fabric"),
            Self::Forge => Some("forge"),
            Self::NeoForge => Some("neoforge"),
            Self::Quilt => Some("quilt"),
        }
    }

    fn curseforge_mod_loader_type(self) -> Option<u32> {
        match self {
            Self::Any => None,
            Self::Forge => Some(1),
            Self::Fabric => Some(4),
            Self::Quilt => Some(5),
            Self::NeoForge => Some(6),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum DiscoverSortMode {
    #[default]
    Popularity,
    Relevance,
    LastUpdated,
}

impl DiscoverSortMode {
    const ALL: [Self; 3] = [Self::Popularity, Self::Relevance, Self::LastUpdated];

    fn label(self) -> &'static str {
        match self {
            Self::Popularity => "Popularity",
            Self::Relevance => "Relevance",
            Self::LastUpdated => "Last Updated",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DiscoverSource {
    Modrinth,
    CurseForge,
}

impl DiscoverSource {
    fn label(self) -> &'static str {
        match self {
            Self::Modrinth => "Modrinth",
            Self::CurseForge => "CurseForge",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DiscoverSearchRequest {
    query: String,
    tags: Vec<String>,
    game_version: Option<String>,
    provider_filter: DiscoverProviderFilter,
    loader_filter: DiscoverLoaderFilter,
    sort_mode: DiscoverSortMode,
    page: u32,
}

#[derive(Clone, Debug)]
struct DiscoverSearchResult {
    request_serial: u64,
    request: DiscoverSearchRequest,
    outcome: Result<DiscoverSearchSnapshot, String>,
}

#[derive(Clone, Debug, Default)]
struct DiscoverSearchSnapshot {
    entries: Vec<DiscoverEntry>,
    warnings: Vec<String>,
    has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchMode {
    Replace,
    Append,
}

#[derive(Clone, Debug)]
struct DiscoverEntry {
    dedupe_key: String,
    name: String,
    summary: String,
    author: Option<String>,
    icon_url: Option<String>,
    primary_url: Option<String>,
    sources: Vec<DiscoverSource>,
    provider_refs: Vec<DiscoverProviderRef>,
    popularity_score: Option<u64>,
    updated_at: Option<String>,
    relevance_rank: u32,
}

#[derive(Clone, Debug)]
struct DiscoverProviderEntry {
    project_ref: DiscoverProjectRef,
    name: String,
    summary: String,
    author: Option<String>,
    icon_url: Option<String>,
    primary_url: Option<String>,
    source: DiscoverSource,
    popularity_score: Option<u64>,
    updated_at: Option<String>,
    relevance_rank: u32,
}

#[derive(Clone, Debug)]
struct DiscoverProviderRef {
    source: DiscoverSource,
    project_ref: DiscoverProjectRef,
    primary_url: Option<String>,
}

#[derive(Clone, Debug)]
enum DiscoverProjectRef {
    Modrinth { project_id: String },
    CurseForge { project_id: u64 },
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut DiscoverState,
    detail_mode: bool,
) -> DiscoverOutput {
    let full_width = ui.available_width().max(1.0);
    let full_height = ui.available_height().max(1.0);
    let mut output = DiscoverOutput::default();
    ui.horizontal(|ui| {
        ui.add_space(style::SPACE_XS);
        ui.allocate_ui_with_layout(
            egui::vec2((full_width - style::SPACE_XS * 2.0).max(1.0), full_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                output = if detail_mode {
                    render_discover_detail_content(ui, text_ui, state)
                } else {
                    render_discover_browse_content(ui, text_ui, state)
                };
            },
        );
        ui.add_space(style::SPACE_XS);
    });
    output
}

fn render_discover_browse_content(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut DiscoverState,
) -> DiscoverOutput {
    let mut output = DiscoverOutput::default();
    poll_version_catalog(state);
    request_version_catalog(state);
    poll_search_results(state);
    if !state.initial_search_requested {
        state.initial_search_requested = true;
        request_search(state, false, SearchMode::Replace);
    }
    if state.search_in_flight {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }

    let muted_style = style::muted(ui);
    let warning_style = style::warning_text(ui);
    let mut filter_controls_have_focus = false;

    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_MD))
        .inner_margin(egui::Margin::same(style::SPACE_MD as i8))
        .show(ui, |ui| {
            let old_provider_filter = state.provider_filter;
            let old_loader_filter = state.loader_filter;
            let old_sort_mode = state.sort_mode;
            let old_game_version_filter = state.game_version_filter.clone();
            let search_response = themed_text_input(
                text_ui,
                ui,
                "discover_search_input",
                &mut state.query_input,
                InputOptions {
                    desired_width: Some(ui.available_width()),
                    placeholder_text: Some("Search modpacks and press Enter".to_owned()),
                    ..InputOptions::default()
                },
            );
            let mut search_submitted = false;
            let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));
            let submit_pressed =
                enter_pressed && (search_response.has_focus() || search_response.lost_focus());
            if submit_pressed {
                if ui.input(|input| input.modifiers.shift) {
                    if add_search_tag(&mut state.search_tags, state.query_input.as_str()) {
                        state.query_input.clear();
                        search_submitted = true;
                    }
                } else {
                    search_submitted = true;
                }
            }
            if !state.search_tags.is_empty() && render_search_tag_chips(ui, &mut state.search_tags)
            {
                search_submitted = true;
            }
            ui.add_space(style::SPACE_SM);

            let (filter_columns, dropdown_width) =
                responsive_columns(ui.available_width(), 180.0, DISCOVER_CARD_GAP, 4);
            filter_controls_have_focus = search_response.has_focus();
            let mut render_filter_row = |ui: &mut Ui, row: usize| {
                ui.spacing_mut().item_spacing.x = DISCOVER_CARD_GAP;
                let provider_index = row * filter_columns;
                if provider_index == 0 {
                    let provider_options =
                        DiscoverProviderFilter::ALL.map(DiscoverProviderFilter::label);
                    let mut provider_index = DiscoverProviderFilter::ALL
                        .iter()
                        .position(|provider| *provider == state.provider_filter)
                        .unwrap_or(0);
                    let response = sized_dropdown_picker(
                        ui,
                        text_ui,
                        "discover_provider_filter",
                        dropdown_width,
                        &mut provider_index,
                        &provider_options,
                    );
                    state.provider_filter = DiscoverProviderFilter::ALL[provider_index];
                    filter_controls_have_focus |= response.has_focus();
                }
                if provider_index <= 1 && 1 < (row + 1) * filter_columns {
                    let loader_options = DiscoverLoaderFilter::ALL.map(DiscoverLoaderFilter::label);
                    let mut loader_index = DiscoverLoaderFilter::ALL
                        .iter()
                        .position(|loader| *loader == state.loader_filter)
                        .unwrap_or(0);
                    let response = sized_dropdown_picker(
                        ui,
                        text_ui,
                        "discover_loader_filter",
                        dropdown_width,
                        &mut loader_index,
                        &loader_options,
                    );
                    state.loader_filter = DiscoverLoaderFilter::ALL[loader_index];
                    filter_controls_have_focus |= response.has_focus();
                }
                if provider_index <= 2 && 2 < (row + 1) * filter_columns {
                    let sort_options = DiscoverSortMode::ALL.map(DiscoverSortMode::label);
                    let mut sort_index = DiscoverSortMode::ALL
                        .iter()
                        .position(|sort_mode| *sort_mode == state.sort_mode)
                        .unwrap_or(0);
                    let response = sized_dropdown_picker(
                        ui,
                        text_ui,
                        "discover_sort_mode",
                        dropdown_width,
                        &mut sort_index,
                        &sort_options,
                    );
                    state.sort_mode = DiscoverSortMode::ALL[sort_index];
                    filter_controls_have_focus |= response.has_focus();
                }
                if provider_index <= 3 && 3 < (row + 1) * filter_columns {
                    let mut game_version_options =
                        Vec::<String>::with_capacity(state.available_game_versions.len() + 1);
                    game_version_options.push("Any version".to_owned());
                    for version in &state.available_game_versions {
                        game_version_options.push(version.display_label());
                    }
                    let game_version_option_refs = game_version_options
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>();
                    let mut game_version_index = 0_usize;
                    if !state.game_version_filter.trim().is_empty()
                        && let Some(found_index) = state
                            .available_game_versions
                            .iter()
                            .position(|version| version.id == state.game_version_filter)
                    {
                        game_version_index = found_index + 1;
                    }
                    let response = sized_dropdown_picker(
                        ui,
                        text_ui,
                        "discover_game_version",
                        dropdown_width,
                        &mut game_version_index,
                        &game_version_option_refs,
                    );
                    state.game_version_filter = if game_version_index == 0 {
                        String::new()
                    } else {
                        state.available_game_versions[game_version_index - 1]
                            .id
                            .clone()
                    };
                    filter_controls_have_focus |= response.has_focus();
                }
            };
            let filter_rows = 4_usize.div_ceil(filter_columns);
            ui.vertical(|ui| {
                for row in 0..filter_rows {
                    ui.horizontal(|ui| render_filter_row(ui, row));
                    if row + 1 < filter_rows {
                        ui.add_space(DISCOVER_CARD_GAP);
                    }
                }
            });
            let filters_changed = state.provider_filter != old_provider_filter
                || state.loader_filter != old_loader_filter
                || state.sort_mode != old_sort_mode
                || state.game_version_filter != old_game_version_filter;
            if search_submitted || filters_changed {
                request_search(state, true, SearchMode::Replace);
            }
        });

    ui.add_space(style::SPACE_MD);
    if let Some(status) = state.status_message.as_deref() {
        let _ = text_ui.label(ui, "discover_status", status, &muted_style);
    }
    for warning in &state.warnings {
        let _ = text_ui.label(ui, ("discover_warning", warning), warning, &warning_style);
    }

    if state.search_in_flight {
        ui.add_space(style::SPACE_SM);
        ui.horizontal(|ui| {
            ui.spinner();
            let _ = text_ui.label(
                ui,
                "discover_search_in_flight",
                "Loading modpacks...",
                &muted_style,
            );
        });
    }

    ui.add_space(style::SPACE_MD);
    let mut should_load_more = false;
    let results_height = ui.available_height().max(1.0);
    let scroll_output = egui::ScrollArea::vertical()
        .id_salt("discover_results_scroll")
        .auto_shrink([false, false])
        .max_height(results_height)
        .show_viewport(ui, |ui, viewport| {
            if state.entries.is_empty() && !state.search_in_flight {
                let _ = text_ui.label(
                    ui,
                    "discover_empty",
                    "No modpacks matched the current search and filters.",
                    &muted_style,
                );
                return;
            }
            if let Some(entry) = render_masonry_tiles(
                ui,
                text_ui,
                state.entries.as_slice(),
                &mut state.tile_height_cache,
                &mut state.tile_height_cache_revision,
                &mut state.masonry_layout_cache,
                viewport,
            ) {
                open_detail_page(state, &entry);
                output.requested_screen = Some(AppScreen::DiscoverDetail);
            }
            let content_bottom = ui.min_rect().bottom();
            should_load_more = state.has_more_results
                && !state.search_in_flight
                && viewport.bottom() >= content_bottom - 320.0;
        });
    make_gamepad_scrollable(ui.ctx(), &scroll_output);
    if filter_controls_have_focus
        && apply_gamepad_scroll_to_registered_id(
            ui.ctx(),
            scroll_output.id,
            egui::vec2(0.0, 100_000.0),
        )
    {
        ui.ctx().request_repaint();
    }

    if should_load_more {
        request_search(state, false, SearchMode::Append);
    }
    output
}

fn render_masonry_tiles(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    entries: &[DiscoverEntry],
    tile_height_cache: &mut HashMap<(String, u32), f32>,
    tile_height_cache_revision: &mut u64,
    masonry_layout_cache: &mut Option<CachedDiscoverMasonryLayout>,
    viewport: egui::Rect,
) -> Option<DiscoverEntry> {
    let metrics = DiscoverUiMetrics::from_ui(ui);
    let layout = build_discover_masonry_layout(
        ui,
        entries,
        tile_height_cache,
        *tile_height_cache_revision,
        masonry_layout_cache,
        metrics,
    );
    let overscan = metrics.card_image_height.max(220.0);
    let visible_top = viewport.top() - overscan;
    let visible_bottom = viewport.bottom() + overscan;
    let mut opened_entry = None;
    ui.allocate_ui_with_layout(
        egui::vec2(layout.content_width, 0.0),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.x = DISCOVER_CARD_GAP;
            for column in &layout.columns {
                ui.allocate_ui_with_layout(
                    egui::vec2(layout.column_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(layout.column_width);
                        let mut visible_started = false;
                        let mut skipped_height = 0.0;
                        let mut rendered_bottom = 0.0;

                        for item in &column.items {
                            let item_bottom = item.offset_y + item.height;
                            if item_bottom < visible_top {
                                skipped_height = item_bottom + DISCOVER_CARD_GAP;
                                continue;
                            }
                            if item.offset_y > visible_bottom {
                                break;
                            }
                            if !visible_started {
                                if skipped_height > 0.0 {
                                    ui.add_space(skipped_height);
                                }
                                visible_started = true;
                            }
                            let tile_result = render_discover_tile(
                                ui,
                                text_ui,
                                &entries[item.entry_index],
                                metrics,
                            );
                            let height_cache_key = discover_tile_height_cache_key(
                                entries[item.entry_index].dedupe_key.as_str(),
                                layout.column_width,
                            );
                            let previous = tile_height_cache
                                .insert(height_cache_key, tile_result.measured_height);
                            if previous.is_none_or(|height| {
                                (height - tile_result.measured_height).abs() > 0.5
                            }) {
                                *tile_height_cache_revision =
                                    tile_height_cache_revision.saturating_add(1);
                            }
                            if tile_result.clicked {
                                opened_entry = Some(entries[item.entry_index].clone());
                            }
                            rendered_bottom = item_bottom + DISCOVER_CARD_GAP;
                            ui.add_space(DISCOVER_CARD_GAP);
                        }

                        if !visible_started {
                            if column.total_height > 0.0 {
                                ui.add_space(column.total_height);
                            }
                        } else {
                            let trailing_height = (column.total_height - rendered_bottom).max(0.0);
                            if trailing_height > 0.0 {
                                ui.add_space(trailing_height);
                            }
                        }
                    },
                );
            }
        },
    );
    opened_entry
}

fn build_discover_masonry_layout(
    ui: &Ui,
    entries: &[DiscoverEntry],
    tile_height_cache: &HashMap<(String, u32), f32>,
    tile_height_cache_revision: u64,
    masonry_layout_cache: &mut Option<CachedDiscoverMasonryLayout>,
    metrics: DiscoverUiMetrics,
) -> DiscoverMasonryLayout {
    let content_width = ui.available_width().max(metrics.card_min_width);
    let (column_count, column_width) =
        responsive_columns(content_width, metrics.card_min_width, DISCOVER_CARD_GAP, 4);
    let width_bucket = (column_width / 16.0).round().max(1.0) as u32;
    let entries_fingerprint = discover_entries_fingerprint(entries, width_bucket);
    if let Some(cached) = masonry_layout_cache.as_ref()
        && cached.width_bucket == width_bucket
        && cached.entries_fingerprint == entries_fingerprint
        && cached.height_cache_revision == tile_height_cache_revision
    {
        return cached.layout.clone();
    }
    let mut columns = (0..column_count)
        .map(|_| DiscoverMasonryColumn {
            items: Vec::new(),
            total_height: 0.0,
        })
        .collect::<Vec<_>>();

    for (index, entry) in entries.iter().enumerate() {
        let estimated_height = tile_height_cache
            .get(&discover_tile_height_cache_key(
                entry.dedupe_key.as_str(),
                column_width,
            ))
            .copied()
            .unwrap_or_else(|| discover_tile_estimated_height(entry, column_width, metrics));
        let target_column = columns
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| left.total_height.total_cmp(&right.total_height))
            .map(|(index, _)| index)
            .unwrap_or(0);
        let offset_y = columns[target_column].total_height;
        columns[target_column].items.push(DiscoverMasonryItem {
            entry_index: index,
            offset_y,
            height: estimated_height,
        });
        columns[target_column].total_height += estimated_height + DISCOVER_CARD_GAP;
    }

    let layout = DiscoverMasonryLayout {
        columns,
        content_width,
        column_width,
    };
    *masonry_layout_cache = Some(CachedDiscoverMasonryLayout {
        width_bucket,
        entries_fingerprint,
        height_cache_revision: tile_height_cache_revision,
        layout: layout.clone(),
    });
    layout
}

fn discover_tile_estimated_height(
    entry: &DiscoverEntry,
    column_width: f32,
    metrics: DiscoverUiMetrics,
) -> f32 {
    let summary_lines = (entry.summary.len() as f32 / (column_width / 5.8).max(32.0))
        .ceil()
        .clamp(2.0, 6.0);
    metrics.estimated_card_base_height + (summary_lines * 18.0)
}

fn discover_tile_height_cache_key(dedupe_key: &str, column_width: f32) -> (String, u32) {
    (
        dedupe_key.to_owned(),
        (column_width / 16.0).round().max(1.0) as u32,
    )
}

fn discover_entries_fingerprint(entries: &[DiscoverEntry], width_bucket: u32) -> u64 {
    let mut hasher = DefaultHasher::new();
    width_bucket.hash(&mut hasher);
    entries.len().hash(&mut hasher);
    for entry in entries {
        entry.dedupe_key.hash(&mut hasher);
    }
    hasher.finish()
}

fn render_discover_tile(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    entry: &DiscoverEntry,
    metrics: DiscoverUiMetrics,
) -> DiscoverTileRenderResult {
    let id = ui.make_persistent_id(("discover_tile_click", entry.dedupe_key.as_str()));

    // Read last frame's hover/focus state so the frame is styled before it renders.
    let (was_hovered, was_focused): (bool, bool) =
        ui.ctx().data(|d| d.get_temp(id).unwrap_or_default());

    let base_stroke_width = ui.visuals().widgets.noninteractive.bg_stroke.width;
    let sel_color = ui.visuals().selection.bg_fill;
    let (fill, stroke) = if was_focused {
        (
            sel_color.gamma_multiply(0.15),
            egui::Stroke::new(base_stroke_width, sel_color),
        )
    } else if was_hovered {
        (
            ui.visuals().window_fill,
            egui::Stroke::new(
                base_stroke_width,
                ui.visuals().widgets.hovered.bg_stroke.color,
            ),
        )
    } else {
        (
            ui.visuals().window_fill,
            ui.visuals().widgets.noninteractive.bg_stroke,
        )
    };

    let heading_style = LabelOptions {
        font_size: 20.0,
        line_height: 24.0,
        wrap: true,
        ..style::stat_label(ui)
    };
    let body_style = style::body(ui);
    let muted_style = style::muted(ui);
    let badge_fill = ui.visuals().widgets.inactive.weak_bg_fill;
    let badge_stroke = ui.visuals().widgets.inactive.bg_stroke;

    let response = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_MD))
        .inner_margin(egui::Margin::same(style::SPACE_MD as i8))
        .show(ui, |ui| {
            let page_link_clicked = false;
            if let Some(icon_url) = entry.icon_url.as_deref() {
                remote_tiled_image::show(
                    ui,
                    icon_url,
                    egui::vec2(ui.available_width(), metrics.card_image_height),
                    ("discover_tile_image", entry.dedupe_key.as_str()),
                    assets::DISCOVER_SVG,
                );
            } else {
                ui.add(
                    egui::Image::from_bytes(
                        format!("bytes://discover/placeholder/{}", entry.dedupe_key),
                        assets::DISCOVER_SVG.to_vec(),
                    )
                    .fit_to_exact_size(egui::vec2(ui.available_width(), metrics.card_image_height)),
                );
            }

            ui.add_space(style::SPACE_SM);
            let _ = text_ui.label(
                ui,
                ("discover_tile_name", entry.dedupe_key.as_str()),
                entry.name.as_str(),
                &heading_style,
            );
            if let Some(author) = entry
                .author
                .as_deref()
                .filter(|author| !author.trim().is_empty())
            {
                let _ = text_ui.label(
                    ui,
                    ("discover_tile_author", entry.dedupe_key.as_str()),
                    &format!("by {author}"),
                    &muted_style,
                );
            }
            ui.add_space(style::SPACE_XS);
            let _ = text_ui.label(
                ui,
                ("discover_tile_summary", entry.dedupe_key.as_str()),
                entry.summary.as_str(),
                &body_style,
            );

            ui.add_space(style::SPACE_SM);
            ui.horizontal_wrapped(|ui| {
                for source in &entry.sources {
                    egui::Frame::new()
                        .fill(badge_fill)
                        .stroke(badge_stroke)
                        .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_SM))
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            let _ = text_ui.label(
                                ui,
                                (
                                    "discover_tile_source",
                                    entry.dedupe_key.as_str(),
                                    source.label(),
                                ),
                                source.label(),
                                &LabelOptions {
                                    wrap: false,
                                    ..style::body(ui)
                                },
                            );
                        });
                }
            });

            ui.add_space(style::SPACE_SM);
            if let Some(downloads) = entry.popularity_score {
                let _ = text_ui.label(
                    ui,
                    ("discover_tile_downloads", entry.dedupe_key.as_str()),
                    &format!("Downloads: {}", format_compact_number(downloads)),
                    &muted_style,
                );
            }
            if let Some(updated_at) = entry.updated_at.as_deref() {
                let _ = text_ui.label(
                    ui,
                    ("discover_tile_updated", entry.dedupe_key.as_str()),
                    &format!("Updated: {}", format_short_date(updated_at)),
                    &muted_style,
                );
            }
            page_link_clicked
        });
    let interaction = ui.interact(response.response.rect, id, egui::Sense::click());

    // Persist state for the next frame so the frame fill/stroke can be set
    // correctly before the frame renders (egui renders before interact returns).
    let now_hovered = interaction.hovered();
    let now_focused = interaction.has_focus();
    let state_changed = now_hovered != was_hovered || now_focused != was_focused;
    if state_changed {
        ui.ctx()
            .data_mut(|d| d.insert_temp(id, (now_hovered, now_focused)));
        ui.ctx().request_repaint();
    }

    DiscoverTileRenderResult {
        clicked: interaction.clicked() && !response.inner,
        measured_height: response.response.rect.height(),
    }
}

#[derive(Clone, Copy, Debug)]
struct DiscoverTileRenderResult {
    clicked: bool,
    measured_height: f32,
}

fn ensure_search_channel(state: &mut DiscoverState) {
    if state.search_results_tx.is_some() && state.search_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<DiscoverSearchResult>();
    state.search_results_tx = Some(tx);
    state.search_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_search(state: &mut DiscoverState, show_cached_status: bool, mode: SearchMode) {
    if state.search_in_flight {
        return;
    }
    ensure_search_channel(state);
    let request = current_request(state, mode);
    if let Some(snapshot) = state.cached_snapshots.get(&request).cloned() {
        apply_search_snapshot(state, &request, snapshot, mode);
        state.search_in_flight = false;
        if show_cached_status {
            state.status_message = Some("Loaded cached discover results.".to_owned());
        }
        return;
    }

    let Some(tx) = state.search_results_tx.as_ref().cloned() else {
        return;
    };
    state.search_request_serial = state.search_request_serial.saturating_add(1);
    let request_serial = state.search_request_serial;
    state.search_in_flight = true;
    if mode == SearchMode::Replace {
        state.page = 1;
        state.has_more_results = true;
    }
    state.status_message = Some(format!(
        "Searching {} for modpacks...",
        request.provider_filter.label()
    ));
    let request_for_task = request.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let outcome = Ok(perform_search(&request_for_task));
        if let Err(err) = tx.send(DiscoverSearchResult {
            request_serial,
            request,
            outcome,
        }) {
            tracing::error!(
                target: "vertexlauncher/discover",
                request_serial,
                error = %err,
                "Failed to deliver discover search result."
            );
        }
    });
}

fn trim_discover_search_cache(state: &mut DiscoverState) {
    if state.cached_snapshots.len() <= DISCOVER_SEARCH_CACHE_MAX_ENTRIES {
        return;
    }
    let active_request = current_request(state, SearchMode::Replace);
    state.cached_snapshots.retain(|request, _| {
        request == &active_request || request.page > state.page.saturating_sub(2)
    });
    if state.cached_snapshots.len() <= DISCOVER_SEARCH_CACHE_MAX_ENTRIES {
        return;
    }
    let mut requests = state.cached_snapshots.keys().cloned().collect::<Vec<_>>();
    requests.sort_by_key(|request| request.page);
    for request in requests {
        if state.cached_snapshots.len() <= DISCOVER_SEARCH_CACHE_MAX_ENTRIES {
            break;
        }
        if request != active_request {
            state.cached_snapshots.remove(&request);
        }
    }
}

fn poll_search_results(state: &mut DiscoverState) {
    let Some(rx) = state.search_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/discover",
            request_serial = state.search_request_serial,
            "Discover search receiver mutex was poisoned."
        );
        return;
    };
    loop {
        match receiver.try_recv() {
            Ok(result) => {
                if result.request_serial != state.search_request_serial {
                    tracing::debug!(
                        target: "vertexlauncher/discover",
                        request_serial = result.request_serial,
                        active_request_serial = state.search_request_serial,
                        "Ignoring stale discover search result."
                    );
                    continue;
                }
                state.search_in_flight = false;
                match result.outcome {
                    Ok(snapshot) => {
                        let mode = if result.request.page <= 1 {
                            SearchMode::Replace
                        } else {
                            SearchMode::Append
                        };
                        apply_search_snapshot(state, &result.request, snapshot.clone(), mode);
                        state.cached_snapshots.insert(result.request, snapshot);
                        trim_discover_search_cache(state);
                        state.status_message =
                            Some(format!("Showing {} modpacks.", state.entries.len()));
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "vertexlauncher/discover",
                            request_serial = result.request_serial,
                            error = %error,
                            "Discover search failed."
                        );
                        state.status_message = Some(format!("Discover search failed: {error}"));
                        state.entries.clear();
                        state.warnings.clear();
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/discover",
                    request_serial = state.search_request_serial,
                    "Discover search worker disconnected unexpectedly."
                );
                state.search_in_flight = false;
                state.status_message =
                    Some("Discover search worker stopped unexpectedly.".to_owned());
                break;
            }
        }
    }
}

fn current_request(state: &DiscoverState, mode: SearchMode) -> DiscoverSearchRequest {
    let combined_query =
        compose_search_query(state.query_input.as_str(), state.search_tags.as_slice());
    DiscoverSearchRequest {
        query: combined_query,
        tags: state.search_tags.clone(),
        game_version: non_empty(state.game_version_filter.as_str()),
        provider_filter: state.provider_filter,
        loader_filter: state.loader_filter,
        sort_mode: state.sort_mode,
        page: match mode {
            SearchMode::Replace => 1,
            SearchMode::Append => state.page.saturating_add(1).max(1),
        },
    }
}

fn perform_search(request: &DiscoverSearchRequest) -> DiscoverSearchSnapshot {
    let modrinth = ModrinthClient::default();
    let curseforge = CurseForgeClient::from_env();
    let mut warnings = Vec::new();
    let offset = request
        .page
        .saturating_sub(1)
        .saturating_mul(DISCOVER_PROVIDER_LIMIT);
    let mut provider_entries = Vec::new();
    let mut provider_result_count = 0usize;

    if matches!(
        request.provider_filter,
        DiscoverProviderFilter::All | DiscoverProviderFilter::Modrinth
    ) {
        let modrinth_sort_index = match request.sort_mode {
            DiscoverSortMode::Popularity => Some("downloads"),
            DiscoverSortMode::LastUpdated => Some("updated"),
            DiscoverSortMode::Relevance => None,
        };
        match modrinth.search_projects_with_filters(
            request.query.as_str(),
            DISCOVER_PROVIDER_LIMIT,
            offset,
            Some("modpack"),
            request.game_version.as_deref(),
            request.loader_filter.modrinth_slug(),
            modrinth_sort_index,
        ) {
            Ok(entries) => {
                provider_result_count += entries.len();
                provider_entries.extend(entries.into_iter().enumerate().map(|(index, entry)| {
                    DiscoverProviderEntry {
                        project_ref: DiscoverProjectRef::Modrinth {
                            project_id: entry.project_id,
                        },
                        name: entry.title,
                        summary: entry.description,
                        author: entry.author,
                        icon_url: entry.icon_url,
                        primary_url: Some(entry.project_url),
                        source: DiscoverSource::Modrinth,
                        popularity_score: Some(entry.downloads),
                        updated_at: entry.date_modified,
                        relevance_rank: index as u32,
                    }
                }));
            }
            Err(error) => warnings.push(format!("Modrinth search failed: {error}")),
        }
    }

    if matches!(
        request.provider_filter,
        DiscoverProviderFilter::All | DiscoverProviderFilter::CurseForge
    ) {
        match curseforge {
            Some(client) => {
                let class_id = resolve_curseforge_modpack_class_id_cached(&client, &mut warnings);
                if let Some(class_id) = class_id {
                    // CurseForge sortField: 6 = TotalDownloads, 3 = LastUpdated
                    let curseforge_sort_field = match request.sort_mode {
                        DiscoverSortMode::Popularity => Some(6),
                        DiscoverSortMode::LastUpdated => Some(3),
                        DiscoverSortMode::Relevance => None,
                    };
                    match client.search_projects_with_filters(
                        MINECRAFT_GAME_ID,
                        request.query.as_str(),
                        offset,
                        DISCOVER_PROVIDER_LIMIT,
                        Some(class_id),
                        request.game_version.as_deref(),
                        request.loader_filter.curseforge_mod_loader_type(),
                        curseforge_sort_field,
                    ) {
                        Ok(entries) => {
                            provider_result_count += entries.len();
                            provider_entries.extend(entries.into_iter().enumerate().map(
                                |(index, entry)| DiscoverProviderEntry {
                                    project_ref: DiscoverProjectRef::CurseForge {
                                        project_id: entry.id,
                                    },
                                    name: entry.name,
                                    summary: entry.summary,
                                    author: None,
                                    icon_url: entry.icon_url,
                                    primary_url: entry.website_url,
                                    source: DiscoverSource::CurseForge,
                                    popularity_score: Some(entry.download_count),
                                    updated_at: entry.date_modified,
                                    relevance_rank: index as u32,
                                },
                            ));
                        }
                        Err(error) => warnings.push(format!("CurseForge search failed: {error}")),
                    }
                }
            }
            None => warnings.push(
                "CurseForge API key missing in settings. Showing Modrinth results only.".to_owned(),
            ),
        }
    }

    let entries = build_snapshot_entries(provider_entries, request.sort_mode);
    let expected_page_size =
        enabled_provider_count(request).saturating_mul(DISCOVER_PROVIDER_LIMIT as usize);
    DiscoverSearchSnapshot {
        entries,
        warnings,
        has_more: expected_page_size > 0 && provider_result_count >= expected_page_size,
    }
}

fn build_snapshot_entries(
    provider_entries: Vec<DiscoverProviderEntry>,
    sort_mode: DiscoverSortMode,
) -> Vec<DiscoverEntry> {
    let mut deduped = HashMap::<String, DiscoverEntry>::new();
    for entry in provider_entries {
        let dedupe_key = normalize_search_key(entry.name.as_str());
        match deduped.get_mut(&dedupe_key) {
            Some(existing) => {
                if !existing.sources.contains(&entry.source) {
                    existing.sources.push(entry.source);
                }
                if existing.summary.len() < entry.summary.len() {
                    existing.summary = entry.summary.clone();
                }
                if existing.author.is_none() {
                    existing.author = entry.author.clone();
                }
                if existing.icon_url.is_none() {
                    existing.icon_url = entry.icon_url.clone();
                }
                if existing.primary_url.is_none() {
                    existing.primary_url = entry.primary_url.clone();
                }
                if !existing.provider_refs.iter().any(|provider| {
                    provider.source == entry.source
                        && match (&provider.project_ref, &entry.project_ref) {
                            (
                                DiscoverProjectRef::Modrinth {
                                    project_id: left_project_id,
                                },
                                DiscoverProjectRef::Modrinth {
                                    project_id: right_project_id,
                                },
                            ) => left_project_id == right_project_id,
                            (
                                DiscoverProjectRef::CurseForge {
                                    project_id: left_project_id,
                                },
                                DiscoverProjectRef::CurseForge {
                                    project_id: right_project_id,
                                },
                            ) => left_project_id == right_project_id,
                            _ => false,
                        }
                }) {
                    existing.provider_refs.push(DiscoverProviderRef {
                        source: entry.source,
                        project_ref: entry.project_ref.clone(),
                        primary_url: entry.primary_url.clone(),
                    });
                }
                existing.popularity_score =
                    match (existing.popularity_score, entry.popularity_score) {
                        (Some(left), Some(right)) => Some(left.max(right)),
                        (None, right) => right,
                        (left, None) => left,
                    };
                existing.updated_at = existing
                    .updated_at
                    .take()
                    .or_else(|| entry.updated_at.clone());
                existing.relevance_rank = existing.relevance_rank.min(entry.relevance_rank);
            }
            None => {
                let primary_url = entry.primary_url.clone();
                deduped.insert(
                    dedupe_key.clone(),
                    DiscoverEntry {
                        dedupe_key,
                        name: entry.name,
                        summary: entry.summary,
                        author: entry.author,
                        icon_url: entry.icon_url,
                        primary_url: primary_url.clone(),
                        sources: vec![entry.source],
                        provider_refs: vec![DiscoverProviderRef {
                            source: entry.source,
                            project_ref: entry.project_ref,
                            primary_url,
                        }],
                        popularity_score: entry.popularity_score,
                        updated_at: entry.updated_at,
                        relevance_rank: entry.relevance_rank,
                    },
                );
            }
        }
    }

    let mut entries = deduped.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| match sort_mode {
        DiscoverSortMode::Popularity => right
            .popularity_score
            .cmp(&left.popularity_score)
            .then_with(|| left.name.cmp(&right.name)),
        DiscoverSortMode::LastUpdated => right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.name.cmp(&right.name)),
        DiscoverSortMode::Relevance => left
            .relevance_rank
            .cmp(&right.relevance_rank)
            .then_with(|| right.popularity_score.cmp(&left.popularity_score))
            .then_with(|| left.name.cmp(&right.name)),
    });
    entries
}

fn resolve_curseforge_modpack_class_id_cached(
    client: &CurseForgeClient,
    warnings: &mut Vec<String>,
) -> Option<u32> {
    static CACHE: OnceLock<Mutex<Option<Option<u32>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(cache) = cache.lock()
        && let Some(class_id) = *cache
    {
        return class_id;
    }

    let class_id = resolve_curseforge_modpack_class_id(client, warnings);
    if let Ok(mut cache) = cache.lock() {
        *cache = Some(class_id);
    }
    class_id
}

fn resolve_curseforge_modpack_class_id(
    client: &CurseForgeClient,
    warnings: &mut Vec<String>,
) -> Option<u32> {
    match client.list_content_classes(MINECRAFT_GAME_ID) {
        Ok(classes) => classes
            .into_iter()
            .find(|class_entry| {
                let normalized = normalize_search_key(class_entry.name.as_str());
                normalized.contains("modpack") || normalized.contains("mod pack")
            })
            .map(|class_entry| class_entry.id),
        Err(error) => {
            warnings.push(format!(
                "CurseForge modpack class discovery failed: {error}"
            ));
            None
        }
    }
}

fn normalize_search_key(value: &str) -> String {
    value
        .trim()
        .chars()
        .flat_map(|ch| ch.to_lowercase())
        .filter(|ch| ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace())
        .collect::<String>()
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn add_search_tag(search_tags: &mut Vec<String>, candidate: &str) -> bool {
    let Some(normalized) = normalize_search_tag(candidate) else {
        return false;
    };
    if search_tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(normalized.as_str()))
    {
        return false;
    }
    search_tags.push(normalized);
    true
}

fn normalize_search_tag(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn compose_search_query(input: &str, tags: &[String]) -> String {
    let mut parts = Vec::with_capacity(tags.len() + 1);
    for tag in tags {
        let trimmed = tag.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_owned());
        }
    }
    let trimmed_input = input.trim();
    if !trimmed_input.is_empty() {
        parts.push(trimmed_input.to_owned());
    }
    parts.join(" ")
}

fn render_search_tag_chips(ui: &mut Ui, search_tags: &mut Vec<String>) -> bool {
    let mut removed_index: Option<usize> = None;
    ui.add_space(style::SPACE_SM);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_SM, style::SPACE_SM);
        for (index, tag) in search_tags.iter().enumerate() {
            let fill = ui.visuals().selection.bg_fill.gamma_multiply(0.28);
            let stroke = egui::Stroke::new(1.0, ui.visuals().selection.bg_fill.gamma_multiply(0.7));
            let text_color = ui.visuals().text_color();
            let themed_svg = themed_svg_bytes(assets::X_SVG, text_color);
            let uri = format!(
                "bytes://discover/tag-remove/{index}-{:02x}{:02x}{:02x}.svg",
                text_color.r(),
                text_color.g(),
                text_color.b()
            );
            egui::Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_height(24.0);
                        let icon_button = egui::Button::image(
                            egui::Image::from_bytes(uri, themed_svg)
                                .fit_to_exact_size(egui::vec2(16.0, 16.0)),
                        )
                        .frame(false)
                        .min_size(egui::vec2(22.0, 22.0));
                        if ui
                            .add(icon_button)
                            .on_hover_text(format!("Remove tag: {tag}"))
                            .clicked()
                        {
                            removed_index = Some(index);
                        }
                        let _ = ui.label(tag.as_str());
                    });
                });
        }
    });
    if let Some(index) = removed_index {
        search_tags.remove(index);
        true
    } else {
        false
    }
}

fn themed_svg_bytes(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    String::from_utf8_lossy(svg_bytes)
        .replace("currentColor", color_hex.as_str())
        .into_bytes()
}

fn sized_dropdown_picker(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    id: impl std::hash::Hash,
    width: f32,
    selected_index: &mut usize,
    options: &[&str],
) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(width, style::CONTROL_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            settings_widgets::dropdown_picker(text_ui, ui, id, selected_index, options, Some(width))
        },
    )
    .inner
}

fn sized_combo_box(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    width: f32,
    selected_text: &str,
    add_contents: impl FnOnce(&mut Ui),
) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, style::CONTROL_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            egui::ComboBox::from_id_salt(id)
                .width(width)
                .selected_text(selected_text)
                .show_ui(ui, add_contents);
        },
    );
}

fn request_version_catalog(state: &mut DiscoverState) {
    if state.version_catalog_in_flight
        || !state.available_game_versions.is_empty()
        || state.version_catalog_error.is_some()
    {
        return;
    }

    ensure_version_catalog_channel(state);
    let Some(tx) = state.version_catalog_tx.as_ref().cloned() else {
        return;
    };

    state.version_catalog_in_flight = true;
    tracing::info!(target: "vertexlauncher/discover", "Starting discover version catalog fetch.");
    let _ = tokio_runtime::spawn_detached(async move {
        let result: Result<Vec<MinecraftVersionEntry>, String> = match tokio::time::timeout(
            VERSION_CATALOG_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || {
                fetch_version_catalog(false)
                    .map(|catalog| catalog.game_versions)
                    .map_err(|err| err.to_string())
            }),
        )
        .await
        {
            Ok(join_result) => join_result
                .map_err(|err| err.to_string())
                .and_then(|result| result),
            Err(_) => Err(format!(
                "version catalog request timed out after {}s",
                VERSION_CATALOG_FETCH_TIMEOUT.as_secs()
            )),
        };
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/discover",
                error = %err,
                "Failed to deliver discover version catalog result."
            );
        }
    });
}

fn ensure_version_catalog_channel(state: &mut DiscoverState) {
    if state.version_catalog_tx.is_some() && state.version_catalog_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<Result<Vec<MinecraftVersionEntry>, String>>();
    state.version_catalog_tx = Some(tx);
    state.version_catalog_rx = Some(Arc::new(Mutex::new(rx)));
}

fn poll_version_catalog(state: &mut DiscoverState) {
    let mut should_reset_channel = false;
    let mut updates = Vec::new();

    if let Some(rx) = state.version_catalog_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/discover",
                            "Discover version catalog worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/discover",
                    "Discover version catalog receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.version_catalog_tx = None;
        state.version_catalog_rx = None;
        state.version_catalog_in_flight = false;
        state.version_catalog_error =
            Some("Version catalog worker stopped unexpectedly.".to_owned());
    }

    for update in updates {
        state.version_catalog_in_flight = false;
        match update {
            Ok(versions) => {
                state.available_game_versions = versions;
                state.version_catalog_error = None;
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/discover",
                    error = %err,
                    "Discover version catalog fetch failed."
                );
                state.version_catalog_error = Some(err);
            }
        }
    }
}

fn apply_search_snapshot(
    state: &mut DiscoverState,
    request: &DiscoverSearchRequest,
    snapshot: DiscoverSearchSnapshot,
    mode: SearchMode,
) {
    match mode {
        SearchMode::Replace => {
            state.entries = snapshot.entries;
            state.page = request.page.max(1);
        }
        SearchMode::Append => {
            for entry in snapshot.entries {
                if !state
                    .entries
                    .iter()
                    .any(|existing| existing.dedupe_key == entry.dedupe_key)
                {
                    state.entries.push(entry);
                }
            }
            state.page = request.page.max(state.page);
        }
    }
    state.warnings = snapshot.warnings;
    state.has_more_results = snapshot.has_more;
    state.masonry_layout_cache = None;
    retain_discover_tile_heights(state);
}

fn retain_discover_tile_heights(state: &mut DiscoverState) {
    state.tile_height_cache.retain(|(dedupe_key, _), _| {
        state
            .entries
            .iter()
            .any(|entry| entry.dedupe_key.as_str() == dedupe_key.as_str())
    });
    state.tile_height_cache_revision = state.tile_height_cache_revision.saturating_add(1);
    state.masonry_layout_cache = None;
}

fn enabled_provider_count(request: &DiscoverSearchRequest) -> usize {
    match request.provider_filter {
        DiscoverProviderFilter::All => 1 + usize::from(CurseForgeClient::from_env().is_some()),
        DiscoverProviderFilter::Modrinth => 1,
        DiscoverProviderFilter::CurseForge => usize::from(CurseForgeClient::from_env().is_some()),
    }
}
