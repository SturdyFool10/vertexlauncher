use vertex_i18n::tr;
use std::io::Write;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};

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
        tracing::error!(
            target: "vertexlauncher/home",
            queued_pings = stale_addresses.len(),
            "Home server ping results channel mutex was poisoned while scheduling server pings."
        );
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
        motd: Vec::new(),
        players_online: None,
        players_max: None,
        players_sample: Vec::new(),
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
                motd: Vec::new(),
                players_online: None,
                players_max: None,
                players_sample: Vec::new(),
                checked_at: Instant::now(),
            };
        }
    };
    let _ = stream.set_read_timeout(Some(SERVER_PING_CONNECT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SERVER_PING_CONNECT_TIMEOUT));

    let start = Instant::now();
    match request_server_status(&mut stream, host.as_str(), port) {
        Ok(status) => ServerPingSnapshot {
            status: ServerPingStatus::Online {
                latency_ms: start.elapsed().as_millis() as u64,
            },
            motd: status.motd,
            players_online: status.players_online,
            players_max: status.players_max,
            players_sample: status.players_sample,
            checked_at: Instant::now(),
        },
        Err(_) => ServerPingSnapshot {
            status: ServerPingStatus::Online {
                latency_ms: start.elapsed().as_millis() as u64,
            },
            motd: Vec::new(),
            players_online: None,
            players_max: None,
            players_sample: Vec::new(),
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
) -> Result<ParsedServerStatus, ()> {
    send_handshake_packet(stream, host, port)?;
    send_status_request_packet(stream)?;
    let json = read_status_response_packet(stream)?;
    parse_status_json(json.as_str())
}

fn send_handshake_packet(stream: &mut TcpStream, host: &str, port: u16) -> Result<(), ()> {
    let mut payload = Vec::new();
    write_varint(&mut payload, 0);
    write_varint_i32(&mut payload, -1);
    write_mc_string(&mut payload, host)?;
    payload.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut payload, 1);
    write_framed_packet(stream, &payload)
}

fn send_status_request_packet(stream: &mut TcpStream) -> Result<(), ()> {
    write_framed_packet(stream, &[0])
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





