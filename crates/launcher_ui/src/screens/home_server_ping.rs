use vertex_i18n::tr;

use base64::Engine;

use super::*;

#[path = "home_server_ping/server_entry.rs"]
mod server_entry;
#[path = "home_server_ping/server_ping_result.rs"]
mod server_ping_result;
#[path = "home_server_ping/server_ping_result_channel.rs"]
mod server_ping_result_channel;
#[path = "home_server_ping/server_ping_snapshot.rs"]
mod server_ping_snapshot;
#[path = "home_server_ping/server_ping_status.rs"]
mod server_ping_status;

pub(super) use self::server_entry::ServerEntry;
use self::server_ping_result::ServerPingResult;
use self::server_ping_result_channel::ServerPingResultChannel;
pub(super) use self::server_ping_snapshot::ServerPingSnapshot;
pub(super) use self::server_ping_status::ServerPingStatus;

static SERVER_PING_RESULTS: OnceLock<Mutex<ServerPingResultChannel>> = OnceLock::new();

fn server_ping_results() -> &'static Mutex<ServerPingResultChannel> {
    SERVER_PING_RESULTS.get_or_init(|| {
        let (result_tx, result_rx) = mpsc::channel::<ServerPingResult>();
        Mutex::new(ServerPingResultChannel {
            rx: result_rx,
            tx: result_tx,
        })
    })
}

pub(super) fn poll_server_ping_results(state: &mut HomeState) {
    let Ok(channel) = server_ping_results().lock() else {
        tracing::error!(
            target: "vertexlauncher/home",
            in_flight = state.server_ping_in_flight.len(),
            "Home server ping results receiver mutex was poisoned while polling ping results."
        );
        return;
    };

    while let Ok(result) = channel.rx.try_recv() {
        state.server_ping_in_flight.remove(result.address.as_str());
        state.server_pings.insert(result.address, result.snapshot);
    }
}

pub(super) fn collect_servers_from_request(request: &HomeActivityScanRequest) -> Vec<ServerEntry> {
    let mut servers = Vec::new();
    for instance in &request.instances {
        let servers_dat = instance.instance_root.join("servers.dat");
        let last_used_at_ms = modified_millis(servers_dat.as_path());
        let parsed = parse_servers_dat(servers_dat.as_path()).unwrap_or_default();
        for server in parsed {
            let favorite_id = normalize_server_address(server.ip.as_str());
            servers.push(ServerEntry {
                instance_id: instance.instance_id.clone(),
                server_name: server.name,
                address: server.ip.clone(),
                favorite_id: favorite_id.clone(),
                icon_png: decode_server_icon(server.icon.as_deref()),
                last_used_at_ms,
                favorite: instance
                    .favorite_server_ids
                    .iter()
                    .any(|id| id == &favorite_id),
                protocol_version: protocol_version_of(instance.instance_root.as_path(), instance.game_version.as_str()),
                instances: vec![EntryInstance {
                    instance_id: instance.instance_id.clone(),
                    instance_name: instance.instance_name.clone(),
                    target: server.ip.clone(),
                    favorite: instance
                        .favorite_server_ids
                        .iter()
                        .any(|id| id == &favorite_id),
                }],
            });
        }
    }
    servers.sort_by(|a, b| {
        b.last_used_at_ms
            .unwrap_or(0)
            .cmp(&a.last_used_at_ms.unwrap_or(0))
            .then_with(|| a.server_name.cmp(&b.server_name))
    });
    home_grouping::collapse_servers(servers)
}

pub(super) fn retain_known_server_pings(state: &mut HomeState) {
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

pub(super) fn queue_server_pings(state: &mut HomeState) {
    retain_known_server_pings(state);
    let mut stale_addresses: Vec<(String, String, Option<i32>)> = Vec::new();
    for server in &state.servers {
        let key = normalize_server_address(server.address.as_str());
        let stale = state
            .server_pings
            .get(&key)
            .is_none_or(|snapshot| snapshot.checked_at.elapsed() >= SERVER_PING_REFRESH_INTERVAL);
        if stale
            && !state.server_ping_in_flight.contains(&key)
            && !stale_addresses.iter().any(|(candidate, _, _)| candidate == &key)
        {
            stale_addresses.push((key, server.address.clone(), server.protocol_version));
        }
    }

    let Ok(channel) = server_ping_results().lock() else {
        tracing::error!(
            target: "vertexlauncher/home",
            queued_pings = stale_addresses.len(),
            "Home server ping results channel mutex was poisoned while scheduling server pings."
        );
        return;
    };
    let result_tx = channel.tx.clone();
    drop(channel);

    for (address, typed_address, protocol_version) in stale_addresses.into_iter().take(SERVER_PINGS_PER_SCAN) {
        state.server_ping_in_flight.insert(address.clone());
        let result_tx = result_tx.clone();
        // A ping can wait the game's 30 seconds for a server that does not answer, so it runs where blocking is fine.
        let _ = tokio_runtime::spawn_blocking_detached(move || {
            let snapshot = query_server_snapshot(typed_address.as_str(), protocol_version.unwrap_or(-1));
            if let Err(err) = result_tx.send(ServerPingResult {
                address: address.clone(),
                snapshot,
            }) {
                tracing::error!(
                    target: "vertexlauncher/home",
                    address = %address,
                    error = %err,
                    "Failed to deliver home server ping result."
                );
            }
        });
    }
}

pub(super) fn normalize_server_address(address: &str) -> String {
    instances::normalize_server_key(address)
}

/// Pings a server the way Minecraft's server list does (see `mc_server_ping`): `address` is the text exactly as the
/// game stores it, `protocol_version` is what the game sends in its handshake.
fn query_server_snapshot(address: &str, protocol_version: i32) -> ServerPingSnapshot {
    use mc_server_ping::{Outcome, Timeouts};
    let snapshot = |status, motd, players_online, players_max, players_sample| ServerPingSnapshot {
        status,
        motd,
        players_online,
        players_max,
        players_sample,
        checked_at: Instant::now(),
    };
    match mc_server_ping::ping(address, protocol_version, Timeouts::default()) {
        Outcome::InvalidAddress => snapshot(ServerPingStatus::Unknown, Vec::new(), None, None, Vec::new()),
        // Can't resolve, can't connect, or no answer the game would accept.
        Outcome::Unresolved | Outcome::ConnectFailed | Outcome::NoResponse => {
            snapshot(ServerPingStatus::Offline, Vec::new(), None, None, Vec::new())
        }
        Outcome::Modern { json, latency_ms } => match parse_status_json(json.as_str()) {
            Ok(status) => snapshot(
                ServerPingStatus::Online { latency_ms },
                status.motd,
                status.players_online,
                status.players_max,
                status.players_sample,
            ),
            Err(()) => snapshot(ServerPingStatus::Offline, Vec::new(), None, None, Vec::new()),
        },
        Outcome::Legacy { motd, online, max, .. } => {
            let spans = crate::mc_text::parse_legacy(&motd, crate::mc_text::Style::default());
            let lines = crate::mc_text::split_lines(&spans);
            // The legacy ping has no ping packet, so the game shows no latency for it either.
            snapshot(
                ServerPingStatus::Online { latency_ms: 0 },
                lines,
                u32::try_from(online).ok(),
                u32::try_from(max).ok(),
                Vec::new(),
            )
        }
    }
}

/// The protocol version of the client jar for `game_version` in `instance_root`, from the `version.json` inside it.
fn protocol_version_of(instance_root: &Path, game_version: &str) -> Option<i32> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<i32>>>> = OnceLock::new();
    let jar = instance_root.join("versions").join(game_version).join(format!("{game_version}.jar"));
    let cache = CACHE.get_or_init(Default::default);
    if let Some(found) = cache.lock().ok().and_then(|cache| cache.get(&jar).copied()) {
        return found;
    }
    let read = || -> Option<i32> {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&jar).ok()?).ok()?;
        let mut text = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("version.json").ok()?, &mut text).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        i32::try_from(value.get("protocol_version")?.as_i64()?).ok()
    };
    let protocol = read();
    // A jar that is not there yet is worth asking about again later; one without the field is not.
    if protocol.is_some() || jar.exists() {
        if let Ok(mut cache) = cache.lock() {
            cache.insert(jar, protocol);
        }
    }
    protocol
}

struct ParsedServerStatus {
    motd: Vec<Vec<crate::mc_text::Span>>,
    players_online: Option<u32>,
    players_max: Option<u32>,
    players_sample: Vec<String>,
}

fn parse_status_json(raw: &str) -> Result<ParsedServerStatus, ()> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| ())?;
    let motd = value.get("description").map(motd_lines).unwrap_or_default();
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
    let players_sample = value
        .get("players")
        .and_then(|players| players.get("sample"))
        .and_then(|sample| sample.as_array())
        .map(|sample| {
            sample
                .iter()
                .filter_map(|player| player.get("name")?.as_str())
                .map(|name| crate::mc_text::strip_codes(name).trim().to_owned())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(ParsedServerStatus {
        motd,
        players_online,
        players_max,
        players_sample,
    })
}

/// The status `description` as the game draws it: its lines, each as styled runs, with the blanks around each line (many
/// servers centre their MOTD with spaces) and empty lines at the ends left out.
fn motd_lines(description: &serde_json::Value) -> Vec<Vec<crate::mc_text::Span>> {
    let mut lines = crate::mc_text::split_lines(&crate::mc_text::from_component(description));
    for line in &mut lines {
        if let Some(first) = line.first_mut() {
            first.text = first.text.trim_start().to_owned();
        }
        if let Some(last) = line.last_mut() {
            last.text = last.text.trim_end().to_owned();
        }
        line.retain(|span| !span.text.is_empty());
    }
    while lines.last().is_some_and(Vec::is_empty) {
        lines.pop();
    }
    while lines.first().is_some_and(Vec::is_empty) {
        lines.remove(0);
    }
    lines
}

fn decode_server_icon(raw: Option<&str>) -> Option<Arc<[u8]>> {
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
    Some(crate::image_memory::prepare_owned_image_bytes_for_memory(decoded))
}



pub(super) fn server_meta_line(
    server: &ServerEntry,
    _ping: Option<&ServerPingSnapshot>,
    now_ms: u64,
) -> String {
    // The screen draws the MOTD (or says there is none) on its own, so this is only when the server was last used.
    tr!("last used {last_used}", last_used = format_time_ago(server.last_used_at_ms, now_ms)).into_owned()
}





