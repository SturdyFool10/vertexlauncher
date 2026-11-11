//! `ServerStatusPinger`: connect, handshake, status request, ping request, and the legacy ping when the modern one fails
//! after connecting. The handshake carries the *typed* address (not the SRV target), exactly like the game's server list.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::address::ServerAddress;
use crate::dns::SystemSrv;
use crate::resolve::{HostResolver, SrvResolver, SystemHosts, resolve_address};

/// Netty's default connect timeout, and the game's `ReadTimeoutHandler(30)`.
pub const GAME_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub connect: Duration,
    pub read: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self { connect: GAME_TIMEOUT, read: GAME_TIMEOUT }
    }
}

/// What pinging a server found.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The address is not valid (the game's `server.invalid`).
    InvalidAddress,
    /// "Can't resolve hostname".
    Unresolved,
    /// "Can't connect to server": the connection was refused or timed out.
    ConnectFailed,
    /// Connected, but neither the status protocol nor the legacy ping worked.
    NoResponse,
    /// A modern status: the raw JSON and the round trip of the ping request.
    Modern { json: String, latency_ms: u64 },
    /// An answer from the pre-1.7 ping.
    Legacy { protocol: i32, version: String, motd: String, online: i32, max: i32 },
}

/// Pings `address_text` with the system's resolvers. `protocol_version` is what the game itself would send.
pub fn ping(address_text: &str, protocol_version: i32, timeouts: Timeouts) -> Outcome {
    ping_with(address_text, protocol_version, timeouts, &SystemHosts, &SystemSrv)
}

pub fn ping_with(
    address_text: &str,
    protocol_version: i32,
    timeouts: Timeouts,
    hosts: &dyn HostResolver,
    srv: &dyn SrvResolver,
) -> Outcome {
    let Some(address) = ServerAddress::parse(address_text) else {
        return Outcome::InvalidAddress;
    };
    let Some(target) = resolve_address(&address, hosts, srv) else {
        return Outcome::Unresolved;
    };
    let Some(mut stream) = connect(target, timeouts) else {
        return Outcome::ConnectFailed;
    };
    match modern_ping(&mut stream, &address, protocol_version) {
        Some((json, latency_ms)) => Outcome::Modern { json, latency_ms },
        None => {
            drop(stream);
            // The game falls back to the legacy ping on a new connection.
            match connect(target, timeouts).and_then(|mut stream| legacy_ping(&mut stream, &address)) {
                Some(legacy) => legacy,
                None => Outcome::NoResponse,
            }
        }
    }
}

fn connect(target: SocketAddr, timeouts: Timeouts) -> Option<TcpStream> {
    let stream = TcpStream::connect_timeout(&target, timeouts.connect).ok()?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(timeouts.read));
    let _ = stream.set_write_timeout(Some(timeouts.read));
    Some(stream)
}

// ---------------------------------------------------------------------------------------------------------------------
// Modern status protocol
// ---------------------------------------------------------------------------------------------------------------------

fn write_varint(out: &mut Vec<u8>, value: i32) {
    let mut value = value as u32;
    loop {
        if value & !0x7f == 0 {
            out.push(value as u8);
            return;
        }
        out.push((value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
}

fn read_varint(stream: &mut impl Read, max_bytes: usize) -> Option<i32> {
    let mut result = 0u32;
    for index in 0..max_bytes {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).ok()?;
        result |= u32::from(byte[0] & 0x7f) << (7 * index);
        if byte[0] & 0x80 == 0 {
            return Some(result as i32);
        }
    }
    None
}

fn write_string(out: &mut Vec<u8>, text: &str) {
    write_varint(out, text.len() as i32);
    out.extend_from_slice(text.as_bytes());
}

fn send_packet(stream: &mut impl Write, payload: &[u8]) -> Option<()> {
    let mut framed = Vec::with_capacity(payload.len() + 3);
    write_varint(&mut framed, payload.len() as i32);
    framed.extend_from_slice(payload);
    stream.write_all(&framed).ok()?;
    stream.flush().ok()
}

/// One packet: its id and body. The game's frame decoder allows a three byte length.
fn read_packet(stream: &mut impl Read) -> Option<(i32, Vec<u8>)> {
    let length = read_varint(stream, 3)?;
    if length <= 0 {
        return None;
    }
    let mut body = vec![0u8; usize::try_from(length).ok()?];
    stream.read_exact(&mut body).ok()?;
    let mut cursor = std::io::Cursor::new(&body);
    let id = read_varint(&mut cursor, 5)?;
    let rest = body[cursor.position() as usize..].to_vec();
    Some((id, rest))
}

fn handshake(address: &ServerAddress, protocol_version: i32) -> Vec<u8> {
    let mut payload = Vec::new();
    write_varint(&mut payload, 0);
    write_varint(&mut payload, protocol_version);
    write_string(&mut payload, &address.ascii_host());
    payload.extend_from_slice(&address.port.to_be_bytes());
    // The status intent.
    write_varint(&mut payload, 1);
    payload
}

fn millis_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// The status JSON and the ping round trip. Like the game, a status without a pong is a failure.
fn modern_ping(stream: &mut TcpStream, address: &ServerAddress, protocol_version: i32) -> Option<(String, u64)> {
    send_packet(stream, &handshake(address, protocol_version))?;
    send_packet(stream, &[0])?;
    let (id, body) = read_packet(stream)?;
    if id != 0 {
        return None;
    }
    let mut cursor = std::io::Cursor::new(&body);
    let length = usize::try_from(read_varint(&mut cursor, 3)?).ok()?;
    let start = cursor.position() as usize;
    let json = String::from_utf8(body.get(start..start.checked_add(length)?)?.to_vec()).ok()?;
    // The status must be a JSON object; fields the game does not understand are ignored.
    if !serde_json::from_str::<serde_json::Value>(&json).ok()?.is_object() {
        return None;
    }

    let sent = Instant::now();
    let mut ping = Vec::new();
    write_varint(&mut ping, 1);
    ping.extend_from_slice(&millis_now().to_be_bytes());
    send_packet(stream, &ping)?;
    let (pong_id, _) = read_packet(stream)?;
    if pong_id != 1 {
        return None;
    }
    Some((json, sent.elapsed().as_millis() as u64))
}

// ---------------------------------------------------------------------------------------------------------------------
// Legacy (pre-1.7) ping
// ---------------------------------------------------------------------------------------------------------------------

fn write_legacy_string(out: &mut Vec<u8>, text: &str) {
    let units: Vec<u16> = text.encode_utf16().collect();
    out.extend_from_slice(&(units.len() as i16).to_be_bytes());
    for unit in units {
        out.extend_from_slice(&unit.to_be_bytes());
    }
}

fn legacy_ping(stream: &mut TcpStream, address: &ServerAddress) -> Option<Outcome> {
    let mut request = vec![0xFE, 0x01, 0xFA];
    write_legacy_string(&mut request, "MC|PingHost");
    let mut data = vec![127u8];
    write_legacy_string(&mut data, &address.ascii_host());
    data.extend_from_slice(&i32::from(address.port).to_be_bytes());
    request.extend_from_slice(&(data.len() as i16).to_be_bytes());
    request.extend_from_slice(&data);
    stream.write_all(&request).ok()?;
    stream.flush().ok()?;

    let mut kind = [0u8; 1];
    stream.read_exact(&mut kind).ok()?;
    if kind[0] != 0xFF {
        return None;
    }
    let mut length = [0u8; 2];
    stream.read_exact(&mut length).ok()?;
    let units = usize::try_from(i16::from_be_bytes(length)).ok()?;
    let mut raw = vec![0u8; units * 2];
    stream.read_exact(&mut raw).ok()?;
    let text = String::from_utf16(&raw.chunks_exact(2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect::<Vec<_>>()).ok()?;
    parse_legacy_response(&text)
}

/// Guava `Splitter.on('\0').limit(6)`, then the game's reading of the parts: only the `§1` format is understood.
pub fn parse_legacy_response(text: &str) -> Option<Outcome> {
    let parts: Vec<&str> = text.splitn(6, '\0').collect();
    if parts.first() != Some(&"\u{a7}1") || parts.len() < 6 {
        return None;
    }
    let number = |text: &str, default: i32| text.trim().parse::<i32>().unwrap_or(default);
    Some(Outcome::Legacy {
        protocol: number(parts[1], 0),
        version: parts[2].to_owned(),
        motd: parts[3].to_owned(),
        online: number(parts[4], -1),
        max: number(parts[5], -1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn varints_round_trip() {
        for value in [0, 1, 127, 128, 255, 25565, 777, 2_097_151, -1] {
            let mut bytes = Vec::new();
            write_varint(&mut bytes, value);
            assert_eq!(read_varint(&mut bytes.as_slice(), 5), Some(value), "{value}");
        }
    }

    #[test]
    fn the_handshake_carries_the_typed_host_and_port_and_the_status_intent() {
        let address = ServerAddress::parse("Play.Example.com:25570").unwrap();
        let payload = handshake(&address, 777);
        let mut expected = vec![0u8];
        write_varint(&mut expected, 777);
        write_string(&mut expected, "Play.Example.com");
        expected.extend_from_slice(&25570u16.to_be_bytes());
        expected.push(1);
        assert_eq!(payload, expected);
    }

    #[test]
    fn legacy_responses_follow_the_game() {
        let text = "\u{a7}1\u{0}127\u{0}1.6.4\u{0}A Server\u{0}3\u{0}20";
        assert_eq!(
            parse_legacy_response(text),
            Some(Outcome::Legacy { protocol: 127, version: "1.6.4".into(), motd: "A Server".into(), online: 3, max: 20 })
        );
        assert_eq!(parse_legacy_response("A Server\u{a7}3\u{a7}20"), None);
        assert_eq!(parse_legacy_response("\u{a7}1\u{0}127\u{0}1.6.4"), None);
    }

    struct Fixed(std::net::IpAddr);
    impl HostResolver for Fixed {
        fn resolve(&self, _: &str) -> Option<std::net::IpAddr> {
            Some(self.0)
        }
    }
    struct NoSrv;
    impl SrvResolver for NoSrv {
        fn first_srv(&self, _: &str) -> Option<crate::resolve::SrvRecord> {
            None
        }
    }

    /// A tiny server speaking the status protocol.
    fn serve(status: &'static str, answer_ping: bool) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (id, handshake) = read_packet(&mut stream).unwrap();
            assert_eq!(id, 0);
            // protocol, host, port, intent
            let mut cursor = std::io::Cursor::new(&handshake);
            assert_eq!(read_varint(&mut cursor, 5), Some(777));
            let (id, _) = read_packet(&mut stream).unwrap();
            assert_eq!(id, 0);
            let mut response = Vec::new();
            write_varint(&mut response, 0);
            write_string(&mut response, status);
            send_packet(&mut stream, &response).unwrap();
            if answer_ping {
                let (id, body) = read_packet(&mut stream).unwrap();
                assert_eq!(id, 1);
                let mut pong = Vec::new();
                write_varint(&mut pong, 1);
                pong.extend_from_slice(&body);
                send_packet(&mut stream, &pong).unwrap();
            }
        });
        port
    }

    #[test]
    fn a_status_with_a_pong_is_online() {
        let port = serve(r#"{"description":"hi","players":{"online":1,"max":2}}"#, true);
        let outcome = ping_with(&format!("localhost:{port}"), 777, Timeouts::default(), &Fixed("127.0.0.1".parse().unwrap()), &NoSrv);
        assert!(matches!(outcome, Outcome::Modern { ref json, .. } if json.contains("hi")), "{outcome:?}");
    }

    #[test]
    fn a_status_without_a_pong_is_not_online() {
        let port = serve(r#"{"description":"hi"}"#, false);
        let timeouts = Timeouts { connect: Duration::from_secs(2), read: Duration::from_millis(300) };
        let outcome = ping_with(&format!("localhost:{port}"), 777, timeouts, &Fixed("127.0.0.1".parse().unwrap()), &NoSrv);
        assert_eq!(outcome, Outcome::NoResponse);
    }

    #[test]
    fn bad_addresses_and_closed_ports_are_told_apart() {
        let fixed = Fixed("127.0.0.1".parse().unwrap());
        assert_eq!(ping_with("", 777, Timeouts::default(), &fixed, &NoSrv), Outcome::InvalidAddress);
        // Nothing listens on port 1.
        let timeouts = Timeouts { connect: Duration::from_millis(500), read: Duration::from_millis(500) };
        assert_eq!(ping_with("localhost:1", 777, timeouts, &fixed, &NoSrv), Outcome::ConnectFailed);
    }
}
