use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use config::Config;
use egui::{Color32, Layout, Ui};
use flate2::read::GzDecoder;
use instances::{InstanceStore, instance_root_path, set_server_favorite, set_world_favorite};
use launcher_runtime as tokio_runtime;
use textui::{
    LabelOptions, TextUi,
    truncate_single_line_text_with_ellipsis_preserving_whitespace as truncate_for_width,
};

use crate::{
    assets, desktop, notification,
    ui::{
        components::lazy_image_bytes::{LazyImageBytes, LazyImageBytesStatus},
        instance_context_menu::{self, InstanceContextAction},
        modal, style,
    },
};

use super::{AppScreen, PendingLaunchIntent, queue_launch_intent};

const HOME_SCAN_INTERVAL: Duration = Duration::from_secs(3);
const SERVER_PING_REFRESH_INTERVAL: Duration = Duration::from_secs(20);
const SERVER_PING_CONNECT_TIMEOUT: Duration = Duration::from_millis(350);
const SERVER_PINGS_PER_SCAN: usize = 3;
const INSTANCE_ROW_HEIGHT: f32 = 34.0;
const ACTIVITY_ROW_HEIGHT: f32 = 54.0;
const ENTRY_ICON_SIZE: f32 = 14.0;
const SERVER_PING_ICON_SIZE: f32 = 24.0;
const FAVORITE_STAR_BUTTON_SIZE: f32 = 20.0;
const FAVORITE_STAR_ICON_SIZE: f32 = 14.0;
const SCREENSHOT_SCAN_INTERVAL: Duration = Duration::from_secs(10);
const SCREENSHOT_PAGE_SIZE: usize = 30;
const HOME_TAB_HEIGHT: f32 = 38.0;
const SCREENSHOT_TILE_GAP: f32 = 10.0;
const SCREENSHOT_PRELOAD_VIEWPORTS: f32 = 0.75;
const SCREENSHOT_VIEWER_MIN_ZOOM: f32 = 1.0;
const SCREENSHOT_VIEWER_MAX_ZOOM: f32 = 8.0;
const SCREENSHOT_VIEWER_ZOOM_STEP: f32 = 0.2;
const SCREENSHOT_COPY_BUTTON_SIZE: f32 = 28.0;

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
    instance_thumbnail_results_tx: Option<mpsc::Sender<(String, Option<Arc<[u8]>>)>>,
    instance_thumbnail_results_rx: Option<Arc<Mutex<mpsc::Receiver<(String, Option<Arc<[u8]>>)>>>>,
    instance_thumbnail_cache: HashMap<String, Option<Arc<[u8]>>>,
    instance_thumbnail_in_flight: HashSet<String>,
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
    thumbnail_png: Option<Vec<u8>>,
    last_used_at_ms: Option<u64>,
    favorite: bool,
}

#[derive(Debug, Clone)]
struct ServerEntry {
    instance_id: String,
    instance_name: String,
    server_name: String,
    address: String,
    favorite_id: String,
    host: String,
    port: u16,
    icon_png: Option<Vec<u8>>,
    last_used_at_ms: Option<u64>,
    favorite: bool,
}

#[derive(Debug, Clone, Copy)]
enum ServerPingStatus {
    Unknown,
    Offline,
    Online { latency_ms: u64 },
}

#[derive(Debug, Clone)]
struct ServerPingSnapshot {
    status: ServerPingStatus,
    motd: Option<String>,
    players_online: Option<u32>,
    players_max: Option<u32>,
    checked_at: Instant,
}

#[derive(Debug, Clone)]
struct ServerDatEntry {
    name: String,
    ip: String,
    icon: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct WorldMetadata {
    level_name: Option<String>,
    game_mode: Option<String>,
    hardcore: Option<bool>,
    cheats_enabled: Option<bool>,
    difficulty: Option<String>,
    version_name: Option<String>,
    last_played_ms: Option<u64>,
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

#[derive(Debug, Clone)]
struct ScreenshotEntry {
    instance_name: String,
    path: PathBuf,
    file_name: String,
    width: u32,
    height: u32,
    modified_at_ms: Option<u64>,
}

impl ScreenshotEntry {
    fn key(&self) -> String {
        self.path.to_string_lossy().to_string()
    }

    fn uri(&self) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.path.hash(&mut hasher);
        self.modified_at_ms.hash(&mut hasher);
        format!("bytes://home/screenshot/{}.png", hasher.finish())
    }

    fn aspect_ratio(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }
}

#[derive(Debug, Clone)]
struct ScreenshotViewerState {
    screenshot_key: String,
    zoom: f32,
    pan_uv: egui::Vec2,
}

#[derive(Debug, Clone, Copy, Default)]
struct ScreenshotTileAction {
    open_viewer: bool,
    request_delete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScreenshotOverlayAction {
    Copy,
    Delete,
}

#[derive(Debug, Clone, Copy, Default)]
struct ScreenshotOverlayResult {
    action: Option<ScreenshotOverlayAction>,
    contains_pointer: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct ScreenshotOverlayButtonResult {
    clicked: bool,
    contains_pointer: bool,
}

#[derive(Debug, Clone)]
struct ScreenshotCandidate {
    instance_name: String,
    path: PathBuf,
    file_name: String,
    modified_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct ScreenshotScanRequest {
    scanned_instance_count: usize,
    instances: Vec<ScreenshotScanInstance>,
}

#[derive(Debug, Clone)]
struct ScreenshotScanInstance {
    instance_name: String,
    screenshots_dir: PathBuf,
}

enum ScreenshotScanMessage {
    /// Sent by each per-file task the moment image dimensions are validated.
    EntryLoaded {
        request_id: u64,
        entry: ScreenshotEntry,
    },
    /// Sent by each per-file task when it finishes, whether or not it produced an entry.
    TaskDone { request_id: u64 },
}

struct ScreenshotResultChannel {
    tx: mpsc::Sender<ScreenshotScanMessage>,
    rx: mpsc::Receiver<ScreenshotScanMessage>,
}

struct HomeActivityResultChannel {
    tx: mpsc::Sender<HomeActivityScanResult>,
    rx: mpsc::Receiver<HomeActivityScanResult>,
}

#[derive(Debug, Clone)]
struct ServerPingResult {
    address: String,
    snapshot: ServerPingSnapshot,
}

struct ServerPingResultChannel {
    tx: mpsc::Sender<ServerPingResult>,
    rx: mpsc::Receiver<ServerPingResult>,
}

static HOME_ACTIVITY_RESULTS: OnceLock<Mutex<HomeActivityResultChannel>> = OnceLock::new();
static SCREENSHOT_RESULTS: OnceLock<Mutex<ScreenshotResultChannel>> = OnceLock::new();
static SERVER_PING_RESULTS: OnceLock<Mutex<ServerPingResultChannel>> = OnceLock::new();

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

fn home_activity_results() -> &'static Mutex<HomeActivityResultChannel> {
    HOME_ACTIVITY_RESULTS.get_or_init(|| {
        let (result_tx, result_rx) = mpsc::channel::<HomeActivityScanResult>();
        Mutex::new(HomeActivityResultChannel {
            rx: result_rx,
            tx: result_tx,
        })
    })
}

fn screenshot_results() -> &'static Mutex<ScreenshotResultChannel> {
    SCREENSHOT_RESULTS.get_or_init(|| {
        let (result_tx, result_rx) = mpsc::channel::<ScreenshotScanMessage>();
        Mutex::new(ScreenshotResultChannel {
            rx: result_rx,
            tx: result_tx,
        })
    })
}

fn server_ping_results() -> &'static Mutex<ServerPingResultChannel> {
    SERVER_PING_RESULTS.get_or_init(|| {
        let (result_tx, result_rx) = mpsc::channel::<ServerPingResult>();
        Mutex::new(ServerPingResultChannel {
            rx: result_rx,
            tx: result_tx,
        })
    })
}

fn build_home_activity_scan_request(
    instances: &InstanceStore,
    config: &Config,
) -> HomeActivityScanRequest {
    let installations_root = PathBuf::from(config.minecraft_installations_root());
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

fn build_screenshot_scan_request(
    instances: &InstanceStore,
    config: &Config,
) -> ScreenshotScanRequest {
    let installations_root = PathBuf::from(config.minecraft_installations_root());
    let screenshot_instances = instances
        .instances
        .iter()
        .map(|instance| ScreenshotScanInstance {
            instance_name: instance.name.clone(),
            screenshots_dir: instance_root_path(installations_root.as_path(), instance)
                .join("screenshots"),
        })
        .collect();

    ScreenshotScanRequest {
        scanned_instance_count: instances.instances.len(),
        instances: screenshot_instances,
    }
}

fn poll_home_activity_results(state: &mut HomeState) {
    let Ok(channel) = home_activity_results().lock() else {
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

fn poll_screenshot_results(state: &mut HomeState) {
    let Ok(channel) = screenshot_results().lock() else {
        return;
    };

    let mut messages = Vec::new();
    while let Ok(msg) = channel.rx.try_recv() {
        messages.push(msg);
    }
    drop(channel);

    let mut any_entries = false;
    for msg in messages {
        match msg {
            ScreenshotScanMessage::EntryLoaded { request_id, entry } => {
                if request_id != state.latest_requested_screenshot_scan_id {
                    continue;
                }
                state.screenshots.push(entry);
                any_entries = true;
            }
            ScreenshotScanMessage::TaskDone { request_id } => {
                if request_id != state.latest_requested_screenshot_scan_id {
                    continue;
                }
                state.screenshot_tasks_done += 1;
                let all_pages_spawned =
                    state.screenshot_loaded_count >= state.screenshot_candidates.len();
                if state.screenshot_scan_ready
                    && all_pages_spawned
                    && state.screenshot_tasks_done >= state.screenshot_tasks_total
                {
                    state.screenshot_scan_pending = false;
                    state.last_screenshot_scan_at = Some(Instant::now());
                }
            }
        }
    }

    // Sort once per poll cycle rather than after every individual entry.
    if any_entries {
        state.screenshots.sort_by(|a, b| {
            b.modified_at_ms
                .unwrap_or(0)
                .cmp(&a.modified_at_ms.unwrap_or(0))
                .then_with(|| a.file_name.cmp(&b.file_name))
        });
    }
}

fn spawn_screenshot_load_page(state: &mut HomeState, request_id: u64, page_size: usize) {
    let start = state.screenshot_loaded_count;
    let end = (start + page_size).min(state.screenshot_candidates.len());
    if start >= end {
        return;
    }

    let Ok(channel) = screenshot_results().lock() else {
        return;
    };
    let result_tx = channel.tx.clone();
    drop(channel);

    let page: Vec<ScreenshotCandidate> = state.screenshot_candidates[start..end].to_vec();
    state.screenshot_loaded_count = end;
    state.screenshot_tasks_total += page.len();

    for candidate in page {
        let tx = result_tx.clone();
        let _ = tokio_runtime::spawn(async move {
            let entry = (|| {
                let Ok((width, height)) = image::image_dimensions(&candidate.path) else {
                    return None;
                };
                if width == 0 || height == 0 {
                    return None;
                }
                Some(ScreenshotEntry {
                    instance_name: candidate.instance_name,
                    path: candidate.path,
                    file_name: candidate.file_name,
                    width,
                    height,
                    modified_at_ms: candidate.modified_at_ms,
                })
            })();
            if let Some(entry) = entry {
                let _ = tx.send(ScreenshotScanMessage::EntryLoaded { request_id, entry });
            }
            let _ = tx.send(ScreenshotScanMessage::TaskDone { request_id });
        });
    }
}

fn poll_server_ping_results(state: &mut HomeState) {
    let Ok(channel) = server_ping_results().lock() else {
        return;
    };

    while let Ok(result) = channel.rx.try_recv() {
        state.server_ping_in_flight.remove(result.address.as_str());
        state.server_pings.insert(result.address, result.snapshot);
    }
}

fn ensure_delete_screenshot_channel(state: &mut HomeState) {
    if state.delete_screenshot_results_tx.is_some() && state.delete_screenshot_results_rx.is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, String, Result<(), String>)>();
    state.delete_screenshot_results_tx = Some(tx);
    state.delete_screenshot_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_screenshot_delete(
    state: &mut HomeState,
    screenshot_key: String,
    path: PathBuf,
    file_name: String,
) {
    if state.delete_screenshot_in_flight {
        return;
    }

    ensure_delete_screenshot_channel(state);
    let Some(tx) = state.delete_screenshot_results_tx.as_ref().cloned() else {
        return;
    };

    state.delete_screenshot_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = fs::remove_file(path.as_path()).map_err(|err| err.to_string());
        let _ = tx.send((screenshot_key, file_name, result));
    });
}

fn poll_delete_screenshot_results(
    state: &mut HomeState,
    instances: &InstanceStore,
    config: &Config,
) {
    let Some(rx) = state.delete_screenshot_results_rx.as_ref() else {
        return;
    };

    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    match rx.lock() {
        Ok(receiver) => loop {
            match receiver.try_recv() {
                Ok(update) => updates.push(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    should_reset_channel = true;
                    break;
                }
            }
        },
        Err(_) => should_reset_channel = true,
    }

    if should_reset_channel {
        state.delete_screenshot_results_tx = None;
        state.delete_screenshot_results_rx = None;
        state.delete_screenshot_in_flight = false;
    }

    for (screenshot_key, file_name, result) in updates {
        state.delete_screenshot_in_flight = false;
        match result {
            Ok(()) => {
                if state
                    .screenshot_viewer
                    .as_ref()
                    .is_some_and(|viewer| viewer.screenshot_key == screenshot_key)
                {
                    state.screenshot_viewer = None;
                }
                state.pending_delete_screenshot_key = None;
                refresh_screenshot_state(state, instances, config, true);
                notification::info!("home/screenshots", "Deleted '{}' from disk.", file_name);
            }
            Err(err) => {
                state.pending_delete_screenshot_key = None;
                notification::error!(
                    "home/screenshots",
                    "Failed to delete '{}': {}",
                    file_name,
                    err
                );
            }
        }
    }
}

pub(super) fn handle_escape(ctx: &egui::Context) -> bool {
    let state_id = home_state_id();
    let mut handled = false;
    ctx.data_mut(|data| {
        let Some(mut state) = data.get_temp::<HomeState>(state_id) else {
            return;
        };
        if state.pending_delete_screenshot_key.is_some() {
            if !state.delete_screenshot_in_flight {
                state.pending_delete_screenshot_key = None;
            }
            data.insert_temp(state_id, state);
            handled = true;
            return;
        }
        if state.screenshot_viewer.take().is_some() {
            data.insert_temp(state_id, state);
            handled = true;
        }
    });
    handled
}

pub fn presence_section(ctx: &egui::Context) -> HomePresenceSection {
    let state_id = home_state_id();
    let state = ctx.data_mut(|data| data.get_temp::<HomeState>(state_id));
    state
        .map(|state| state.active_tab.presence_section())
        .unwrap_or(HomePresenceSection::Activity)
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instances: &mut InstanceStore,
    config: &Config,
    streamer_mode: bool,
) -> HomeOutput {
    let mut output = HomeOutput::default();
    let state_id = home_state_id();
    let mut state = ui
        .ctx()
        .data_mut(|data| data.get_temp::<HomeState>(state_id))
        .unwrap_or_default();
    ui.ctx().request_repaint_after(Duration::from_millis(250));
    let screenshot_images_updated = state.screenshot_images.poll();

    render_home_tab_row(ui, &mut state.active_tab);
    ui.add_space(14.0);

    match state.active_tab {
        HomeTab::InstancesAndWorlds => {
            poll_home_activity_results(&mut state);
            poll_server_ping_results(&mut state);
            poll_instance_thumbnail_results(&mut state);
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
            if !state.instance_thumbnail_in_flight.is_empty() {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }

            let mut requested_rescan = false;
            render_instance_usage(ui, text_ui, instances, config, &mut state, &mut output);
            ui.add_space(12.0);
            render_activity_feed(
                ui,
                text_ui,
                instances,
                &state,
                streamer_mode,
                &mut output,
                &mut requested_rescan,
            );

            if requested_rescan {
                refresh_home_state(&mut state, instances, config, true);
            }

            let mut retained_image_keys = HashSet::new();
            retain_home_viewer_image(&mut state, &mut retained_image_keys);
            state.screenshot_images.retain_loaded(&retained_image_keys);
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
            }) {
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
            render_screenshot_gallery(ui, text_ui, &mut state, &mut retained_image_keys);
            retain_home_viewer_image(&mut state, &mut retained_image_keys);
            state.screenshot_images.retain_loaded(&retained_image_keys);
        }
    }

    if screenshot_images_updated
        || (state.screenshot_images.has_in_flight()
            && (state.active_tab == HomeTab::Screenshots || state.screenshot_viewer.is_some()))
    {
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }

    render_screenshot_viewer_modal(ui.ctx(), text_ui, &mut state);
    render_delete_screenshot_modal(ui.ctx(), text_ui, &mut state, instances, config);
    output.presence_section = state.active_tab.presence_section();
    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
    output
}

fn render_home_tab_row(ui: &mut Ui, active_tab: &mut HomeTab) {
    let button_gap = style::SPACE_MD;
    let tab_count = 2.0;
    let button_width =
        ((ui.available_width() - button_gap * (tab_count - 1.0)) / tab_count).max(0.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = button_gap;
        for tab in [HomeTab::InstancesAndWorlds, HomeTab::Screenshots] {
            let selected = *active_tab == tab;
            let button =
                egui::Button::new(egui::RichText::new(tab.label()).size(15.0).strong().color(
                    if selected {
                        ui.visuals().widgets.active.fg_stroke.color
                    } else {
                        ui.visuals().text_color()
                    },
                ))
                .min_size(egui::vec2(button_width, HOME_TAB_HEIGHT))
                .fill(if selected {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().widgets.inactive.bg_fill
                })
                .stroke(if selected {
                    ui.visuals().selection.stroke
                } else {
                    ui.visuals().widgets.inactive.bg_stroke
                })
                .corner_radius(egui::CornerRadius::same(10));
            if ui
                .add_sized([button_width, HOME_TAB_HEIGHT], button)
                .clicked()
            {
                *active_tab = tab;
            }
        }
    });
}

fn render_screenshot_gallery(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut HomeState,
    retained_image_keys: &mut HashSet<String>,
) {
    let title_style = style::heading(ui, 18.0, 24.0);
    let body_style = style::muted(ui);
    let _ = text_ui.label(ui, "home_screenshots_title", "Screenshots", &title_style);
    let total_candidates = state.screenshot_candidates.len();
    let loaded_count = state.screenshots.len();
    let summary = if state.screenshot_scan_pending && state.screenshots.is_empty() {
        "Loading screenshots...".to_owned()
    } else if state.screenshots.is_empty() {
        "No screenshots found in any instance.".to_owned()
    } else if state.screenshot_loaded_count < total_candidates {
        format!(
            "Showing {loaded_count} of {total_candidates} screenshots — scroll down to load more."
        )
    } else {
        format!("{loaded_count} screenshots across your instances.")
    };
    let _ = text_ui.label(
        ui,
        "home_screenshots_summary",
        summary.as_str(),
        &body_style,
    );
    ui.add_space(style::SPACE_SM);

    if state.screenshots.is_empty() {
        return;
    }

    let available_width = ui.available_width().max(1.0);
    let column_count: usize = if available_width >= 980.0 {
        3
    } else if available_width >= 620.0 {
        2
    } else {
        1
    };
    let column_width = ((available_width
        - SCREENSHOT_TILE_GAP * (column_count.saturating_sub(1) as f32))
        / column_count as f32)
        .max(180.0);
    let assignments = screenshot_mosaic_assignments(&state.screenshots, column_count, column_width);

    let mut open_screenshot_key = None;
    let mut delete_screenshot_key = None;
    let scroll_output = egui::ScrollArea::vertical()
        .id_salt("home_screenshots_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(style::SPACE_SM);
            ui.columns(column_count, |columns| {
                for (column_ui, items) in columns.iter_mut().zip(assignments.iter()) {
                    column_ui.spacing_mut().item_spacing.y = SCREENSHOT_TILE_GAP;
                    let preload_rect = screenshot_preload_rect(column_ui);
                    for &(index, tile_height) in items {
                        let screenshot = state.screenshots[index].clone();
                        let action = render_screenshot_tile(
                            column_ui,
                            &mut state.screenshot_images,
                            &screenshot,
                            tile_height,
                            preload_rect,
                            retained_image_keys,
                        );
                        if action.open_viewer {
                            open_screenshot_key = Some(screenshot.key());
                        }
                        if action.request_delete {
                            delete_screenshot_key = Some(screenshot.key());
                        }
                    }
                }
            });
        });

    // Trigger next page when the user scrolls near the bottom.
    let has_more = state.screenshot_loaded_count < state.screenshot_candidates.len();
    if has_more {
        let visible_height = scroll_output.inner_rect.height();
        let near_bottom = scroll_output.state.offset.y + visible_height
            >= scroll_output.content_size.y - visible_height;
        if near_bottom {
            let request_id = state.latest_requested_screenshot_scan_id;
            spawn_screenshot_load_page(state, request_id, SCREENSHOT_PAGE_SIZE);
        }
    }

    if let Some(screenshot_key) = open_screenshot_key {
        state.screenshot_viewer = Some(ScreenshotViewerState {
            screenshot_key,
            zoom: SCREENSHOT_VIEWER_MIN_ZOOM,
            pan_uv: egui::Vec2::ZERO,
        });
    }
    if let Some(screenshot_key) = delete_screenshot_key {
        state.pending_delete_screenshot_key = Some(screenshot_key);
    }
}

fn screenshot_mosaic_assignments(
    screenshots: &[ScreenshotEntry],
    column_count: usize,
    column_width: f32,
) -> Vec<Vec<(usize, f32)>> {
    let mut assignments = vec![Vec::new(); column_count];
    let mut column_heights = vec![0.0; column_count];
    for (index, screenshot) in screenshots.iter().enumerate() {
        let tile_height = screenshot_tile_height(screenshot, column_width, index);
        let column_index = column_heights
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(index, _)| index)
            .unwrap_or(0);
        assignments[column_index].push((index, tile_height));
        column_heights[column_index] += tile_height + SCREENSHOT_TILE_GAP;
    }
    assignments
}

fn screenshot_tile_height(screenshot: &ScreenshotEntry, column_width: f32, _index: usize) -> f32 {
    column_width / screenshot.aspect_ratio().max(0.01)
}

fn render_screenshot_tile(
    ui: &mut Ui,
    screenshot_images: &mut LazyImageBytes,
    screenshot: &ScreenshotEntry,
    tile_height: f32,
    preload_rect: egui::Rect,
    retained_image_keys: &mut HashSet<String>,
) -> ScreenshotTileAction {
    let width = ui.available_width().max(1.0);
    let tile_size = egui::vec2(width, tile_height);
    let (rect, _) = ui.allocate_exact_size(tile_size, egui::Sense::hover());
    let mut image_response = ui.interact(
        rect,
        ui.id().with(("home_screenshot_tile", screenshot.key())),
        egui::Sense::click(),
    );
    let image_key = screenshot.uri();
    let should_preload = rect_intersects(rect, preload_rect);
    let image_status = if should_preload {
        retained_image_keys.insert(image_key.clone());
        screenshot_images.request(image_key.clone(), screenshot.path.clone())
    } else {
        screenshot_images.status(image_key.as_str())
    };
    let image_bytes = screenshot_images.bytes(image_key.as_str());
    if let Some(bytes) = image_bytes.as_ref() {
        egui::Image::from_bytes(image_key, Arc::clone(bytes))
            .fit_to_exact_size(rect.size())
            .corner_radius(egui::CornerRadius::same(14))
            .paint_at(ui, rect);
    } else {
        paint_screenshot_tile_placeholder(ui, rect, image_status);
    }

    let tile_contains_pointer = ui_pointer_over_rect(ui, rect);
    let overlay_memory_id = image_response.id.with("home_screenshot_overlay_active");
    let overlay_was_active = ui
        .ctx()
        .data_mut(|data| data.get_temp::<bool>(overlay_memory_id))
        .unwrap_or(false);
    let mut overlay_clicked = false;
    let mut action = ScreenshotTileAction::default();
    let mut overlay_result = ScreenshotOverlayResult::default();
    if tile_contains_pointer || overlay_was_active {
        overlay_result = render_screenshot_overlay_action(
            ui,
            rect,
            "home_gallery",
            screenshot,
            image_bytes.as_deref(),
            image_status == LazyImageBytesStatus::Loading,
        );
        match overlay_result.action {
            Some(ScreenshotOverlayAction::Copy) => {
                overlay_clicked = true;
            }
            Some(ScreenshotOverlayAction::Delete) => {
                overlay_clicked = true;
                action.request_delete = true;
            }
            None => {}
        }
    }
    let overlay_active = tile_contains_pointer || overlay_result.contains_pointer;
    ui.ctx()
        .data_mut(|data| data.insert_temp(overlay_memory_id, overlay_active));
    let stroke = if overlay_active {
        ui.visuals().widgets.hovered.bg_stroke
    } else {
        ui.visuals().widgets.inactive.bg_stroke
    };
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(14),
        stroke,
        egui::StrokeKind::Inside,
    );

    let label_bg_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 10.0, rect.max.y - 34.0),
        egui::pos2(rect.max.x - 10.0, rect.max.y - 10.0),
    );
    ui.painter().rect_filled(
        label_bg_rect,
        egui::CornerRadius::same(8),
        Color32::from_rgba_premultiplied(6, 9, 14, 185),
    );
    let age_label = format_time_ago(screenshot.modified_at_ms, current_time_millis());
    ui.painter().text(
        egui::pos2(label_bg_rect.min.x + 8.0, label_bg_rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!("{} | {}", screenshot.instance_name, age_label),
        egui::TextStyle::Body.resolve(ui.style()),
        Color32::WHITE,
    );

    image_response = image_response.on_hover_text(format!(
        "{}\n{}\n{}",
        screenshot.instance_name,
        screenshot.file_name,
        screenshot.path.display()
    ));
    action.open_viewer = image_response.clicked() && !overlay_clicked;
    action
}

fn screenshot_preload_rect(ui: &Ui) -> egui::Rect {
    let clip_rect = ui.clip_rect();
    let margin = (clip_rect.height() * SCREENSHOT_PRELOAD_VIEWPORTS).max(220.0);
    egui::Rect::from_min_max(
        egui::pos2(clip_rect.min.x, clip_rect.min.y - margin),
        egui::pos2(clip_rect.max.x, clip_rect.max.y + margin),
    )
}

fn rect_intersects(left: egui::Rect, right: egui::Rect) -> bool {
    left.min.x <= right.max.x
        && left.max.x >= right.min.x
        && left.min.y <= right.max.y
        && left.max.y >= right.min.y
}

fn ui_pointer_over_rect(ui: &Ui, rect: egui::Rect) -> bool {
    ui.input(|input| {
        input
            .pointer
            .interact_pos()
            .or_else(|| input.pointer.hover_pos())
            .is_some_and(|pointer_pos| rect.contains(pointer_pos))
    })
}

fn paint_screenshot_tile_placeholder(
    ui: &mut Ui,
    rect: egui::Rect,
    image_status: LazyImageBytesStatus,
) {
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(14),
        ui.visuals().widgets.inactive.bg_fill,
    );
    let label = match image_status {
        LazyImageBytesStatus::Loading => "Loading...",
        LazyImageBytesStatus::Failed => "Failed to load",
        LazyImageBytesStatus::Ready | LazyImageBytesStatus::Unrequested => "Waiting...",
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        ui.visuals().weak_text_color(),
    );
}

fn retain_home_viewer_image(state: &mut HomeState, retained_image_keys: &mut HashSet<String>) {
    let Some(viewer_key) = state
        .screenshot_viewer
        .as_ref()
        .map(|viewer| viewer.screenshot_key.as_str())
    else {
        return;
    };
    let Some(screenshot) = state
        .screenshots
        .iter()
        .find(|entry| entry.key() == viewer_key)
        .cloned()
    else {
        return;
    };
    let image_key = screenshot.uri();
    retained_image_keys.insert(image_key.clone());
    state
        .screenshot_images
        .request(image_key, screenshot.path.clone());
}

fn render_screenshot_viewer_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut HomeState,
) {
    let Some(screenshot_key) = state
        .screenshot_viewer
        .as_ref()
        .map(|viewer_state| viewer_state.screenshot_key.clone())
    else {
        return;
    };
    let Some(screenshot) = state
        .screenshots
        .iter()
        .find(|entry| entry.key() == screenshot_key)
        .cloned()
    else {
        state.screenshot_viewer = None;
        return;
    };
    let Some(viewer_state) = state.screenshot_viewer.as_mut() else {
        return;
    };
    let image_key = screenshot.uri();
    let image_status = state
        .screenshot_images
        .request(image_key.clone(), screenshot.path.clone());
    let image_bytes = state.screenshot_images.bytes(image_key.as_str());

    let viewport_rect = ctx.input(|i| i.content_rect());
    let modal_width = (viewport_rect.width() * 0.92).max(320.0);
    let modal_height = (viewport_rect.height() * 0.9).max(280.0);
    let modal_pos = egui::pos2(
        (viewport_rect.center().x - modal_width * 0.5)
            .clamp(viewport_rect.left(), viewport_rect.right() - modal_width),
        (viewport_rect.center().y - modal_height * 0.5)
            .clamp(viewport_rect.top(), viewport_rect.bottom() - modal_height),
    );
    let now_ms = current_time_millis();
    let mut open = true;
    let mut close_requested = false;
    let mut delete_requested = false;
    modal::show_scrim(ctx, "home_screenshot_viewer_scrim", viewport_rect);

    egui::Window::new("Screenshot Viewer")
        .id(egui::Id::new("home_screenshot_viewer_window"))
        .order(egui::Order::Foreground)
        .open(&mut open)
        .fixed_pos(modal_pos)
        .fixed_size(egui::vec2(modal_width, modal_height))
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .title_bar(false)
        .hscroll(false)
        .vscroll(false)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(modal::window_frame(ctx))
        .show(ctx, |ui| {
            let title_style = style::heading(ui, 24.0, 28.0);
            let body_style = style::muted_single_line(ui);

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let _ = text_ui.label(
                        ui,
                        "home_screenshot_viewer_title",
                        screenshot.file_name.as_str(),
                        &title_style,
                    );
                    let details = format!(
                        "{} | {}x{} | {}",
                        screenshot.instance_name,
                        screenshot.width,
                        screenshot.height,
                        format_time_ago(screenshot.modified_at_ms, now_ms)
                    );
                    let _ = text_ui.label(
                        ui,
                        "home_screenshot_viewer_details",
                        details.as_str(),
                        &body_style,
                    );
                });
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close_requested = true;
                    }
                    if ui.button("Delete").clicked() {
                        delete_requested = true;
                    }
                    if ui
                        .add_enabled(image_bytes.is_some(), egui::Button::new("Copy"))
                        .clicked()
                        && let Some(bytes) = image_bytes.as_deref()
                    {
                        copy_screenshot_to_clipboard(
                            ui.ctx(),
                            screenshot.file_name.as_str(),
                            bytes,
                        );
                    }
                    if ui.button("Reset").clicked() {
                        viewer_state.zoom = SCREENSHOT_VIEWER_MIN_ZOOM;
                        viewer_state.pan_uv = egui::Vec2::ZERO;
                    }
                    if ui.button("+").clicked() {
                        viewer_state.zoom = adjust_viewer_zoom(viewer_state.zoom, 1.0);
                        clamp_viewer_pan(viewer_state);
                    }
                    if ui.button("-").clicked() {
                        viewer_state.zoom = adjust_viewer_zoom(viewer_state.zoom, -1.0);
                        clamp_viewer_pan(viewer_state);
                    }
                });
            });
            ui.add_space(12.0);

            let canvas_size = ui.available_size().max(egui::vec2(1.0, 1.0));
            let (canvas_rect, response) = ui.allocate_exact_size(canvas_size, egui::Sense::drag());
            ui.painter().rect_filled(
                canvas_rect,
                egui::CornerRadius::same(12),
                ui.visuals().widgets.noninteractive.bg_fill,
            );

            let image_rect = fit_rect_to_aspect(canvas_rect.shrink(8.0), screenshot.aspect_ratio());
            if response.hovered() {
                let scroll_delta = ui.ctx().input(|input| input.smooth_scroll_delta.y);
                if scroll_delta.abs() > 0.0 {
                    viewer_state.zoom =
                        adjust_viewer_zoom(viewer_state.zoom, scroll_delta.signum());
                    clamp_viewer_pan(viewer_state);
                    ui.ctx().request_repaint();
                }
            }
            if response.dragged() && viewer_state.zoom > SCREENSHOT_VIEWER_MIN_ZOOM {
                let visible_fraction = 1.0 / viewer_state.zoom.max(SCREENSHOT_VIEWER_MIN_ZOOM);
                let delta = ui.ctx().input(|input| input.pointer.delta());
                viewer_state.pan_uv.x -= delta.x / image_rect.width().max(1.0) * visible_fraction;
                viewer_state.pan_uv.y -= delta.y / image_rect.height().max(1.0) * visible_fraction;
                clamp_viewer_pan(viewer_state);
                ui.ctx().request_repaint();
            }

            if let Some(bytes) = image_bytes.as_ref() {
                egui::Image::from_bytes(image_key, Arc::clone(bytes))
                    .fit_to_exact_size(image_rect.size())
                    .maintain_aspect_ratio(false)
                    .uv(viewer_uv_rect(viewer_state))
                    .paint_at(ui, image_rect);
            } else {
                ui.painter().rect_filled(
                    image_rect,
                    egui::CornerRadius::same(12),
                    ui.visuals().widgets.inactive.bg_fill,
                );
                let label = if image_status == LazyImageBytesStatus::Failed {
                    "Failed to load screenshot"
                } else {
                    "Loading screenshot..."
                };
                ui.painter().text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::TextStyle::Button.resolve(ui.style()),
                    ui.visuals().weak_text_color(),
                );
            }
            ui.painter().rect_stroke(
                image_rect,
                egui::CornerRadius::same(12),
                ui.visuals().widgets.inactive.bg_stroke,
                egui::StrokeKind::Inside,
            );
        });

    if delete_requested {
        state.pending_delete_screenshot_key = Some(screenshot_key);
    }
    if !open || close_requested {
        state.screenshot_viewer = None;
    }
}

fn render_delete_screenshot_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut HomeState,
    _instances: &InstanceStore,
    _config: &Config,
) {
    let Some(screenshot_key) = state.pending_delete_screenshot_key.clone() else {
        return;
    };
    let Some(screenshot) = state
        .screenshots
        .iter()
        .find(|entry| entry.key() == screenshot_key)
        .cloned()
    else {
        state.pending_delete_screenshot_key = None;
        return;
    };

    let viewport_rect = ctx.input(|input| input.content_rect());
    let modal_size = egui::vec2(viewport_rect.width().min(520.0), 260.0);
    let modal_pos = egui::pos2(
        (viewport_rect.center().x - modal_size.x * 0.5)
            .clamp(viewport_rect.left(), viewport_rect.right() - modal_size.x),
        (viewport_rect.center().y - modal_size.y * 0.5)
            .clamp(viewport_rect.top(), viewport_rect.bottom() - modal_size.y),
    );
    let danger = ctx.style().visuals.error_fg_color;
    let mut cancel_requested = false;
    let mut delete_requested = false;
    modal::show_scrim(ctx, "home_delete_screenshot_modal_scrim", viewport_rect);

    egui::Window::new("Delete Screenshot")
        .id(egui::Id::new("home_delete_screenshot_modal"))
        .order(egui::Order::Foreground)
        .fixed_pos(modal_pos)
        .fixed_size(modal_size)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(modal::window_frame(ctx))
        .show(ctx, |ui| {
            let heading_style = style::heading_color(ui, 28.0, 32.0, danger);
            let body_style = style::body(ui);
            let muted_style = style::muted(ui);

            let _ = text_ui.label(
                ui,
                (
                    "home_delete_screenshot_heading",
                    screenshot.path.display().to_string(),
                ),
                "Delete Screenshot?",
                &heading_style,
            );
            let _ = text_ui.label(
                ui,
                (
                    "home_delete_screenshot_body",
                    screenshot.path.display().to_string(),
                ),
                &format!(
                    "Delete \"{}\" from disk? This permanently removes the screenshot.",
                    screenshot.file_name
                ),
                &body_style,
            );
            let _ = text_ui.label(
                ui,
                (
                    "home_delete_screenshot_instance",
                    screenshot.path.display().to_string(),
                ),
                &format!("Instance: {}", screenshot.instance_name),
                &muted_style,
            );
            let _ = text_ui.label(
                ui,
                (
                    "home_delete_screenshot_path",
                    screenshot.path.display().to_string(),
                ),
                &format!("Path: {}", screenshot.path.display()),
                &muted_style,
            );

            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                let delete_button =
                    egui::Button::new(egui::RichText::new("Delete").color(egui::Color32::WHITE))
                        .fill(danger.gamma_multiply(0.84))
                        .stroke(egui::Stroke::new(1.0, danger))
                        .min_size(egui::vec2(120.0, 34.0))
                        .corner_radius(egui::CornerRadius::same(8));
                let cancel_button = egui::Button::new("Cancel")
                    .min_size(egui::vec2(120.0, 34.0))
                    .corner_radius(egui::CornerRadius::same(8));
                if ui
                    .add_enabled(!state.delete_screenshot_in_flight, delete_button)
                    .clicked()
                {
                    delete_requested = true;
                }
                if ui
                    .add_enabled(!state.delete_screenshot_in_flight, cancel_button)
                    .clicked()
                {
                    cancel_requested = true;
                }
                if state.delete_screenshot_in_flight {
                    ui.spinner();
                }
            });
        });

    if cancel_requested && !state.delete_screenshot_in_flight {
        state.pending_delete_screenshot_key = None;
        return;
    }

    if delete_requested {
        request_screenshot_delete(
            state,
            screenshot_key,
            screenshot.path.clone(),
            screenshot.file_name.clone(),
        );
    }
}

fn render_screenshot_overlay_action(
    ui: &mut Ui,
    tile_rect: egui::Rect,
    scope: &str,
    screenshot: &ScreenshotEntry,
    copy_bytes: Option<&[u8]>,
    copy_loading: bool,
) -> ScreenshotOverlayResult {
    let screenshot_key = screenshot.key();
    let mut result = ScreenshotOverlayResult::default();
    let copy_result = render_screenshot_overlay_button(
        ui,
        tile_rect,
        scope,
        screenshot_key.as_str(),
        "home_screenshot_copy_button",
        assets::COPY_SVG,
        ui.visuals().text_color(),
        if copy_loading {
            "Image is still loading"
        } else {
            "Copy image to clipboard"
        },
        8.0,
        copy_bytes.is_some(),
    );
    result.contains_pointer |= copy_result.contains_pointer;
    if copy_result.clicked {
        let Some(bytes) = copy_bytes else {
            return result;
        };
        copy_screenshot_to_clipboard(ui.ctx(), screenshot.file_name.as_str(), bytes);
        result.action = Some(ScreenshotOverlayAction::Copy);
        return result;
    }
    let delete_result = render_screenshot_overlay_button(
        ui,
        tile_rect,
        scope,
        screenshot_key.as_str(),
        "home_screenshot_delete_button",
        assets::TRASH_X_SVG,
        ui.visuals().error_fg_color,
        "Delete screenshot",
        8.0 + SCREENSHOT_COPY_BUTTON_SIZE + 6.0,
        true,
    );
    result.contains_pointer |= delete_result.contains_pointer;
    if delete_result.clicked {
        result.action = Some(ScreenshotOverlayAction::Delete);
    }
    result
}

fn render_screenshot_overlay_button(
    ui: &mut Ui,
    tile_rect: egui::Rect,
    scope: &str,
    screenshot_key: &str,
    id_source: &str,
    icon_svg: &[u8],
    icon_color: Color32,
    tooltip: &str,
    x_offset: f32,
    enabled: bool,
) -> ScreenshotOverlayButtonResult {
    let button_rect = egui::Rect::from_min_size(
        tile_rect.min + egui::vec2(x_offset, 8.0),
        egui::vec2(SCREENSHOT_COPY_BUTTON_SIZE, SCREENSHOT_COPY_BUTTON_SIZE),
    );
    let themed_svg = apply_color_to_svg(icon_svg, icon_color);
    let icon_color_key = format!(
        "{:02x}{:02x}{:02x}",
        icon_color.r(),
        icon_color.g(),
        icon_color.b()
    );
    let uri = format!("bytes://home/{id_source}/{scope}-{screenshot_key}-{icon_color_key}.svg");
    let response = ui.interact(
        button_rect,
        ui.id().with((id_source, scope, screenshot_key)),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let button_contains_pointer = ui_pointer_over_rect(ui, button_rect);
    let button_pressed = button_contains_pointer && ui.input(|input| input.pointer.primary_down());
    let fill = if response.is_pointer_button_down_on() || button_pressed {
        ui.visuals().widgets.active.bg_fill
    } else if button_contains_pointer {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        Color32::from_rgba_premultiplied(12, 16, 24, 210)
    };
    ui.painter()
        .rect_filled(button_rect, egui::CornerRadius::same(8), fill);
    ui.painter().rect_stroke(
        button_rect,
        egui::CornerRadius::same(8),
        ui.visuals().widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let icon_rect = egui::Rect::from_center_size(button_rect.center(), egui::vec2(14.0, 14.0));
    egui::Image::from_bytes(uri, themed_svg)
        .fit_to_exact_size(icon_rect.size())
        .tint(if enabled {
            Color32::WHITE
        } else {
            Color32::from_white_alpha(120)
        })
        .paint_at(ui, icon_rect);
    let clicked = response.clicked();
    if button_contains_pointer {
        let _ = egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            response.id.with("tooltip"),
            egui::PopupAnchor::Pointer,
        )
        .gap(12.0)
        .show(|ui| {
            ui.label(tooltip);
        });
    }
    ScreenshotOverlayButtonResult {
        clicked,
        contains_pointer: button_contains_pointer || response.is_pointer_button_down_on(),
    }
}

fn copy_screenshot_to_clipboard(ctx: &egui::Context, label: &str, bytes: &[u8]) {
    match decode_color_image(bytes) {
        Ok(image) => {
            ctx.copy_image(image);
            notification::info!("home/screenshots", "Copied '{}' to clipboard.", label);
        }
        Err(err) => {
            notification::error!(
                "home/screenshots",
                "Failed to copy '{}' to clipboard: {}",
                label,
                err
            );
        }
    }
}

fn decode_color_image(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    let rgba = image::load_from_memory(bytes)
        .map_err(|err| err.to_string())?
        .to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

fn fit_rect_to_aspect(rect: egui::Rect, aspect_ratio: f32) -> egui::Rect {
    let safe_aspect = aspect_ratio.max(0.01);
    let rect_aspect = rect.width() / rect.height().max(1.0);
    if rect_aspect > safe_aspect {
        let width = rect.height() * safe_aspect;
        let x = rect.center().x - width * 0.5;
        egui::Rect::from_min_size(egui::pos2(x, rect.min.y), egui::vec2(width, rect.height()))
    } else {
        let height = rect.width() / safe_aspect;
        let y = rect.center().y - height * 0.5;
        egui::Rect::from_min_size(egui::pos2(rect.min.x, y), egui::vec2(rect.width(), height))
    }
}

fn adjust_viewer_zoom(current_zoom: f32, direction: f32) -> f32 {
    (current_zoom + direction * SCREENSHOT_VIEWER_ZOOM_STEP)
        .clamp(SCREENSHOT_VIEWER_MIN_ZOOM, SCREENSHOT_VIEWER_MAX_ZOOM)
}

fn clamp_viewer_pan(viewer_state: &mut ScreenshotViewerState) {
    let visible_fraction = 1.0 / viewer_state.zoom.max(SCREENSHOT_VIEWER_MIN_ZOOM);
    let max_offset = (1.0 - visible_fraction) * 0.5;
    viewer_state.pan_uv.x = viewer_state.pan_uv.x.clamp(-max_offset, max_offset);
    viewer_state.pan_uv.y = viewer_state.pan_uv.y.clamp(-max_offset, max_offset);
}

fn viewer_uv_rect(viewer_state: &ScreenshotViewerState) -> egui::Rect {
    let visible_fraction = 1.0 / viewer_state.zoom.max(SCREENSHOT_VIEWER_MIN_ZOOM);
    let half = visible_fraction * 0.5;
    let center = egui::pos2(0.5 + viewer_state.pan_uv.x, 0.5 + viewer_state.pan_uv.y);
    egui::Rect::from_min_max(
        egui::pos2(center.x - half, center.y - half),
        egui::pos2(center.x + half, center.y + half),
    )
}

fn render_instance_usage(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instances: &InstanceStore,
    config: &Config,
    state: &mut HomeState,
    output: &mut HomeOutput,
) {
    let mut title_style = LabelOptions::default();
    title_style.font_size = 18.0;
    title_style.line_height = 24.0;
    title_style.weight = 700;
    title_style.color = ui.visuals().text_color();
    let _ = text_ui.label(ui, "home_usage_title", "Most Used Instances", &title_style);
    ui.add_space(6.0);

    let mut items = instances.instances.clone();
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
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        return;
    }

    let max_height = (ui.available_height() * (1.0 / 3.0)).clamp(140.0, 340.0);
    let now_ms = current_time_millis();
    egui::ScrollArea::vertical()
        .id_salt("home_instances_scroll")
        .max_height(max_height)
        .show(ui, |ui| {
            for (index, instance) in items.iter().enumerate() {
                let thumbnail_png = instance
                    .thumbnail_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .and_then(|path| {
                        let key = instance_thumbnail_cache_key(instance.id.as_str(), path);
                        match state.instance_thumbnail_cache.get(&key).cloned() {
                            Some(bytes) => bytes,
                            None => {
                                request_instance_thumbnail(
                                    state,
                                    instance.id.as_str(),
                                    path.to_owned(),
                                );
                                None
                            }
                        }
                    });
                let row_response = render_clickable_entry_row(
                    ui,
                    ("home_instance_row", index),
                    INSTANCE_ROW_HEIGHT,
                    |ui| {
                        render_entry_thumbnail(
                            ui,
                            ("home_instance_thumb", index),
                            thumbnail_png.as_deref(),
                            assets::LIBRARY_SVG,
                            40.0,
                            18.0,
                        );
                        ui.add_space(8.0);
                        let _ = text_ui.label(
                            ui,
                            ("home_usage_name", index),
                            instance.name.as_str(),
                            &LabelOptions {
                                weight: 600,
                                color: ui.visuals().text_color(),
                                wrap: false,
                                ..LabelOptions::default()
                            },
                        );
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
                                &LabelOptions {
                                    color: ui.visuals().weak_text_color(),
                                    wrap: false,
                                    ..LabelOptions::default()
                                },
                            );
                        });
                    },
                );
                let context_id =
                    ui.make_persistent_id(("home_instance_context", instance.id.as_str()));

                if row_response.clicked() {
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
                        InstanceContextAction::Delete => {
                            output.delete_requested_instance_id = Some(instance.id.clone());
                        }
                    }
                }

                ui.add_space(3.0);
            }
        });
}

fn ensure_instance_thumbnail_channel(state: &mut HomeState) {
    if state.instance_thumbnail_results_tx.is_some()
        && state.instance_thumbnail_results_rx.is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, Option<Arc<[u8]>>)>();
    state.instance_thumbnail_results_tx = Some(tx);
    state.instance_thumbnail_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn instance_thumbnail_cache_key(instance_id: &str, path: &str) -> String {
    format!("{instance_id}\n{path}")
}

fn request_instance_thumbnail(state: &mut HomeState, instance_id: &str, path: String) {
    let key = instance_thumbnail_cache_key(instance_id, path.as_str());
    if state.instance_thumbnail_in_flight.contains(key.as_str()) {
        return;
    }
    ensure_instance_thumbnail_channel(state);
    let Some(tx) = state.instance_thumbnail_results_tx.as_ref().cloned() else {
        return;
    };
    state.instance_thumbnail_in_flight.insert(key.clone());
    let _ = tokio_runtime::spawn_detached(async move {
        let bytes = std::fs::read(path.as_str())
            .ok()
            .map(|bytes| Arc::<[u8]>::from(bytes.into_boxed_slice()));
        let _ = tx.send((key, bytes));
    });
}

fn poll_instance_thumbnail_results(state: &mut HomeState) {
    let Some(rx) = state.instance_thumbnail_results_rx.as_ref() else {
        return;
    };
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    match rx.lock() {
        Ok(receiver) => loop {
            match receiver.try_recv() {
                Ok(update) => updates.push(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    should_reset_channel = true;
                    break;
                }
            }
        },
        Err(_) => should_reset_channel = true,
    }
    if should_reset_channel {
        state.instance_thumbnail_results_tx = None;
        state.instance_thumbnail_results_rx = None;
        state.instance_thumbnail_in_flight.clear();
    }
    for (key, bytes) in updates {
        state.instance_thumbnail_in_flight.remove(key.as_str());
        state.instance_thumbnail_cache.insert(key, bytes);
    }
}

fn render_activity_feed(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instances: &mut InstanceStore,
    state: &HomeState,
    streamer_mode: bool,
    output: &mut HomeOutput,
    requested_rescan: &mut bool,
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
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
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
                    &LabelOptions {
                        weight: 700,
                        color: ui.visuals().text_color(),
                        wrap: false,
                        ..LabelOptions::default()
                    },
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
                    &LabelOptions {
                        color: ui.visuals().weak_text_color(),
                        wrap: true,
                        ..LabelOptions::default()
                    },
                );
                return;
            }

            let _ = text_ui.label(
                ui,
                "home_activity_recent_title",
                "Recent",
                &LabelOptions {
                    weight: 700,
                    color: ui.visuals().text_color(),
                    wrap: false,
                    ..LabelOptions::default()
                },
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
) {
    let mut star_clicked = false;
    let row_response =
        render_clickable_entry_row(ui, (id_source, "row"), ACTIVITY_ROW_HEIGHT, |ui| {
            if render_favorite_star_button(ui, (id_source, "world_star"), world.favorite)
                .on_hover_text("Toggle world favorite")
                .clicked()
            {
                star_clicked = true;
            }
            ui.add_space(8.0);
            render_entry_thumbnail(
                ui,
                (id_source, "thumb"),
                world.thumbnail_png.as_deref(),
                assets::HOME_SVG,
                34.0,
                34.0,
            );
            ui.add_space(8.0);
            let name_label_options = LabelOptions {
                weight: 600,
                color: ui.visuals().text_color(),
                wrap: false,
                ..LabelOptions::default()
            };
            let meta_label_options = LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: false,
                ..LabelOptions::default()
            };
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
) {
    let server_meta_full = server_meta_line(server, ping, now_ms, streamer_mode);
    let mut star_clicked = false;
    let row_response =
        render_clickable_entry_row(ui, (id_source, "row"), ACTIVITY_ROW_HEIGHT, |ui| {
            if render_favorite_star_button(ui, (id_source, "server_star"), server.favorite)
                .on_hover_text("Toggle server favorite")
                .clicked()
            {
                star_clicked = true;
            }
            ui.add_space(8.0);
            render_entry_thumbnail(
                ui,
                (id_source, "thumb"),
                server.icon_png.as_deref(),
                assets::TERMINAL_SVG,
                34.0,
                34.0,
            );
            ui.add_space(8.0);
            let name_label_options = LabelOptions {
                weight: 600,
                color: ui.visuals().text_color(),
                wrap: false,
                ..LabelOptions::default()
            };
            let meta_label_options = LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: false,
                ..LabelOptions::default()
            };
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
}

fn server_meta_line(
    server: &ServerEntry,
    ping: Option<&ServerPingSnapshot>,
    now_ms: u64,
    streamer_mode: bool,
) -> String {
    let address = if streamer_mode {
        "IP hidden".to_owned()
    } else if server.port == 25565 {
        server.host.clone()
    } else {
        format!("{}:{}", server.host, server.port)
    };
    let ping_text = match ping.map(|value| value.status) {
        Some(ServerPingStatus::Online { latency_ms }) => format!("reachable {latency_ms}ms"),
        Some(ServerPingStatus::Offline) => "offline".to_owned(),
        _ => "status unknown".to_owned(),
    };
    let players_text = match ping {
        Some(ServerPingSnapshot {
            players_online: Some(online),
            players_max: Some(max),
            ..
        }) => format!("players {online}/{max}"),
        _ => "players n/a".to_owned(),
    };
    let motd = ping
        .and_then(|snapshot| snapshot.motd.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("motd unavailable")
        .to_owned();
    format!(
        "{} | {} | {} | {} | {} | last used {}",
        format!("instance {}", server.instance_name),
        address,
        motd,
        players_text,
        ping_text,
        format_time_ago(server.last_used_at_ms, now_ms)
    )
}

fn render_clickable_entry_row(
    ui: &mut Ui,
    id_source: impl std::hash::Hash,
    height: f32,
    add_contents: impl FnOnce(&mut Ui),
) -> egui::Response {
    let width = ui.available_width().max(1.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let visuals = ui.visuals();
    let fill = if response.is_pointer_button_down_on() {
        visuals.widgets.active.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.widgets.inactive.weak_bg_fill
    };
    let stroke = if response.hovered() {
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
    let inner = rect.shrink2(egui::vec2(8.0, 5.0));
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
    image_png: Option<&[u8]>,
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
        if let Some(png) = image_png {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            id_source.hash(&mut hasher);
            let uri = format!("bytes://home/entry-thumb/{}.png", hasher.finish());
            let image_size = (height - 4.0).max(1.0).min(width - 4.0).max(1.0);
            let image_rect =
                egui::Rect::from_center_size(rect.center(), egui::vec2(image_size, image_size));
            ui.put(
                image_rect,
                egui::Image::from_bytes(uri, png.to_vec())
                    .fit_to_exact_size(egui::vec2(image_size, image_size)),
            );
        } else {
            ui.with_layout(Layout::top_down(egui::Align::Center), |ui| {
                ui.add_space(((height - ENTRY_ICON_SIZE) * 0.5).max(0.0));
                let themed_svg = apply_color_to_svg(icon_svg, ui.visuals().text_color());
                let uri = format!("bytes://home/entry-thumb/{:?}.svg", ui.id().with(id_source));
                ui.add(
                    egui::Image::from_bytes(uri, themed_svg)
                        .fit_to_exact_size(egui::vec2(ENTRY_ICON_SIZE, ENTRY_ICON_SIZE)),
                );
            });
        }
    });
}

fn render_server_ping_icon(ui: &mut Ui, ping: Option<&ServerPingSnapshot>) {
    let (icon, color, tip) =
        ping_icon_for_status(ui.visuals(), ping.map(|snapshot| snapshot.status));
    let themed_svg = apply_color_to_svg(icon, color);
    let uri = format!(
        "bytes://home/server-ping/{:?}-{:02x}{:02x}{:02x}.svg",
        ping.map(|value| value.status),
        color.r(),
        color.g(),
        color.b()
    );
    ui.add(
        egui::Image::from_bytes(uri, themed_svg)
            .fit_to_exact_size(egui::vec2(SERVER_PING_ICON_SIZE, SERVER_PING_ICON_SIZE))
            .sense(egui::Sense::hover()),
    )
    .on_hover_text(tip);
}

fn render_favorite_star_button(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + Copy,
    active: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(FAVORITE_STAR_BUTTON_SIZE, FAVORITE_STAR_BUTTON_SIZE),
        egui::Sense::click(),
    );
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

fn ping_icon_for_status(
    visuals: &egui::Visuals,
    status: Option<ServerPingStatus>,
) -> (&'static [u8], Color32, String) {
    match status.unwrap_or(ServerPingStatus::Unknown) {
        ServerPingStatus::Unknown => (
            assets::ANTENNA_BARS_OFF_SVG,
            visuals.weak_text_color().gamma_multiply(0.9),
            "Ping unknown".to_owned(),
        ),
        ServerPingStatus::Offline => (
            assets::ANTENNA_BARS_OFF_SVG,
            visuals.error_fg_color,
            "Server offline".to_owned(),
        ),
        ServerPingStatus::Online { latency_ms } => {
            let (icon, color) = if latency_ms <= 80 {
                (assets::ANTENNA_BARS_5_SVG, visuals.text_cursor.stroke.color)
            } else if latency_ms <= 140 {
                (
                    assets::ANTENNA_BARS_4_SVG,
                    visuals.text_cursor.stroke.color.gamma_multiply(0.9),
                )
            } else if latency_ms <= 220 {
                (
                    assets::ANTENNA_BARS_3_SVG,
                    visuals.warn_fg_color.gamma_multiply(0.85),
                )
            } else if latency_ms <= 320 {
                (assets::ANTENNA_BARS_2_SVG, visuals.warn_fg_color)
            } else {
                (
                    assets::ANTENNA_BARS_1_SVG,
                    visuals.error_fg_color.gamma_multiply(0.92),
                )
            };
            (icon, color, format!("Latency: {latency_ms}ms"))
        }
    }
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
        return;
    };
    let result_tx = channel.tx.clone();
    drop(channel);

    state.latest_requested_activity_scan_id = request_id;
    state.activity_scan_pending = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let scanned_instance_count = request.scanned_instance_count;
        let result = HomeActivityScanResult {
            request_id,
            scanned_instance_count,
            worlds: collect_worlds_from_request(&request),
            servers: collect_servers_from_request(&request),
        };
        let _ = result_tx.send(result);
    });
}

fn refresh_screenshot_state(
    state: &mut HomeState,
    instances: &InstanceStore,
    config: &Config,
    force: bool,
) {
    if state.screenshot_scan_pending && !force {
        return;
    }

    let request_id = state.latest_requested_screenshot_scan_id.saturating_add(1);
    let request = build_screenshot_scan_request(instances, config);

    state.latest_requested_screenshot_scan_id = request_id;
    state.screenshot_scan_pending = true;
    state.screenshot_scan_ready = true;
    state.screenshot_tasks_total = 0;
    state.screenshot_tasks_done = 0;
    state.screenshots.clear();

    // Directory listing reads only file names and mtimes — no file content.
    // Doing it synchronously avoids a full frame of latency before dimension
    // tasks can be spawned.
    let candidates = collect_screenshot_candidates(&request);
    state.scanned_screenshot_instance_count = request.scanned_instance_count;
    state.screenshot_candidates = candidates;
    state.screenshot_loaded_count = 0;

    if state.screenshot_candidates.is_empty() {
        state.screenshot_scan_pending = false;
        state.last_screenshot_scan_at = Some(Instant::now());
    } else {
        spawn_screenshot_load_page(state, request_id, SCREENSHOT_PAGE_SIZE);
    }
}

fn collect_screenshot_candidates(request: &ScreenshotScanRequest) -> Vec<ScreenshotCandidate> {
    let mut candidates = Vec::new();
    for instance in &request.instances {
        let Ok(entries) = fs::read_dir(&instance.screenshots_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || !is_supported_screenshot_path(path.as_path()) {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            candidates.push(ScreenshotCandidate {
                instance_name: instance.instance_name.clone(),
                file_name,
                modified_at_ms: modified_millis(path.as_path()),
                path,
            });
        }
    }
    candidates.sort_by(|a, b| {
        b.modified_at_ms
            .unwrap_or(0)
            .cmp(&a.modified_at_ms.unwrap_or(0))
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    candidates
}

fn is_supported_screenshot_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(OsStr::to_str)
            .map(|extension| extension.to_ascii_lowercase()),
        Some(extension)
            if matches!(
                extension.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
            )
    )
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

fn collect_servers_from_request(request: &HomeActivityScanRequest) -> Vec<ServerEntry> {
    let mut servers = Vec::new();
    for instance in &request.instances {
        let servers_dat = instance.instance_root.join("servers.dat");
        let last_used_at_ms = modified_millis(servers_dat.as_path());
        let parsed = parse_servers_dat(servers_dat.as_path()).unwrap_or_default();
        for server in parsed {
            let favorite_id = normalize_server_address(server.ip.as_str());
            let (host, port) = split_server_address(server.ip.as_str());
            servers.push(ServerEntry {
                instance_id: instance.instance_id.clone(),
                instance_name: instance.instance_name.clone(),
                server_name: server.name,
                address: server.ip,
                favorite_id: favorite_id.clone(),
                host,
                port,
                icon_png: decode_server_icon(server.icon.as_deref()),
                last_used_at_ms,
                favorite: instance
                    .favorite_server_ids
                    .iter()
                    .any(|id| id == &favorite_id),
            });
        }
    }
    servers.sort_by(|a, b| {
        b.last_used_at_ms
            .unwrap_or(0)
            .cmp(&a.last_used_at_ms.unwrap_or(0))
            .then_with(|| a.server_name.cmp(&b.server_name))
    });
    servers
}

fn retain_known_server_pings(state: &mut HomeState) {
    let known_addresses: HashSet<String> = state
        .servers
        .iter()
        .map(|server| normalize_server_address(server.address.as_str()))
        .collect();
    state
        .server_pings
        .retain(|address, _| known_addresses.contains(address));
    state
        .server_ping_in_flight
        .retain(|address| known_addresses.contains(address));
}

fn queue_server_pings(state: &mut HomeState) {
    retain_known_server_pings(state);
    let mut stale_addresses = Vec::new();
    for server in &state.servers {
        let key = normalize_server_address(server.address.as_str());
        let stale = state
            .server_pings
            .get(&key)
            .is_none_or(|snapshot| snapshot.checked_at.elapsed() >= SERVER_PING_REFRESH_INTERVAL);
        if stale
            && !state.server_ping_in_flight.contains(&key)
            && !stale_addresses.iter().any(|candidate| candidate == &key)
        {
            stale_addresses.push(key);
        }
    }

    let Ok(channel) = server_ping_results().lock() else {
        return;
    };
    let result_tx = channel.tx.clone();
    drop(channel);

    for address in stale_addresses.into_iter().take(SERVER_PINGS_PER_SCAN) {
        state.server_ping_in_flight.insert(address.clone());
        let worker_address = address.clone();
        let result_tx = result_tx.clone();
        let _ = tokio_runtime::spawn_detached(async move {
            let snapshot = query_server_snapshot(worker_address.as_str());
            let _ = result_tx.send(ServerPingResult { address, snapshot });
        });
    }
}

fn normalize_server_address(address: &str) -> String {
    address.trim().to_ascii_lowercase()
}

fn split_server_address(address: &str) -> (String, u16) {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return (String::new(), 25565);
    }
    if let Ok(socket) = trimmed.parse::<SocketAddr>() {
        return (socket.ip().to_string(), socket.port());
    }
    if let Some(host) = trimmed
        .strip_prefix('[')
        .and_then(|value| value.split(']').next())
        && let Some(port) = trimmed
            .rsplit_once(':')
            .and_then(|(_, value)| value.parse().ok())
    {
        return (host.to_owned(), port);
    }
    if let Some((host, port)) = trimmed.rsplit_once(':')
        && !host.is_empty()
        && !host.contains(':')
        && let Ok(port) = port.parse::<u16>()
    {
        return (host.to_owned(), port);
    }
    (trimmed.to_owned(), 25565)
}

fn query_server_snapshot(address: &str) -> ServerPingSnapshot {
    let unknown = || ServerPingSnapshot {
        status: ServerPingStatus::Unknown,
        motd: None,
        players_online: None,
        players_max: None,
        checked_at: Instant::now(),
    };
    let (host, port) = split_server_address(address);
    if host.is_empty() {
        return unknown();
    }
    let mut stream = match connect_to_server(host.as_str(), port) {
        Some(stream) => stream,
        None => {
            return ServerPingSnapshot {
                status: ServerPingStatus::Offline,
                motd: None,
                players_online: None,
                players_max: None,
                checked_at: Instant::now(),
            };
        }
    };
    let _ = stream.set_read_timeout(Some(SERVER_PING_CONNECT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SERVER_PING_CONNECT_TIMEOUT));

    let start = Instant::now();
    match request_server_status(&mut stream, host.as_str(), port) {
        Ok((motd, players_online, players_max)) => ServerPingSnapshot {
            status: ServerPingStatus::Online {
                latency_ms: start.elapsed().as_millis() as u64,
            },
            motd,
            players_online,
            players_max,
            checked_at: Instant::now(),
        },
        Err(_) => ServerPingSnapshot {
            status: ServerPingStatus::Online {
                latency_ms: start.elapsed().as_millis() as u64,
            },
            motd: None,
            players_online: None,
            players_max: None,
            checked_at: Instant::now(),
        },
    }
}

fn connect_to_server(host: &str, port: u16) -> Option<TcpStream> {
    let mut saw_target = false;
    if let Ok(ip) = host.parse::<IpAddr>() {
        saw_target = true;
        if let Ok(stream) =
            TcpStream::connect_timeout(&SocketAddr::new(ip, port), SERVER_PING_CONNECT_TIMEOUT)
        {
            return Some(stream);
        }
    } else if let Ok(candidates) = (host, port).to_socket_addrs() {
        for candidate in candidates {
            saw_target = true;
            if let Ok(stream) = TcpStream::connect_timeout(&candidate, SERVER_PING_CONNECT_TIMEOUT)
            {
                return Some(stream);
            }
        }
    }
    if !saw_target {
        return None;
    }
    None
}

fn request_server_status(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
) -> Result<(Option<String>, Option<u32>, Option<u32>), ()> {
    send_handshake_packet(stream, host, port)?;
    send_status_request_packet(stream)?;
    let json = read_status_response_packet(stream)?;
    parse_status_json(json.as_str())
}

fn send_handshake_packet(stream: &mut TcpStream, host: &str, port: u16) -> Result<(), ()> {
    let mut payload = Vec::new();
    write_varint(&mut payload, 0); // Handshake packet ID.
    write_varint_i32(&mut payload, -1); // Status query protocol version sentinel.
    write_mc_string(&mut payload, host)?;
    payload.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut payload, 1); // Next state: status.
    write_framed_packet(stream, &payload)
}

fn send_status_request_packet(stream: &mut TcpStream) -> Result<(), ()> {
    write_framed_packet(stream, &[0]) // Status request packet ID.
}

fn read_status_response_packet(stream: &mut TcpStream) -> Result<String, ()> {
    let _packet_len = read_varint_from_stream(stream)?;
    let packet_id = read_varint_from_stream(stream)?;
    if packet_id != 0 {
        return Err(());
    }
    read_mc_string_from_stream(stream)
}

fn write_framed_packet(stream: &mut TcpStream, payload: &[u8]) -> Result<(), ()> {
    let mut frame = Vec::new();
    write_varint(&mut frame, payload.len() as u32);
    frame.extend_from_slice(payload);
    stream.write_all(frame.as_slice()).map_err(|_| ())
}

fn write_varint(buf: &mut Vec<u8>, mut value: u32) {
    loop {
        if (value & !0x7F) == 0 {
            buf.push(value as u8);
            return;
        }
        buf.push(((value & 0x7F) as u8) | 0x80);
        value >>= 7;
    }
}

fn write_varint_i32(buf: &mut Vec<u8>, value: i32) {
    write_varint(buf, value as u32);
}

fn read_varint_from_stream(stream: &mut TcpStream) -> Result<u32, ()> {
    let mut num_read = 0u32;
    let mut result = 0u32;
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).map_err(|_| ())?;
        let value = (byte[0] & 0x7F) as u32;
        result |= value << (7 * num_read);
        num_read += 1;
        if num_read > 5 {
            return Err(());
        }
        if (byte[0] & 0x80) == 0 {
            break;
        }
    }
    Ok(result)
}

fn write_mc_string(buf: &mut Vec<u8>, value: &str) -> Result<(), ()> {
    let bytes = value.as_bytes();
    let len = u32::try_from(bytes.len()).map_err(|_| ())?;
    write_varint(buf, len);
    buf.extend_from_slice(bytes);
    Ok(())
}

fn read_mc_string_from_stream(stream: &mut TcpStream) -> Result<String, ()> {
    let len = read_varint_from_stream(stream)? as usize;
    let mut bytes = vec![0u8; len];
    stream.read_exact(bytes.as_mut_slice()).map_err(|_| ())?;
    Ok(String::from_utf8_lossy(bytes.as_slice()).to_string())
}

fn parse_status_json(raw: &str) -> Result<(Option<String>, Option<u32>, Option<u32>), ()> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| ())?;
    let motd = value
        .get("description")
        .and_then(motd_from_json)
        .map(|text| strip_minecraft_format_codes(text.as_str()))
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    let players_online = value
        .get("players")
        .and_then(|players| players.get("online"))
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok());
    let players_max = value
        .get("players")
        .and_then(|players| players.get("max"))
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok());
    Ok((motd, players_online, players_max))
}

fn motd_from_json(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    let mut out = String::new();
    append_motd_text(value, &mut out);
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn append_motd_text(value: &serde_json::Value, out: &mut String) {
    if let Some(text) = value.get("text").and_then(|text| text.as_str()) {
        out.push_str(text);
    }
    if let Some(extra) = value.get("extra").and_then(|extra| extra.as_array()) {
        for part in extra {
            append_motd_text(part, out);
        }
    }
}

fn strip_minecraft_format_codes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '§' {
            let _ = chars.next();
            continue;
        }
        out.push(ch);
    }
    out
}

fn parse_world_metadata(path: &Path) -> Option<WorldMetadata> {
    let data = read_nbt_file(path)?;
    parse_world_metadata_from_nbt(data.as_slice()).ok()
}

fn parse_world_metadata_from_nbt(bytes: &[u8]) -> Result<WorldMetadata, ()> {
    let mut cursor = NbtCursor::new(bytes);
    let root_tag = cursor.read_u8()?;
    if root_tag != 10 {
        return Err(());
    }
    let _ = cursor.read_string()?;
    let mut metadata = WorldMetadata::default();
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            break;
        }
        let key = cursor.read_string()?;
        if tag == 10 && key == "Data" {
            parse_world_data_compound(&mut cursor, &mut metadata)?;
        } else {
            skip_nbt_payload(&mut cursor, tag)?;
        }
    }
    Ok(metadata)
}

fn parse_world_data_compound(
    cursor: &mut NbtCursor<'_>,
    metadata: &mut WorldMetadata,
) -> Result<(), ()> {
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            return Ok(());
        }
        let key = cursor.read_string()?;
        match (tag, key.as_str()) {
            (8, "LevelName") => metadata.level_name = Some(cursor.read_string()?),
            (3, "GameType") => metadata.game_mode = Some(game_mode_label(cursor.read_i32()?)),
            (1, "hardcore") => metadata.hardcore = Some(cursor.read_u8()? != 0),
            (1, "allowCommands") => metadata.cheats_enabled = Some(cursor.read_u8()? != 0),
            (1, "Difficulty") => metadata.difficulty = Some(difficulty_label(cursor.read_u8()?)),
            (4, "LastPlayed") => {
                let last_played = cursor.read_i64()?;
                if last_played > 0 {
                    metadata.last_played_ms = Some(last_played as u64);
                }
            }
            (10, "Version") => parse_world_version_compound(cursor, metadata)?,
            _ => skip_nbt_payload(cursor, tag)?,
        }
    }
}

fn parse_world_version_compound(
    cursor: &mut NbtCursor<'_>,
    metadata: &mut WorldMetadata,
) -> Result<(), ()> {
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            return Ok(());
        }
        let key = cursor.read_string()?;
        match (tag, key.as_str()) {
            (8, "Name") => metadata.version_name = Some(cursor.read_string()?),
            _ => skip_nbt_payload(cursor, tag)?,
        }
    }
}

fn game_mode_label(game_type: i32) -> String {
    match game_type {
        0 => "survival".to_owned(),
        1 => "creative".to_owned(),
        2 => "adventure".to_owned(),
        3 => "spectator".to_owned(),
        other => format!("mode {other}"),
    }
}

fn difficulty_label(value: u8) -> String {
    match value {
        0 => "peaceful".to_owned(),
        1 => "easy".to_owned(),
        2 => "normal".to_owned(),
        3 => "hard".to_owned(),
        other => format!("difficulty {other}"),
    }
}

fn read_nbt_file(path: &Path) -> Option<Vec<u8>> {
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() {
        return Some(Vec::new());
    }
    if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).ok()?;
        return Some(out);
    }
    Some(bytes)
}

fn read_world_thumbnail(path: &Path) -> Option<Vec<u8>> {
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    // Guard against unexpectedly large files in world folders.
    if bytes.len() > 4 * 1024 * 1024 {
        return None;
    }
    Some(bytes)
}

fn decode_server_icon(raw: Option<&str>) -> Option<Vec<u8>> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let encoded = raw
        .strip_prefix("data:image/png;base64,")
        .or_else(|| raw.strip_prefix("data:image/png;base64"))
        .unwrap_or(raw)
        .trim_start_matches(',')
        .trim();
    if encoded.is_empty() {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .ok()?;
    if decoded.is_empty() || decoded.len() > 4 * 1024 * 1024 {
        return None;
    }
    Some(decoded)
}

fn parse_servers_dat(path: &Path) -> Option<Vec<ServerDatEntry>> {
    let data = read_nbt_file(path)?;
    parse_servers_from_nbt(data.as_slice()).ok()
}

fn parse_servers_from_nbt(bytes: &[u8]) -> Result<Vec<ServerDatEntry>, ()> {
    let mut cursor = NbtCursor::new(bytes);
    let root_tag = cursor.read_u8()?;
    if root_tag != 10 {
        return Err(());
    }
    let _ = cursor.read_string()?;
    let mut servers = Vec::new();
    parse_compound_for_servers(&mut cursor, &mut servers)?;
    Ok(servers)
}

fn parse_compound_for_servers(
    cursor: &mut NbtCursor<'_>,
    servers: &mut Vec<ServerDatEntry>,
) -> Result<(), ()> {
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            return Ok(());
        }
        let name = cursor.read_string()?;
        if tag == 9 && name == "servers" {
            parse_servers_list(cursor, servers)?;
        } else {
            skip_nbt_payload(cursor, tag)?;
        }
    }
}

fn parse_servers_list(cursor: &mut NbtCursor<'_>, out: &mut Vec<ServerDatEntry>) -> Result<(), ()> {
    let item_tag = cursor.read_u8()?;
    let len = cursor.read_i32()?;
    if len <= 0 {
        return Ok(());
    }
    let len = len as usize;
    for _ in 0..len {
        if item_tag == 10 {
            if let Some(entry) = parse_server_compound(cursor)? {
                out.push(entry);
            }
        } else {
            skip_nbt_payload(cursor, item_tag)?;
        }
    }
    Ok(())
}

fn parse_server_compound(cursor: &mut NbtCursor<'_>) -> Result<Option<ServerDatEntry>, ()> {
    let mut name = String::new();
    let mut ip = String::new();
    let mut icon = None;
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            break;
        }
        let key = cursor.read_string()?;
        match (tag, key.as_str()) {
            (8, "name") => name = cursor.read_string()?,
            (8, "ip") => ip = cursor.read_string()?,
            (8, "icon") => icon = Some(cursor.read_string()?),
            _ => skip_nbt_payload(cursor, tag)?,
        }
    }
    if ip.trim().is_empty() {
        return Ok(None);
    }
    if name.trim().is_empty() {
        name = ip.clone();
    }
    Ok(Some(ServerDatEntry { name, ip, icon }))
}

fn skip_nbt_payload(cursor: &mut NbtCursor<'_>, tag: u8) -> Result<(), ()> {
    match tag {
        0 => Ok(()),
        1 => cursor.skip(1),
        2 => cursor.skip(2),
        3 => cursor.skip(4),
        4 => cursor.skip(8),
        5 => cursor.skip(4),
        6 => cursor.skip(8),
        7 => {
            let len = cursor.read_i32()?;
            if len < 0 {
                return Err(());
            }
            cursor.skip(len as usize)
        }
        8 => {
            let len = cursor.read_u16()? as usize;
            cursor.skip(len)
        }
        9 => {
            let nested_tag = cursor.read_u8()?;
            let len = cursor.read_i32()?;
            if len < 0 {
                return Err(());
            }
            for _ in 0..(len as usize) {
                skip_nbt_payload(cursor, nested_tag)?;
            }
            Ok(())
        }
        10 => loop {
            let nested = cursor.read_u8()?;
            if nested == 0 {
                break Ok(());
            }
            let _ = cursor.read_string()?;
            skip_nbt_payload(cursor, nested)?;
        },
        11 => {
            let len = cursor.read_i32()?;
            if len < 0 {
                return Err(());
            }
            cursor.skip((len as usize) * 4)
        }
        12 => {
            let len = cursor.read_i32()?;
            if len < 0 {
                return Err(());
            }
            cursor.skip((len as usize) * 8)
        }
        _ => Err(()),
    }
}

#[derive(Debug)]
struct NbtCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> NbtCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn skip(&mut self, len: usize) -> Result<(), ()> {
        if self.pos.saturating_add(len) > self.bytes.len() {
            return Err(());
        }
        self.pos += len;
        Ok(())
    }

    fn read_u8(&mut self) -> Result<u8, ()> {
        if self.pos >= self.bytes.len() {
            return Err(());
        }
        let value = self.bytes[self.pos];
        self.pos += 1;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u16, ()> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_i32(&mut self) -> Result<i32, ()> {
        let bytes = self.read_exact(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_i64(&mut self) -> Result<i64, ()> {
        let bytes = self.read_exact(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_string(&mut self) -> Result<String, ()> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_exact(len)?;
        Ok(String::from_utf8_lossy(bytes).to_string())
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], ()> {
        if self.pos.saturating_add(len) > self.bytes.len() {
            return Err(());
        }
        let start = self.pos;
        self.pos += len;
        Ok(&self.bytes[start..start + len])
    }
}

fn modified_millis(path: &Path) -> Option<u64> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some(
        modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default(),
    )
}

fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn format_time_ago(timestamp_ms: Option<u64>, now_ms: u64) -> String {
    let Some(timestamp_ms) = timestamp_ms else {
        return "never".to_owned();
    };
    let seconds = now_ms.saturating_sub(timestamp_ms) / 1000;
    if seconds < 60 {
        return format!("{seconds}s ago");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    format!("{days}d ago")
}

fn open_home_instance_folder(
    instance_id: &str,
    instances: &InstanceStore,
    config: &Config,
) -> Result<(), String> {
    let Some(instance) = instances
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
    else {
        return Err(format!("unknown instance id: {instance_id}"));
    };
    let root = instance_root_path(Path::new(config.minecraft_installations_root()), instance);
    desktop::open_in_file_manager(root.as_path())
}

fn open_home_instance(output: &mut HomeOutput, instance_id: &str) {
    output.selected_instance_id = Some(instance_id.to_owned());
    output.requested_screen = Some(AppScreen::Instance);
}
