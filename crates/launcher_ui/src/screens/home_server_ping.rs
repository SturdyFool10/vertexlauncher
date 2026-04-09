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
    Some(prepare_owned_image_bytes_for_memory(decoded))
}

pub(super) fn home_server_icon_uri(instance_id: &str, favorite_id: &str) -> String {
    format!("bytes://home/server-icon/{instance_id}/{favorite_id}")
}

pub(super) fn server_meta_line(
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

pub(super) fn render_server_ping_icon(ui: &mut Ui, ping: Option<&ServerPingSnapshot>) {
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
