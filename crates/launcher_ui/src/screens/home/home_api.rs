//! Toolkit-free view of Home: worlds and servers across every instance (with server pings and favorites), the
//! most used instances, and the cross-instance screenshot gallery. The scanning and pinging are the egui screen's.

use super::*;
use super::home_server_ping::ServerPingStatus;
use super::home_screenshots::request_screenshot_delete;


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    World,
    Server,
}

/// One instance an entry can be launched in.
#[derive(Clone, Debug)]
pub struct LaunchTarget {
    pub instance_id: String,
    pub instance_name: String,
    /// The world folder name or server address as that instance knows it.
    pub target: String,
}

#[derive(Clone, Debug)]
pub struct PingView {
    pub online: bool,
    pub latency_ms: Option<u64>,
    /// "12/100" once the server reported player counts.
    pub players: Option<String>,
    /// Names from the status sample, for a tooltip.
    pub players_tip: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EntryView {
    pub key: String,
    pub kind: EntryKind,
    pub title: String,
    pub meta: String,
    /// A server's MOTD as the game draws it (one entry per line, each a list of styled runs); empty for worlds and for
    /// servers that have not answered. When it is not empty, `meta` is only the "last used" part.
    pub motd: Vec<Vec<crate::mc_text::Span>>,
    pub favorite: bool,
    /// PNG bytes of the world thumbnail or server icon.
    pub icon_png: Option<Arc<[u8]>>,
    pub ping: Option<PingView>,
    /// Every instance this entry exists in (the first is the primary).
    pub targets: Vec<LaunchTarget>,
    /// The Minecraft version a world was last saved with.
    pub world_version: Option<String>,
    last_used_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct ActivityView {
    pub favorites: Vec<EntryView>,
    pub recent: Vec<EntryView>,
}

#[derive(Clone, Debug)]
pub struct ScreenshotView {
    pub key: String,
    pub path: PathBuf,
    pub file_name: String,
    pub instance_name: String,
    pub width: u32,
    pub height: u32,
    pub modified_at_ms: Option<u64>,
}

/// One per UI.
#[derive(Default)]
pub struct HomeSession {
    state: HomeState,
    rescan: bool,
}

impl HomeSession {
    /// Polls scans and pings, rescanning on a timer or when the instance count changed. Returns whether work is
    /// in flight (the caller should look again soon).
    pub fn tick_activity(&mut self, instances: &InstanceStore, config: &Config) -> bool {
        let state = &mut self.state;
        poll_home_activity_results(state);
        poll_server_ping_results(state);
        let should_scan = state.last_scan_at.is_none_or(|last| last.elapsed() >= HOME_SCAN_INTERVAL)
            || state.scanned_instance_count != instances.instances.len()
            || std::mem::take(&mut self.rescan);
        if should_scan {
            refresh_home_state(state, instances, config, false);
        }
        queue_server_pings(state);
        state.activity_scan_pending || !state.server_ping_in_flight.is_empty()
    }

    pub fn scan_interval() -> Duration {
        HOME_SCAN_INTERVAL
    }

    pub fn activity(&self) -> ActivityView {
        let state = &self.state;
        let now = current_time_millis();
        let mut favorites = Vec::new();
        let mut recent = Vec::new();
        for world in &state.worlds {
            let view = EntryView {
                key: format!("world::{}", world.group_key),
                kind: EntryKind::World,
                title: world.world_name.clone(),
                meta: home_api_world_meta(world, now),
                motd: Vec::new(),
                favorite: world.favorite,
                icon_png: world.thumbnail_png.clone(),
                ping: None,
                targets: targets_of(&world.instances, &world.instance_id, &world.instance_name, &world.world_id),
                world_version: world.version_name.clone(),
                last_used_at_ms: world.last_used_at_ms,
            };
            if view.favorite { favorites.push(view) } else { recent.push(view) }
        }
        for server in &state.servers {
            let ping = state.server_pings.get(&normalize_server_address(&server.address));
            let view = EntryView {
                key: format!("server::{}", server.favorite_id),
                kind: EntryKind::Server,
                title: server.server_name.clone(),
                meta: server_meta_line(server, ping, now),
                motd: ping.map(|ping| ping.motd.clone()).unwrap_or_default(),
                favorite: server.favorite,
                icon_png: server.icon_png.clone(),
                ping: ping.map(|ping| PingView {
                    online: matches!(ping.status, ServerPingStatus::Online { .. }),
                    latency_ms: match ping.status {
                        ServerPingStatus::Online { latency_ms } => Some(latency_ms),
                        _ => None,
                    },
                    players: server_player_count_text(Some(ping)),
                    players_tip: server_player_list_tip(Some(ping)),
                }),
                targets: targets_of(&server.instances, &server.instance_id, &"".to_owned(), &server.address),
                world_version: None,
                last_used_at_ms: server.last_used_at_ms,
            };
            if view.favorite { favorites.push(view) } else { recent.push(view) }
        }
        let order = |a: &EntryView, b: &EntryView| {
            b.last_used_at_ms.unwrap_or(0).cmp(&a.last_used_at_ms.unwrap_or(0)).then_with(|| a.title.cmp(&b.title))
        };
        favorites.sort_by(order);
        recent.sort_by(order);
        ActivityView { favorites, recent }
    }

    /// Rescans worlds and servers on the next tick, e.g. after a world sync changed what is shared.
    pub fn request_rescan(&mut self) {
        self.rescan = true;
    }

    /// Flips the favorite mark of `entry` in every instance that has it, and rescans.
    pub fn toggle_favorite(&mut self, instances: &mut InstanceStore, entry: &EntryView) {
        for target in &entry.targets {
            let _ = match entry.kind {
                EntryKind::World => set_world_favorite(instances, &target.instance_id, &target.target, !entry.favorite),
                EntryKind::Server => {
                    let favorite_id = entry.key.strip_prefix("server::").unwrap_or_default();
                    set_server_favorite(instances, &target.instance_id, favorite_id, !entry.favorite)
                }
            };
        }
        self.rescan = true;
    }

    // -- Screenshots ----------------------------------------------------------------------------------------------

    pub fn tick_screenshots(&mut self, instances: &InstanceStore, config: &Config) -> bool {
        let state = &mut self.state;
        poll_delete_screenshot_results(state, instances, config);
        poll_screenshot_results(state);
        poll_screenshot_candidates(state);
        load_next_screenshot_page(state);
        let should_scan = state.last_screenshot_scan_at.is_none_or(|last| last.elapsed() >= SCREENSHOT_SCAN_INTERVAL)
            || state.scanned_screenshot_instance_count != instances.instances.len();
        if should_scan {
            refresh_screenshot_state(state, instances, config, false);
        }
        state.screenshot_scan_pending || state.candidate_scan_in_flight || state.delete_screenshot_in_flight
    }

    pub fn screenshot_scan_interval() -> Duration {
        SCREENSHOT_SCAN_INTERVAL
    }

    pub fn screenshots(&self) -> Vec<ScreenshotView> {
        self.state
            .screenshots
            .iter()
            .map(|shot| ScreenshotView {
                key: shot.key(),
                path: shot.path.clone(),
                file_name: shot.file_name.clone(),
                instance_name: shot.instance_name.clone(),
                width: shot.width,
                height: shot.height,
                modified_at_ms: shot.modified_at_ms,
            })
            .collect()
    }

    pub fn screenshots_loading(&self) -> bool {
        self.state.screenshot_scan_pending || self.state.candidate_scan_in_flight
    }

    pub fn delete_in_flight(&self) -> bool {
        self.state.delete_screenshot_in_flight
    }

    pub fn delete_screenshot(&mut self, key: &str) {
        let Some(shot) = self.state.screenshots.iter().find(|shot| shot.key() == key).cloned() else { return };
        request_screenshot_delete(&mut self.state, key.to_owned(), shot.path, shot.file_name);
    }

    /// Drops the screenshot list (the tab was left) so it rescans next time.
    pub fn forget_screenshots(&mut self) {
        self.state.latest_requested_screenshot_scan_id = self.state.latest_requested_screenshot_scan_id.saturating_add(1);
        self.state.screenshots.clear();
        self.state.last_screenshot_scan_at = None;
        self.state.scanned_screenshot_instance_count = 0;
        self.state.screenshot_scan_pending = false;
        self.state.screenshot_scan_ready = false;
        self.state.screenshot_candidates.clear();
        self.state.screenshot_loaded_count = 0;
    }
}

fn targets_of(members: &[EntryInstance], instance_id: &str, instance_name: &str, target: &str) -> Vec<LaunchTarget> {
    if members.is_empty() {
        return vec![LaunchTarget { instance_id: instance_id.to_owned(), instance_name: instance_name.to_owned(), target: target.to_owned() }];
    }
    members
        .iter()
        .map(|member| LaunchTarget {
            instance_id: member.instance_id.clone(),
            instance_name: member.instance_name.clone(),
            target: member.target.clone(),
        })
        .collect()
}

fn home_api_world_meta(world: &WorldEntry, now_ms: u64) -> String {
    world_meta_line(world, now_ms)
}

pub fn time_ago(timestamp_ms: Option<u64>) -> String {
    format_time_ago(timestamp_ms, current_time_millis())
}
