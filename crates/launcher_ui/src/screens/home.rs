use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use config::Config;
use egui::{Color32, Layout, TextureOptions, Ui};
use flate2::read::GzDecoder;
use instances::{InstanceStore, instance_root_path, set_server_favorite, set_world_favorite};
use launcher_runtime as tokio_runtime;
use textui::TextUi;
use textui_egui::{
    prelude::*, truncate_single_line_text_with_ellipsis_preserving_whitespace as truncate_for_width,
};
use ui_foundation::{
    DialogPreset, UiMetrics, danger_button, dialog_options, fill_tab_row, secondary_button,
    show_dialog,
};

use crate::{
    assets, desktop, install_activity, notification,
    screens::{
        LaunchAuthContext, QuickLaunchCommandMode, build_quick_launch_command,
        build_quick_launch_steam_options, selected_quick_launch_user,
    },
    ui::{
        components::{
            image_memory::prepare_owned_image_bytes_for_memory,
            image_textures,
            lazy_image_bytes::{LazyImageBytes, LazyImageBytesStatus},
            virtual_masonry::{
                CachedVirtualMasonryLayout, build_virtual_masonry_layout,
                render_virtualized_masonry,
            },
        },
        context_menu::{self, ContextMenuItem, ContextMenuRequest},
        instance_context_menu::{self, InstanceContextAction},
        style,
    },
};

use super::{AppScreen, PendingLaunchIntent, queue_launch_intent};

#[path = "home_nbt.rs"]
mod home_nbt;
use home_nbt::{parse_servers_dat, parse_world_metadata};

const HOME_SCAN_INTERVAL: Duration = Duration::from_secs(3);
const SERVER_PING_REFRESH_INTERVAL: Duration = Duration::from_secs(20);
const SERVER_PING_CONNECT_TIMEOUT: Duration = Duration::from_millis(350);
const SERVER_PINGS_PER_SCAN: usize = 3;
const ENTRY_ICON_SIZE: f32 = 14.0;
const SERVER_PING_ICON_SIZE: f32 = 24.0;
const FAVORITE_STAR_BUTTON_SIZE: f32 = 20.0;
const FAVORITE_STAR_ICON_SIZE: f32 = 14.0;
const ACTIVITY_ENTRY_THUMBNAIL_SIZE: f32 = 34.0;
const ACTIVITY_ENTRY_CONTENT_GAP: f32 = 8.0;
const ACTIVITY_ENTRY_ROW_VERTICAL_PADDING: f32 = 5.0;
const ACTIVITY_ENTRY_ROW_HORIZONTAL_PADDING: f32 = 8.0;
const SCREENSHOT_SCAN_INTERVAL: Duration = Duration::from_secs(10);
const SCREENSHOT_PAGE_SIZE: usize = 30;
const SCREENSHOT_TILE_GAP: f32 = 10.0;
const SCREENSHOT_VIEWER_MIN_ZOOM: f32 = 1.0;
const SCREENSHOT_VIEWER_MAX_ZOOM: f32 = 8.0;
const SCREENSHOT_VIEWER_ZOOM_STEP: f32 = 0.12;
const SCREENSHOT_VIEWER_SCROLL_ZOOM_SENSITIVITY: f32 = 0.0015;
const HOME_SCREENSHOT_OVERSCAN: f32 = 420.0;
const HOME_THUMBNAIL_CACHE_MAX_BYTES: usize = 24 * 1024 * 1024;
const HOME_THUMBNAIL_CACHE_STALE_FRAMES: u64 = 900;

#[path = "home_screenshots.rs"]
mod home_screenshots;
#[path = "home_server_ping.rs"]
mod home_server_ping;
#[path = "home_support.rs"]
mod home_support;
#[path = "home_thumbnails.rs"]
mod home_thumbnails;

use self::home_screenshots::{
    ScreenshotCandidate, ScreenshotEntry, ScreenshotViewerState,
    handle_escape as handle_screenshot_escape, poll_delete_screenshot_results,
    poll_screenshot_results, purge_screenshot_state as purge_home_screenshot_feature_state,
    refresh_screenshot_state, render_delete_screenshot_modal, render_screenshot_gallery,
    render_screenshot_viewer_modal, retain_home_viewer_image,
};
use self::home_server_ping::{
    ServerEntry, ServerPingSnapshot, collect_servers_from_request, home_server_icon_uri,
    normalize_server_address, poll_server_ping_results, queue_server_pings,
    render_server_ping_icon, retain_known_server_pings, server_meta_line,
};
use self::home_support::{
    current_time_millis, format_time_ago, modified_millis, open_home_instance,
    open_home_instance_folder,
};
use self::home_thumbnails::{
    HomeThumbnailState, home_instance_thumbnail_uri, home_world_thumbnail_uri,
    instance_thumbnail_cache_key, poll_instance_thumbnail_results,
    purge_activity_image_state as purge_home_activity_thumbnail_state, request_instance_thumbnail,
    trim_home_thumbnail_cache,
};

#[derive(Clone, Copy, Debug)]
struct HomeUiMetrics {
    tab_height: f32,
    instance_row_height: f32,
    activity_row_height: f32,
    screenshot_overlay_button_size: f32,
    screenshot_min_column_width: f32,
    action_button_width: f32,
}

impl HomeUiMetrics {
    fn from_ui(ui: &Ui) -> Self {
        let metrics = UiMetrics::from_ui(ui, 820.0);
        Self {
            tab_height: metrics.scaled_height(0.045, 34.0, 40.0),
            instance_row_height: metrics.scaled_height(0.04, 34.0, 42.0),
            activity_row_height: metrics.scaled_height(0.062, 50.0, 62.0),
            screenshot_overlay_button_size: metrics.scaled_width(0.022, 24.0, 30.0),
            screenshot_min_column_width: metrics.scaled_width(0.24, 180.0, 320.0),
            action_button_width: metrics.scaled_width(0.075, 92.0, 120.0),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HomeOutput {
    pub requested_screen: Option<AppScreen>,
    pub selected_instance_id: Option<String>,
    pub delete_requested_instance_id: Option<String>,
    pub presence_section: HomePresenceSection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum HomeTab {
    #[default]
    InstancesAndWorlds,
    Screenshots,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomePresenceSection {
    Activity,
    Screenshots,
}

impl Default for HomePresenceSection {
    fn default() -> Self {
        Self::Activity
    }
}

impl HomeTab {
    fn label(self) -> &'static str {
        match self {
            Self::InstancesAndWorlds => "Instances & Worlds",
            Self::Screenshots => "Screenshots",
        }
    }
}

impl HomeTab {
    fn presence_section(self) -> HomePresenceSection {
        match self {
            Self::InstancesAndWorlds => HomePresenceSection::Activity,
            Self::Screenshots => HomePresenceSection::Screenshots,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct HomeState {
    active_tab: HomeTab,
    worlds: Vec<WorldEntry>,
    servers: Vec<ServerEntry>,
    server_pings: HashMap<String, ServerPingSnapshot>,
    last_scan_at: Option<Instant>,
    scanned_instance_count: usize,
    activity_scan_pending: bool,
    latest_requested_activity_scan_id: u64,
    server_ping_in_flight: HashSet<String>,
    screenshots: Vec<ScreenshotEntry>,
    last_screenshot_scan_at: Option<Instant>,
    scanned_screenshot_instance_count: usize,
    screenshot_scan_pending: bool,
    screenshot_scan_ready: bool,
    screenshot_tasks_total: usize,
    screenshot_tasks_done: usize,
    screenshot_candidates: Vec<ScreenshotCandidate>,
    screenshot_loaded_count: usize,
    latest_requested_screenshot_scan_id: u64,
    screenshot_images: LazyImageBytes,
    screenshot_layout_revision: u64,
    screenshot_masonry_layout_cache: Option<CachedVirtualMasonryLayout>,
    thumbnails: HomeThumbnailState,
    screenshot_viewer: Option<ScreenshotViewerState>,
    pending_delete_screenshot_key: Option<String>,
    delete_screenshot_in_flight: bool,
    delete_screenshot_results_tx: Option<mpsc::Sender<(String, String, Result<(), String>)>>,
    delete_screenshot_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, String, Result<(), String>)>>>>,
}

#[derive(Debug, Clone)]
struct WorldEntry {
    instance_id: String,
    instance_name: String,
    world_id: String,
    world_name: String,
    game_mode: Option<String>,
    hardcore: Option<bool>,
    cheats_enabled: Option<bool>,
    difficulty: Option<String>,
    version_name: Option<String>,
    thumbnail_png: Option<Arc<[u8]>>,
    last_used_at_ms: Option<u64>,
    favorite: bool,
}

#[derive(Debug, Clone)]
struct HomeActivityScanRequest {
    scanned_instance_count: usize,
    instances: Vec<HomeActivityScanInstance>,
}

#[derive(Debug, Clone)]
struct HomeActivityScanInstance {
    instance_id: String,
    instance_name: String,
    instance_root: PathBuf,
    favorite_world_ids: Vec<String>,
    favorite_server_ids: Vec<String>,
}

#[derive(Debug, Clone)]
struct HomeActivityScanResult {
    request_id: u64,
    scanned_instance_count: usize,
    worlds: Vec<WorldEntry>,
    servers: Vec<ServerEntry>,
}

impl HomeState {
    fn mark_screenshot_layout_dirty(&mut self) {
        self.screenshot_layout_revision = self.screenshot_layout_revision.saturating_add(1);
        self.screenshot_masonry_layout_cache = None;
    }

    fn purge_screenshot_state(&mut self, ctx: &egui::Context) {
        self.latest_requested_screenshot_scan_id =
            self.latest_requested_screenshot_scan_id.saturating_add(1);
        self.screenshots.clear();
        self.last_screenshot_scan_at = None;
        self.scanned_screenshot_instance_count = 0;
        self.screenshot_scan_pending = false;
        self.screenshot_scan_ready = false;
        self.screenshot_tasks_total = 0;
        self.screenshot_tasks_done = 0;
        self.screenshot_candidates.clear();
        self.screenshot_loaded_count = 0;
        self.screenshot_images.clear(ctx);
        self.screenshot_viewer = None;
        self.pending_delete_screenshot_key = None;
        self.delete_screenshot_in_flight = false;
        self.delete_screenshot_results_tx = None;
        self.delete_screenshot_results_rx = None;
        self.mark_screenshot_layout_dirty();
    }

    fn purge_activity_image_state(&mut self, ctx: &egui::Context) {
        for world in &mut self.worlds {
            if world.thumbnail_png.take().is_some() {
                image_textures::evict_source_key(&home_world_thumbnail_uri(
                    world.instance_id.as_str(),
                    world.world_id.as_str(),
                ));
            }
        }
        for server in &mut self.servers {
            if server.icon_png.take().is_some() {
                image_textures::evict_source_key(&home_server_icon_uri(
                    server.instance_id.as_str(),
                    server.favorite_id.as_str(),
                ));
            }
        }
        purge_home_activity_thumbnail_state(ctx, &mut self.thumbnails);
        self.last_scan_at = None;
    }
}

struct HomeActivityResultChannel {
    tx: mpsc::Sender<HomeActivityScanResult>,
    rx: mpsc::Receiver<HomeActivityScanResult>,
}

static HOME_ACTIVITY_RESULTS: OnceLock<Mutex<HomeActivityResultChannel>> = OnceLock::new();

enum HomeEntryRef<'a> {
    World(&'a WorldEntry),
    Server(&'a ServerEntry),
}

impl HomeEntryRef<'_> {
    fn last_used_at_ms(&self) -> Option<u64> {
        match self {
            Self::World(world) => world.last_used_at_ms,
            Self::Server(server) => server.last_used_at_ms,
        }
    }

    fn primary_label(&self) -> &str {
        match self {
            Self::World(world) => world.world_name.as_str(),
            Self::Server(server) => server.server_name.as_str(),
        }
    }
}

fn home_state_id() -> egui::Id {
    egui::Id::new("home_screen_state")
}

pub fn purge_inactive_state(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        if let Some(mut state) = data.get_temp::<HomeState>(home_state_id()) {
            state.purge_screenshot_state(ctx);
            state.purge_activity_image_state(ctx);
        }
        data.insert_temp(home_state_id(), HomeState::default());
    });
}

pub fn purge_screenshot_state(ctx: &egui::Context) {
    purge_home_screenshot_feature_state(ctx);
}

pub fn set_gamepad_screenshot_viewer_input(ctx: &egui::Context, pan: egui::Vec2, zoom: f32) {
    home_screenshots::set_gamepad_screenshot_viewer_input(ctx, pan, zoom);
}

fn home_activity_results() -> &'static Mutex<HomeActivityResultChannel> {
    HOME_ACTIVITY_RESULTS.get_or_init(|| {
        let (result_tx, result_rx) = mpsc::channel::<HomeActivityScanResult>();
        Mutex::new(HomeActivityResultChannel {
            rx: result_rx,
            tx: result_tx,
        })
    })
}

fn build_home_activity_scan_request(
    instances: &InstanceStore,
    config: &Config,
) -> HomeActivityScanRequest {
    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let activity_instances = instances
        .instances
        .iter()
        .map(|instance| HomeActivityScanInstance {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            instance_root: instance_root_path(installations_root.as_path(), instance),
            favorite_world_ids: instance.favorite_world_ids.clone(),
            favorite_server_ids: instance.favorite_server_ids.clone(),
        })
        .collect();

    HomeActivityScanRequest {
        scanned_instance_count: instances.instances.len(),
        instances: activity_instances,
    }
}

fn poll_home_activity_results(state: &mut HomeState) {
    let Ok(channel) = home_activity_results().lock() else {
        tracing::error!(
            target: "vertexlauncher/home",
            request_id = state.latest_requested_activity_scan_id,
            "Home activity results receiver mutex was poisoned while polling scan results."
        );
        return;
    };

    while let Ok(result) = channel.rx.try_recv() {
        if result.request_id != state.latest_requested_activity_scan_id {
            continue;
        }
        state.worlds = result.worlds;
        state.servers = result.servers;
        state.scanned_instance_count = result.scanned_instance_count;
        state.last_scan_at = Some(Instant::now());
        state.activity_scan_pending = false;
        retain_known_server_pings(state);
    }
}

pub fn presence_section(ctx: &egui::Context) -> HomePresenceSection {
    let state_id = home_state_id();
    let state = ctx.data_mut(|data| data.get_temp::<HomeState>(state_id));
    state
        .map(|state| state.active_tab.presence_section())
        .unwrap_or(HomePresenceSection::Activity)
}

pub(super) fn handle_escape(ctx: &egui::Context) -> bool {
    handle_screenshot_escape(ctx)
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instances: &mut InstanceStore,
    config: &Config,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    streamer_mode: bool,
) -> HomeOutput {
    let mut output = HomeOutput::default();
    ui.ctx()
        .options_mut(|options| options.reduce_texture_memory = true);
    let metrics = HomeUiMetrics::from_ui(ui);
    let state_id = home_state_id();
    let mut state = ui
        .ctx()
        .data_mut(|data| data.get_temp::<HomeState>(state_id))
        .unwrap_or_default();
    let previous_tab = state.active_tab;

    render_home_tab_row(ui, text_ui, &mut state.active_tab, metrics);
    if previous_tab == HomeTab::Screenshots && state.active_tab != HomeTab::Screenshots {
        state.purge_screenshot_state(ui.ctx());
    }
    if previous_tab == HomeTab::InstancesAndWorlds
        && state.active_tab != HomeTab::InstancesAndWorlds
    {
        state.purge_activity_image_state(ui.ctx());
    }
    ui.ctx().request_repaint_after(Duration::from_millis(250));
    state.screenshot_images.begin_frame(ui.ctx());
    state.thumbnails.cache_frame_index = state.thumbnails.cache_frame_index.saturating_add(1);
    trim_home_thumbnail_cache(ui.ctx(), &mut state.thumbnails);
    let screenshot_images_updated = state.screenshot_images.poll(ui.ctx());
    ui.add_space(14.0);

    match state.active_tab {
        HomeTab::InstancesAndWorlds => {
            poll_home_activity_results(&mut state);
            poll_server_ping_results(&mut state);
            poll_instance_thumbnail_results(ui.ctx(), &mut state.thumbnails);
            let should_scan = state
                .last_scan_at
                .is_none_or(|last| last.elapsed() >= HOME_SCAN_INTERVAL)
                || state.scanned_instance_count != instances.instances.len();
            if should_scan {
                refresh_home_state(&mut state, instances, config, false);
            }
            queue_server_pings(&mut state);
            if state.activity_scan_pending || !state.server_ping_in_flight.is_empty() {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
            if !state.thumbnails.cache.is_empty() {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }

            let mut requested_rescan = false;
            render_instance_usage(
                ui,
                text_ui,
                instances,
                config,
                active_username,
                active_launch_auth,
                &mut state,
                &mut output,
                metrics,
            );
            ui.add_space(12.0);
            render_activity_feed(
                ui,
                text_ui,
                instances,
                &state,
                active_username,
                active_launch_auth,
                streamer_mode,
                &mut output,
                &mut requested_rescan,
                metrics,
            );

            if requested_rescan {
                refresh_home_state(&mut state, instances, config, true);
            }

            let mut retained_image_keys = HashSet::new();
            retain_home_viewer_image(&mut state, &mut retained_image_keys);
            state
                .screenshot_images
                .retain_loaded(ui.ctx(), &retained_image_keys);
        }
        HomeTab::Screenshots => {
            poll_delete_screenshot_results(&mut state, instances, config);
            poll_screenshot_results(&mut state);
            let should_scan = state
                .last_screenshot_scan_at
                .is_none_or(|last| last.elapsed() >= SCREENSHOT_SCAN_INTERVAL)
                || state.scanned_screenshot_instance_count != instances.instances.len();
            if should_scan {
                refresh_screenshot_state(&mut state, instances, config, false);
            }
            if state.screenshot_scan_pending || state.delete_screenshot_in_flight {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
            if state.screenshot_viewer.as_ref().is_some_and(|viewer| {
                !state
                    .screenshots
                    .iter()
                    .any(|screenshot| screenshot.key() == viewer.screenshot_key)
            }) && !state.screenshot_scan_pending
            {
                tracing::info!(
                    target: "vertexlauncher/screenshots",
                    "Home screenshot viewer closed because the selected screenshot disappeared from the gallery state."
                );
                state.screenshot_viewer = None;
            }
            if state
                .pending_delete_screenshot_key
                .as_ref()
                .is_some_and(|pending| {
                    !state
                        .screenshots
                        .iter()
                        .any(|screenshot| screenshot.key() == *pending)
                })
            {
                state.pending_delete_screenshot_key = None;
            }
            let mut retained_image_keys = HashSet::new();
            render_screenshot_gallery(ui, text_ui, &mut state, &mut retained_image_keys, metrics);
            retain_home_viewer_image(&mut state, &mut retained_image_keys);
            state
                .screenshot_images
                .retain_loaded(ui.ctx(), &retained_image_keys);
        }
    }

    if screenshot_images_updated
        || (state.screenshot_images.has_in_flight()
            && (state.active_tab == HomeTab::Screenshots || state.screenshot_viewer.is_some()))
    {
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }

    render_screenshot_viewer_modal(ui.ctx(), text_ui, &mut state, metrics);
    render_delete_screenshot_modal(ui.ctx(), text_ui, &mut state, instances, config, metrics);
    output.presence_section = state.active_tab.presence_section();
    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
    output
}

fn render_home_tab_row(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    active_tab: &mut HomeTab,
    metrics: HomeUiMetrics,
) {
    fill_tab_row(
        text_ui,
        ui,
        "home_tab_row",
        active_tab,
        &[
            (
                HomeTab::InstancesAndWorlds,
                HomeTab::InstancesAndWorlds.label(),
            ),
            (HomeTab::Screenshots, HomeTab::Screenshots.label()),
        ],
        metrics.tab_height,
        style::SPACE_MD,
    );
}

fn render_instance_usage(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instances: &InstanceStore,
    config: &Config,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    state: &mut HomeState,
    output: &mut HomeOutput,
    metrics: HomeUiMetrics,
) {
    let mut title_style = LabelOptions::default();
    title_style.font_size = 18.0;
    title_style.line_height = 24.0;
    title_style.weight = 700;
    title_style.color = ui.visuals().text_color();
    let _ = text_ui.label(ui, "home_usage_title", "Most Used Instances", &title_style);
    ui.add_space(6.0);

    let mut items = instances.instances.iter().collect::<Vec<_>>();
    items.sort_by(|a, b| {
        b.launch_count
            .cmp(&a.launch_count)
            .then_with(|| b.last_launched_at_ms.cmp(&a.last_launched_at_ms))
            .then_with(|| a.name.cmp(&b.name))
    });
    if items.is_empty() {
        let _ = text_ui.label(
            ui,
            "home_usage_empty",
            "No instances yet.",
            &style::muted(ui),
        );
        return;
    }

    let max_height = (ui.available_height() * (1.0 / 3.0)).clamp(140.0, 340.0);
    let now_ms = current_time_millis();
    egui::ScrollArea::vertical()
        .id_salt("home_instances_scroll")
        .max_height(max_height)
        .show(ui, |ui| {
            for (index, instance) in items.into_iter().enumerate() {
                let thumbnail = instance
                    .thumbnail_path
                    .as_deref()
                    .filter(|path| !path.as_os_str().is_empty())
                    .and_then(|path| {
                        let key = instance_thumbnail_cache_key(instance.id.as_str(), path);
                        match state.thumbnails.cache.get_mut(&key) {
                            Some(entry) => {
                                entry.last_touched_frame = state.thumbnails.cache_frame_index;
                                entry.bytes.clone().map(|bytes| {
                                    (
                                        home_instance_thumbnail_uri(instance.id.as_str(), path),
                                        bytes,
                                    )
                                })
                            }
                            None => {
                                request_instance_thumbnail(
                                    &mut state.thumbnails,
                                    instance.id.as_str(),
                                    path.to_path_buf(),
                                );
                                None
                            }
                        }
                    });
                let row_response = render_clickable_entry_row(
                    ui,
                    ("home_instance_row", index),
                    metrics.instance_row_height,
                    |ui| {
                        render_entry_thumbnail(
                            ui,
                            ("home_instance_thumb", instance.id.as_str()),
                            thumbnail.clone(),
                            assets::LIBRARY_SVG,
                            40.0,
                            18.0,
                        );
                        ui.add_space(8.0);
                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            let usage_line = format!(
                                "{} launches | {}",
                                instance.launch_count,
                                format_time_ago(instance.last_launched_at_ms, now_ms)
                            );
                            let _ = text_ui.label(
                                ui,
                                ("home_usage_count", index),
                                usage_line.as_str(),
                                &style::muted_single_line(ui),
                            );
                            ui.with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                                let _ = text_ui.label(
                                    ui,
                                    ("home_usage_name", index),
                                    instance.name.as_str(),
                                    &style::body_strong(ui),
                                );
                            });
                        });
                    },
                );
                let context_id =
                    ui.make_persistent_id(("home_instance_context", instance.id.as_str()));

                if row_response.clicked() {
                    if install_activity::is_instance_installing(instance.id.as_str()) {
                        output.selected_instance_id = Some(instance.id.clone());
                        output.requested_screen = Some(AppScreen::Library);
                    } else {
                        queue_launch_intent(
                            ui.ctx(),
                            PendingLaunchIntent {
                                nonce: current_time_millis(),
                                instance_id: instance.id.clone(),
                                quick_play_singleplayer: None,
                                quick_play_multiplayer: None,
                            },
                        );
                        output.selected_instance_id = Some(instance.id.clone());
                        output.requested_screen = Some(AppScreen::Library);
                    }
                }

                if row_response.secondary_clicked() {
                    let anchor = row_response
                        .interact_pointer_pos()
                        .or_else(|| ui.ctx().pointer_latest_pos())
                        .unwrap_or(row_response.rect.left_bottom());
                    instance_context_menu::request_for_instance(ui.ctx(), context_id, anchor, true);
                }

                if let Some(action) = instance_context_menu::take(ui.ctx(), context_id) {
                    match action {
                        InstanceContextAction::OpenInstance => {
                            open_home_instance(output, instance.id.as_str());
                        }
                        InstanceContextAction::OpenFolder => {
                            if let Err(err) =
                                open_home_instance_folder(instance.id.as_str(), instances, config)
                            {
                                notification::emit_replace(
                                    notification::Severity::Error,
                                    format!("home-instance-folder-{}", instance.id),
                                    format!("Failed to open instance folder: {err}"),
                                    format!("home-instance-folder/{}/error", instance.id),
                                );
                            }
                        }
                        InstanceContextAction::CopyLaunchCommand => {
                            copy_instance_launch_command(
                                ui.ctx(),
                                instance.id.as_str(),
                                active_username,
                                active_launch_auth,
                            );
                        }
                        InstanceContextAction::CopySteamLaunchOptions => {
                            copy_instance_steam_launch_options(
                                ui.ctx(),
                                instance.id.as_str(),
                                active_username,
                                active_launch_auth,
                            );
                        }
                        InstanceContextAction::Delete => {
                            output.delete_requested_instance_id = Some(instance.id.clone());
                        }
                    }
                }

                ui.add_space(3.0);
            }
        });
}

fn render_activity_feed(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instances: &mut InstanceStore,
    state: &HomeState,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    streamer_mode: bool,
    output: &mut HomeOutput,
    requested_rescan: &mut bool,
    metrics: HomeUiMetrics,
) {
    let mut title_style = LabelOptions::default();
    title_style.font_size = 18.0;
    title_style.line_height = 24.0;
    title_style.weight = 700;
    title_style.color = ui.visuals().text_color();
    let _ = text_ui.label(ui, "home_activity_title", "Worlds & Servers", &title_style);
    ui.add_space(6.0);

    if state.worlds.is_empty() && state.servers.is_empty() {
        let _ = text_ui.label(
            ui,
            "home_activity_empty",
            "No worlds or servers found in any instance.",
            &style::muted(ui),
        );
        return;
    }

    let now_ms = current_time_millis();
    let mut favorites: Vec<HomeEntryRef<'_>> = state
        .worlds
        .iter()
        .filter(|world| world.favorite)
        .map(HomeEntryRef::World)
        .collect();
    favorites.extend(
        state
            .servers
            .iter()
            .filter(|server| server.favorite)
            .map(HomeEntryRef::Server),
    );
    favorites.sort_by(|a, b| {
        b.last_used_at_ms()
            .unwrap_or(0)
            .cmp(&a.last_used_at_ms().unwrap_or(0))
            .then_with(|| a.primary_label().cmp(b.primary_label()))
    });

    let mut entries: Vec<HomeEntryRef<'_>> = state
        .worlds
        .iter()
        .filter(|world| !world.favorite)
        .map(HomeEntryRef::World)
        .collect();
    entries.extend(
        state
            .servers
            .iter()
            .filter(|server| !server.favorite)
            .map(HomeEntryRef::Server),
    );
    entries.sort_by(|a, b| {
        b.last_used_at_ms()
            .unwrap_or(0)
            .cmp(&a.last_used_at_ms().unwrap_or(0))
            .then_with(|| a.primary_label().cmp(b.primary_label()))
    });

    egui::ScrollArea::vertical()
        .id_salt("home_activity_scroll")
        .max_height(ui.available_height().max(180.0))
        .show(ui, |ui| {
            if !favorites.is_empty() {
                let _ = text_ui.label(
                    ui,
                    "home_activity_favorites_title",
                    "Favorites",
                    &style::body_strong(ui),
                );
                ui.add_space(4.0);
                for (index, entry) in favorites.into_iter().enumerate() {
                    match entry {
                        HomeEntryRef::World(world) => render_world_row(
                            ui,
                            text_ui,
                            world,
                            now_ms,
                            ("home_favorite_world", index),
                            instances,
                            output,
                            requested_rescan,
                            active_username,
                            active_launch_auth,
                            metrics,
                        ),
                        HomeEntryRef::Server(server) => render_server_row(
                            ui,
                            text_ui,
                            server,
                            state
                                .server_pings
                                .get(&normalize_server_address(&server.address)),
                            now_ms,
                            streamer_mode,
                            ("home_favorite_server", index),
                            instances,
                            output,
                            requested_rescan,
                            active_username,
                            active_launch_auth,
                            metrics,
                        ),
                    }
                    ui.add_space(2.0);
                }
                ui.separator();
                ui.add_space(8.0);
            }

            if entries.is_empty() {
                let _ = text_ui.label(
                    ui,
                    "home_activity_recent_empty",
                    "No recent worlds or servers.",
                    &style::muted(ui),
                );
                return;
            }

            let _ = text_ui.label(
                ui,
                "home_activity_recent_title",
                "Recent",
                &style::body_strong(ui),
            );
            ui.add_space(4.0);
            for (index, entry) in entries.into_iter().enumerate() {
                match entry {
                    HomeEntryRef::World(world) => {
                        render_world_row(
                            ui,
                            text_ui,
                            world,
                            now_ms,
                            ("home_recent_world", index),
                            instances,
                            output,
                            requested_rescan,
                            active_username,
                            active_launch_auth,
                            metrics,
                        );
                    }
                    HomeEntryRef::Server(server) => {
                        render_server_row(
                            ui,
                            text_ui,
                            server,
                            state
                                .server_pings
                                .get(&normalize_server_address(&server.address)),
                            now_ms,
                            streamer_mode,
                            ("home_recent_server", index),
                            instances,
                            output,
                            requested_rescan,
                            active_username,
                            active_launch_auth,
                            metrics,
                        );
                    }
                }
                ui.add_space(2.0);
            }
        });
}

fn render_world_row(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    world: &WorldEntry,
    now_ms: u64,
    id_source: impl std::hash::Hash + Copy,
    instances: &mut InstanceStore,
    output: &mut HomeOutput,
    requested_rescan: &mut bool,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    metrics: HomeUiMetrics,
) {
    let name_label_options = activity_entry_name_label_options(ui);
    let meta_label_options = activity_entry_meta_label_options(ui);
    let row_height = activity_entry_min_row_height(
        ui,
        text_ui,
        metrics,
        &name_label_options,
        &meta_label_options,
        0.0,
    );
    let mut star_clicked = false;
    let row_response = render_clickable_entry_row(ui, (id_source, "row"), row_height, |ui| {
        if render_favorite_star_button(ui, (id_source, "world_star"), world.favorite)
            .on_hover_text("Toggle world favorite")
            .clicked()
        {
            star_clicked = true;
        }
        ui.add_space(ACTIVITY_ENTRY_CONTENT_GAP);
        render_entry_thumbnail(
            ui,
            (id_source, "thumb"),
            world.thumbnail_png.clone().map(|bytes| {
                (
                    home_world_thumbnail_uri(world.instance_id.as_str(), world.world_id.as_str()),
                    bytes,
                )
            }),
            assets::HOME_SVG,
            ACTIVITY_ENTRY_THUMBNAIL_SIZE,
            ACTIVITY_ENTRY_THUMBNAIL_SIZE,
        );
        ui.add_space(ACTIVITY_ENTRY_CONTENT_GAP);
        let text_max_width = ui.available_width().max(80.0);
        let world_name = truncate_for_width(
            text_ui,
            ui,
            world.world_name.as_str(),
            text_max_width,
            &name_label_options,
        );
        let world_meta = truncate_for_width(
            text_ui,
            ui,
            world_meta_line(world, now_ms).as_str(),
            text_max_width,
            &meta_label_options,
        );
        ui.vertical(|ui| {
            ui.set_max_width(text_max_width);
            let _ = text_ui.label(
                ui,
                (id_source, "name"),
                world_name.as_str(),
                &name_label_options,
            );
            let _ = text_ui.label(
                ui,
                (id_source, "meta"),
                world_meta.as_str(),
                &meta_label_options,
            );
        });
    });
    if star_clicked {
        let _ = set_world_favorite(
            instances,
            world.instance_id.as_str(),
            world.world_id.as_str(),
            !world.favorite,
        );
        *requested_rescan = true;
        return;
    }
    if row_response.clicked() {
        queue_launch_intent(
            ui.ctx(),
            PendingLaunchIntent {
                nonce: current_time_millis(),
                instance_id: world.instance_id.clone(),
                quick_play_singleplayer: Some(world.world_id.clone()),
                quick_play_multiplayer: None,
            },
        );
        output.selected_instance_id = Some(world.instance_id.clone());
        output.requested_screen = Some(AppScreen::Library);
    }
    let context_id = ui.make_persistent_id((id_source, "world_context"));
    if row_response.secondary_clicked() {
        let anchor = row_response
            .interact_pointer_pos()
            .or_else(|| ui.ctx().pointer_latest_pos())
            .unwrap_or(row_response.rect.left_bottom());
        context_menu::request(
            ui.ctx(),
            ContextMenuRequest::new(
                context_id,
                anchor,
                vec![
                    ContextMenuItem::new_with_icon(
                        "copy_world_launch_command",
                        "Copy command line",
                        assets::TERMINAL_SVG,
                    ),
                    ContextMenuItem::new_with_icon(
                        "copy_world_steam_launch_options",
                        "Copy Steam launch options",
                        assets::STEAM_SVG,
                    ),
                ],
            ),
        );
    }
    match context_menu::take_invocation(ui.ctx(), context_id).as_deref() {
        Some("copy_world_launch_command") => {
            copy_world_launch_command(ui.ctx(), world, active_username, active_launch_auth);
        }
        Some("copy_world_steam_launch_options") => {
            copy_world_steam_launch_options(ui.ctx(), world, active_username, active_launch_auth);
        }
        _ => {}
    }
}

fn world_meta_line(world: &WorldEntry, now_ms: u64) -> String {
    let mut parts = vec![
        format!("instance {}", world.instance_name),
        format!("folder {}", world.world_id),
        format!(
            "last used {}",
            format_time_ago(world.last_used_at_ms, now_ms)
        ),
    ];
    if let Some(game_mode) = world.game_mode.as_deref() {
        parts.push(game_mode.to_owned());
    }
    if let Some(difficulty) = world.difficulty.as_deref() {
        parts.push(format!("difficulty {difficulty}"));
    }
    if let Some(hardcore) = world.hardcore {
        parts.push(if hardcore {
            "hardcore".to_owned()
        } else {
            "non-hardcore".to_owned()
        });
    }
    if let Some(cheats_enabled) = world.cheats_enabled {
        parts.push(if cheats_enabled {
            "cheats on".to_owned()
        } else {
            "cheats off".to_owned()
        });
    }
    if let Some(version_name) = world.version_name.as_deref() {
        parts.push(format!("version {version_name}"));
    }
    parts.join(" | ")
}

fn render_server_row(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    server: &ServerEntry,
    ping: Option<&ServerPingSnapshot>,
    now_ms: u64,
    streamer_mode: bool,
    id_source: impl std::hash::Hash + Copy,
    instances: &mut InstanceStore,
    output: &mut HomeOutput,
    requested_rescan: &mut bool,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    metrics: HomeUiMetrics,
) {
    let server_meta_full = server_meta_line(server, ping, now_ms, streamer_mode);
    let name_label_options = activity_entry_name_label_options(ui);
    let meta_label_options = activity_entry_meta_label_options(ui);
    let row_height = activity_entry_min_row_height(
        ui,
        text_ui,
        metrics,
        &name_label_options,
        &meta_label_options,
        SERVER_PING_ICON_SIZE,
    );
    let mut star_clicked = false;
    let row_response = render_clickable_entry_row(ui, (id_source, "row"), row_height, |ui| {
        if render_favorite_star_button(ui, (id_source, "server_star"), server.favorite)
            .on_hover_text("Toggle server favorite")
            .clicked()
        {
            star_clicked = true;
        }
        ui.add_space(ACTIVITY_ENTRY_CONTENT_GAP);
        render_entry_thumbnail(
            ui,
            (id_source, "thumb"),
            server.icon_png.clone().map(|bytes| {
                (
                    home_server_icon_uri(server.instance_id.as_str(), server.favorite_id.as_str()),
                    bytes,
                )
            }),
            assets::TERMINAL_SVG,
            ACTIVITY_ENTRY_THUMBNAIL_SIZE,
            ACTIVITY_ENTRY_THUMBNAIL_SIZE,
        );
        ui.add_space(ACTIVITY_ENTRY_CONTENT_GAP);
        let text_max_width = (ui.available_width() - SERVER_PING_ICON_SIZE - 8.0).max(80.0);
        let server_name = truncate_for_width(
            text_ui,
            ui,
            server.server_name.as_str(),
            text_max_width,
            &name_label_options,
        );
        let server_meta = truncate_for_width(
            text_ui,
            ui,
            server_meta_full.as_str(),
            text_max_width,
            &meta_label_options,
        );
        ui.vertical(|ui| {
            ui.set_max_width(text_max_width);
            let _ = text_ui.label(
                ui,
                (id_source, "name"),
                server_name.as_str(),
                &name_label_options,
            );
            let _ = text_ui.label(
                ui,
                (id_source, "meta"),
                server_meta.as_str(),
                &meta_label_options,
            );
        });
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            render_server_ping_icon(ui, ping);
        });
    });
    if star_clicked {
        let _ = set_server_favorite(
            instances,
            server.instance_id.as_str(),
            server.favorite_id.as_str(),
            !server.favorite,
        );
        *requested_rescan = true;
        return;
    }
    if row_response.clicked() {
        queue_launch_intent(
            ui.ctx(),
            PendingLaunchIntent {
                nonce: current_time_millis(),
                instance_id: server.instance_id.clone(),
                quick_play_singleplayer: None,
                quick_play_multiplayer: Some(server.address.clone()),
            },
        );
        output.selected_instance_id = Some(server.instance_id.clone());
        output.requested_screen = Some(AppScreen::Library);
    }
    let context_id = ui.make_persistent_id((id_source, "server_context"));
    if row_response.secondary_clicked() {
        let anchor = row_response
            .interact_pointer_pos()
            .or_else(|| ui.ctx().pointer_latest_pos())
            .unwrap_or(row_response.rect.left_bottom());
        context_menu::request(
            ui.ctx(),
            ContextMenuRequest::new(
                context_id,
                anchor,
                vec![
                    ContextMenuItem::new_with_icon(
                        "copy_server_launch_command",
                        "Copy command line",
                        assets::TERMINAL_SVG,
                    ),
                    ContextMenuItem::new_with_icon(
                        "copy_server_steam_launch_options",
                        "Copy Steam launch options",
                        assets::STEAM_SVG,
                    ),
                ],
            ),
        );
    }
    match context_menu::take_invocation(ui.ctx(), context_id).as_deref() {
        Some("copy_server_launch_command") => {
            copy_server_launch_command(ui.ctx(), server, active_username, active_launch_auth);
        }
        Some("copy_server_steam_launch_options") => {
            copy_server_steam_launch_options(ui.ctx(), server, active_username, active_launch_auth);
        }
        _ => {}
    }
}

fn activity_entry_name_label_options(ui: &Ui) -> LabelOptions {
    style::body_strong(ui)
}

fn activity_entry_meta_label_options(ui: &Ui) -> LabelOptions {
    style::muted_single_line(ui)
}

fn activity_entry_min_row_height(
    ui: &Ui,
    text_ui: &mut TextUi,
    metrics: HomeUiMetrics,
    name_label_options: &LabelOptions,
    meta_label_options: &LabelOptions,
    trailing_content_height: f32,
) -> f32 {
    let name_height = activity_entry_label_height(ui, text_ui, name_label_options);
    let meta_height = activity_entry_label_height(ui, text_ui, meta_label_options);
    let text_stack_height = name_height + ui.spacing().item_spacing.y + meta_height;
    let content_height = text_stack_height
        .max(ACTIVITY_ENTRY_THUMBNAIL_SIZE)
        .max(FAVORITE_STAR_BUTTON_SIZE)
        .max(trailing_content_height);
    metrics
        .activity_row_height
        .max((content_height + (ACTIVITY_ENTRY_ROW_VERTICAL_PADDING * 2.0)).ceil())
}

fn activity_entry_label_height(ui: &Ui, text_ui: &mut TextUi, label_options: &LabelOptions) -> f32 {
    (text_ui.measure_text_size(ui, "Ag", label_options).y + (label_options.padding.y * 2.0)).ceil()
}

fn copy_instance_launch_command(
    ctx: &egui::Context,
    instance_id: &str,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
) {
    let Some(user) = selected_quick_launch_user(active_username, active_launch_auth) else {
        notification::warn!(
            "home/quick_launch",
            "Sign in before copying an instance command line."
        );
        return;
    };
    let command = build_quick_launch_command(
        QuickLaunchCommandMode::Pack,
        instance_id,
        user.as_str(),
        None,
        None,
    );
    ctx.copy_text(command);
    notification::info!(
        "home/quick_launch",
        "Copied instance command line to clipboard."
    );
}

fn copy_instance_steam_launch_options(
    ctx: &egui::Context,
    instance_id: &str,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
) {
    let Some(user) = selected_quick_launch_user(active_username, active_launch_auth) else {
        notification::warn!(
            "home/quick_launch",
            "Sign in before copying Steam launch options."
        );
        return;
    };
    let options = build_quick_launch_steam_options(
        QuickLaunchCommandMode::Pack,
        instance_id,
        user.as_str(),
        None,
        None,
    );
    ctx.copy_text(options);
    notification::info!(
        "home/quick_launch",
        "Copied Steam launch options to clipboard."
    );
}

fn copy_world_launch_command(
    ctx: &egui::Context,
    world: &WorldEntry,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
) {
    let Some(user) = selected_quick_launch_user(active_username, active_launch_auth) else {
        notification::warn!(
            "home/quick_launch",
            "Sign in before copying a world command line."
        );
        return;
    };
    let command = build_quick_launch_command(
        QuickLaunchCommandMode::World,
        world.instance_id.as_str(),
        user.as_str(),
        Some(world.world_id.as_str()),
        None,
    );
    ctx.copy_text(command);
    notification::info!(
        "home/quick_launch",
        "Copied world command line to clipboard."
    );
}

fn copy_world_steam_launch_options(
    ctx: &egui::Context,
    world: &WorldEntry,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
) {
    let Some(user) = selected_quick_launch_user(active_username, active_launch_auth) else {
        notification::warn!(
            "home/quick_launch",
            "Sign in before copying Steam launch options."
        );
        return;
    };
    let options = build_quick_launch_steam_options(
        QuickLaunchCommandMode::World,
        world.instance_id.as_str(),
        user.as_str(),
        Some(world.world_id.as_str()),
        None,
    );
    ctx.copy_text(options);
    notification::info!(
        "home/quick_launch",
        "Copied Steam launch options to clipboard."
    );
}

fn copy_server_launch_command(
    ctx: &egui::Context,
    server: &ServerEntry,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
) {
    let Some(user) = selected_quick_launch_user(active_username, active_launch_auth) else {
        notification::warn!(
            "home/quick_launch",
            "Sign in before copying a server command line."
        );
        return;
    };
    let command = build_quick_launch_command(
        QuickLaunchCommandMode::Server,
        server.instance_id.as_str(),
        user.as_str(),
        None,
        Some(server.address.as_str()),
    );
    ctx.copy_text(command);
    notification::info!(
        "home/quick_launch",
        "Copied server command line to clipboard."
    );
}

fn copy_server_steam_launch_options(
    ctx: &egui::Context,
    server: &ServerEntry,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
) {
    let Some(user) = selected_quick_launch_user(active_username, active_launch_auth) else {
        notification::warn!(
            "home/quick_launch",
            "Sign in before copying Steam launch options."
        );
        return;
    };
    let options = build_quick_launch_steam_options(
        QuickLaunchCommandMode::Server,
        server.instance_id.as_str(),
        user.as_str(),
        None,
        Some(server.address.as_str()),
    );
    ctx.copy_text(options);
    notification::info!(
        "home/quick_launch",
        "Copied Steam launch options to clipboard."
    );
}

fn render_clickable_entry_row(
    ui: &mut Ui,
    id_source: impl std::hash::Hash,
    height: f32,
    add_contents: impl FnOnce(&mut Ui),
) -> egui::Response {
    let width = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let response = ui.interact(
        rect,
        ui.make_persistent_id(&id_source),
        egui::Sense::click(),
    );
    let visuals = ui.visuals();
    let has_focus = response.has_focus();
    let fill = if response.is_pointer_button_down_on() {
        visuals.widgets.active.bg_fill
    } else if response.hovered() || has_focus {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.widgets.inactive.weak_bg_fill
    };
    let stroke = if has_focus {
        visuals.selection.stroke
    } else if response.hovered() {
        visuals.widgets.hovered.bg_stroke
    } else {
        visuals.widgets.inactive.bg_stroke
    };
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(8),
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    if has_focus {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            egui::CornerRadius::same(10),
            egui::Stroke::new(
                (visuals.selection.stroke.width + 1.0).max(2.0),
                visuals.selection.stroke.color,
            ),
            egui::StrokeKind::Outside,
        );
    }
    let inner = rect.shrink2(egui::vec2(
        ACTIVITY_ENTRY_ROW_HORIZONTAL_PADDING,
        ACTIVITY_ENTRY_ROW_VERTICAL_PADDING,
    ));
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(id_source)
            .max_rect(inner)
            .layout(Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let horizontal_clip = egui::Rect::from_min_max(
                egui::pos2(inner.min.x, rect.min.y - 8.0),
                egui::pos2(inner.max.x, rect.max.y + 13.0),
            );
            ui.set_clip_rect(horizontal_clip);
            add_contents(ui)
        },
    );
    response
}

fn render_entry_thumbnail(
    ui: &mut Ui,
    id_source: impl std::hash::Hash,
    image_png: Option<(String, Arc<[u8]>)>,
    icon_svg: &'static [u8],
    width: f32,
    height: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let visuals = ui.visuals();
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(6),
        visuals.selection.bg_fill.gamma_multiply(0.16),
        visuals.widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        if let Some((uri, png)) = image_png {
            let image_size = (height - 4.0).max(1.0).min(width - 4.0).max(1.0);
            let image_rect =
                egui::Rect::from_center_size(rect.center(), egui::vec2(image_size, image_size));
            if let image_textures::ManagedTextureStatus::Ready(texture) =
                image_textures::request_texture(ui.ctx(), uri, png, TextureOptions::LINEAR)
            {
                ui.put(
                    image_rect,
                    egui::Image::from_texture(&texture)
                        .fit_to_exact_size(egui::vec2(image_size, image_size)),
                );
                return;
            }
        }

        ui.with_layout(Layout::top_down(egui::Align::Center), |ui| {
            ui.add_space(((height - ENTRY_ICON_SIZE) * 0.5).max(0.0));
            let themed_svg = apply_color_to_svg(icon_svg, ui.visuals().text_color());
            let uri = format!("bytes://home/entry-thumb/{:?}.svg", ui.id().with(id_source));
            ui.add(
                egui::Image::from_bytes(uri, themed_svg)
                    .fit_to_exact_size(egui::vec2(ENTRY_ICON_SIZE, ENTRY_ICON_SIZE)),
            );
        });
    });
}

fn render_favorite_star_button(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + Copy,
    active: bool,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(FAVORITE_STAR_BUTTON_SIZE, FAVORITE_STAR_BUTTON_SIZE),
        egui::Sense::hover(),
    );
    let response = ui.interact(rect, ui.make_persistent_id(id_source), egui::Sense::click());
    let has_focus = response.has_focus();
    let star_fill = if active {
        ui.visuals().warn_fg_color
    } else {
        ui.visuals().extreme_bg_color
    };
    let star_outline = ui.visuals().widgets.active.bg_stroke.color;
    let themed_svg = apply_star_fill_and_stroke_svg(assets::STAR_SVG, star_fill, star_outline);
    let uri = format!(
        "bytes://home/favorite-star/{:?}-{:02x}{:02x}{:02x}-{:02x}{:02x}{:02x}.svg",
        ui.id().with((id_source, active)),
        star_fill.r(),
        star_fill.g(),
        star_fill.b(),
        star_outline.r(),
        star_outline.g(),
        star_outline.b()
    );
    let icon_rect = egui::Rect::from_center_size(
        rect.center(),
        egui::vec2(FAVORITE_STAR_ICON_SIZE, FAVORITE_STAR_ICON_SIZE),
    );
    let button_fill = if response.is_pointer_button_down_on() {
        ui.visuals().widgets.active.bg_fill
    } else if response.hovered() || has_focus {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(6), button_fill);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(6),
        if has_focus {
            ui.visuals().selection.stroke
        } else {
            ui.visuals().widgets.inactive.bg_stroke
        },
        egui::StrokeKind::Inside,
    );
    if has_focus {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            egui::CornerRadius::same(8),
            egui::Stroke::new(
                (ui.visuals().selection.stroke.width + 1.0).max(2.0),
                ui.visuals().selection.stroke.color,
            ),
            egui::StrokeKind::Outside,
        );
    }
    let mut icon_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(icon_rect)
            .layout(Layout::left_to_right(egui::Align::Center)),
    );
    icon_ui.add(
        egui::Image::from_bytes(uri, themed_svg)
            .fit_to_exact_size(icon_rect.size())
            .sense(egui::Sense::hover()),
    );

    response
}

fn modal_default_focus_requested(ctx: &egui::Context, id_source: impl std::hash::Hash) -> bool {
    let key = egui::Id::new(("modal_default_focus_frame", id_source));
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| {
        let last_seen = data.get_temp::<u64>(key);
        data.insert_temp(key, frame);
        !matches!(last_seen, Some(previous) if previous.saturating_add(1) >= frame)
    })
}

fn apply_color_to_svg(svg_bytes: &[u8], color: Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", color_hex.as_str());
    svg.into_bytes()
}

fn apply_star_fill_and_stroke_svg(svg_bytes: &[u8], fill: Color32, stroke: Color32) -> Vec<u8> {
    let fill_hex = format!("#{:02x}{:02x}{:02x}", fill.r(), fill.g(), fill.b());
    let stroke_hex = format!("#{:02x}{:02x}{:02x}", stroke.r(), stroke.g(), stroke.b());
    let mut svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", fill_hex.as_str());
    svg = svg.replace(
        "<path d=\"M8.243",
        format!(
            "<path fill=\"{}\" stroke=\"{}\" stroke-width=\"2.0\" stroke-linejoin=\"round\" paint-order=\"stroke\" d=\"M8.243",
            fill_hex, stroke_hex
        )
        .as_str(),
    );
    svg.into_bytes()
}

fn refresh_home_state(
    state: &mut HomeState,
    instances: &InstanceStore,
    config: &Config,
    force: bool,
) {
    if state.activity_scan_pending && !force {
        queue_server_pings(state);
        return;
    }

    let request_id = state.latest_requested_activity_scan_id.saturating_add(1);
    let request = build_home_activity_scan_request(instances, config);
    let Ok(channel) = home_activity_results().lock() else {
        tracing::error!(
            target: "vertexlauncher/home",
            request_id,
            scanned_instance_count = request.scanned_instance_count,
            "Home activity results channel mutex was poisoned while scheduling a home activity scan."
        );
        return;
    };
    let result_tx = channel.tx.clone();
    drop(channel);

    state.latest_requested_activity_scan_id = request_id;
    state.activity_scan_pending = true;
    tokio_runtime::spawn_detached(async move {
        let scanned_instance_count = request.scanned_instance_count;
        let result = tokio_runtime::spawn_blocking(move || HomeActivityScanResult {
            request_id,
            scanned_instance_count,
            worlds: collect_worlds_from_request(&request),
            servers: collect_servers_from_request(&request),
        })
        .await;
        let Ok(result) = result else {
            tracing::error!(
                target: "vertexlauncher/home",
                request_id,
                "Failed to complete home activity scan task."
            );
            return;
        };
        if let Err(err) = result_tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/home",
                request_id,
                error = %err,
                "Failed to deliver home activity scan result."
            );
        }
    });
}

fn collect_worlds_from_request(request: &HomeActivityScanRequest) -> Vec<WorldEntry> {
    let mut worlds = Vec::new();
    for instance in &request.instances {
        let saves_dir = instance.instance_root.join("saves");
        let Ok(entries) = fs::read_dir(saves_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let world_id = entry.file_name().to_string_lossy().to_string();
            if world_id.trim().is_empty() {
                continue;
            }
            let level_dat_path = path.join("level.dat");
            let metadata = parse_world_metadata(level_dat_path.as_path()).unwrap_or_default();
            let world_name = metadata
                .level_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(world_id.as_str())
                .to_owned();
            let last_used_at_ms = metadata
                .last_played_ms
                .or_else(|| modified_millis(level_dat_path.as_path()))
                .or_else(|| modified_millis(path.as_path()));
            worlds.push(WorldEntry {
                instance_id: instance.instance_id.clone(),
                instance_name: instance.instance_name.clone(),
                world_id: world_id.clone(),
                world_name,
                game_mode: metadata.game_mode,
                hardcore: metadata.hardcore,
                cheats_enabled: metadata.cheats_enabled,
                difficulty: metadata.difficulty,
                version_name: metadata.version_name,
                thumbnail_png: read_world_thumbnail(path.join("icon.png").as_path()),
                last_used_at_ms,
                favorite: instance.favorite_world_ids.iter().any(|id| id == &world_id),
            });
        }
    }
    worlds.sort_by(|a, b| {
        b.last_used_at_ms
            .unwrap_or(0)
            .cmp(&a.last_used_at_ms.unwrap_or(0))
            .then_with(|| a.world_name.cmp(&b.world_name))
    });
    worlds
}

fn read_world_thumbnail(path: &Path) -> Option<Arc<[u8]>> {
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    // Guard against unexpectedly large files in world folders.
    if bytes.len() > 4 * 1024 * 1024 {
        return None;
    }
    Some(prepare_owned_image_bytes_for_memory(bytes))
}
