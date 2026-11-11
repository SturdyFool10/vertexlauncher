//! Server addresses the way Minecraft reads them: Guava's `HostAndPort.fromString(..).withDefaultPort(25565)`, then
//! `IDN.toASCII` on the host. Nothing is trimmed or lowercased, exactly like the game.

/// The port used when an address has none.
pub const DEFAULT_PORT: u16 = 25565;

/// A parsed server address. `host` is as typed (brackets removed); use [`ServerAddress::ascii_host`] for what goes on the
/// wire and into DNS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

impl ServerAddress {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self { host: host.into(), port }
    }

    /// Minecraft's `ServerAddress.parseString`: `None` is its `INVALID` address (`server.invalid`), which never resolves.
    pub fn parse(text: &str) -> Option<Self> {
        let (host, port) = host_and_port(text)?;
        if host.is_empty() {
            return None;
        }
        Some(Self { host, port: port.unwrap_or(DEFAULT_PORT) })
    }

    /// `ServerAddress.getHost`: the host through `IDN.toASCII`, or an empty string when that rejects it.
    pub fn ascii_host(&self) -> String {
        to_ascii(&self.host).unwrap_or_default()
    }
}

/// Guava's `HostAndPort.fromString`: the host and the port if one was given, or `None` where Guava throws.
fn host_and_port(text: &str) -> Option<(String, Option<u16>)> {
    let (host, port_text): (String, Option<String>) = if text.starts_with('[') {
        let colon = text.find(':')?;
        let close = text.rfind(']')?;
        if close <= colon {
            return None;
        }
        let host = text[1..close].to_owned();
        if close + 1 == text.len() {
            (host, None)
        } else {
            let rest = &text[close + 1..];
            let digits = rest.strip_prefix(':')?;
            if !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            (host, Some(digits.to_owned()))
        }
    } else {
        match text.find(':') {
            Some(first) if !text[first + 1..].contains(':') => (text[..first].to_owned(), Some(text[first + 1..].to_owned())),
            _ => (text.to_owned(), None),
        }
    };
    let port = match port_text.filter(|port| !port.is_empty()) {
        None => None,
        Some(port) => {
            if port.starts_with('+') || !port.is_ascii() {
                return None;
            }
            // Java's Integer.parseInt: an optional sign and digits, within i32; then the port range check.
            let value: i32 = port.parse().ok()?;
            Some(u16::try_from(value).ok()?)
        }
    };
    Some((host, port))
}

/// Java's `IDN.toASCII(host)`: an error where it throws `IllegalArgumentException`.
pub fn to_ascii(host: &str) -> Result<String, ()> {
    if host == "." {
        return Ok(".".to_owned());
    }
    let labels_ok = |labels: &str| {
        let mut parts: Vec<&str> = labels.split(['.', '\u{3002}', '\u{ff0e}', '\u{ff61}']).collect();
        // A trailing dot ends the name; it is not an empty label.
        if parts.len() > 1 && parts.last() == Some(&"") {
            parts.pop();
        }
        parts.iter().all(|label| !label.is_empty() && label.len() <= 63)
    };
    if host.is_empty() {
        return Ok(String::new());
    }
    if host.is_ascii() {
        return if labels_ok(host) { Ok(host.to_owned()) } else { Err(()) };
    }
    let ascii = idna::domain_to_ascii(host).map_err(|_| ())?;
    if labels_ok(&ascii) { Ok(ascii) } else { Err(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Option<(String, u16)> {
        ServerAddress::parse(text).map(|address| (address.host, address.port))
    }

    #[test]
    fn plain_hosts_get_the_default_port() {
        assert_eq!(parsed("play.example.com"), Some(("play.example.com".into(), 25565)));
        assert_eq!(parsed("example.com:25570"), Some(("example.com".into(), 25570)));
        assert_eq!(parsed("example.com:"), Some(("example.com".into(), 25565)));
        assert_eq!(parsed("127.0.0.1:1"), Some(("127.0.0.1".into(), 1)));
    }

    #[test]
    fn ipv6_follows_guava() {
        assert_eq!(parsed("[::1]"), Some(("::1".into(), 25565)));
        assert_eq!(parsed("[::1]:25566"), Some(("::1".into(), 25566)));
        // Several colons and no brackets: the whole text is the host.
        assert_eq!(parsed("::1"), Some(("::1".into(), 25565)));
        assert_eq!(parsed("fe80::1"), Some(("fe80::1".into(), 25565)));
        assert_eq!(parsed("[::1]x"), None);
        assert_eq!(parsed("[::1]:a"), None);
        assert_eq!(parsed("[nocolon]"), None);
    }

    #[test]
    fn invalid_ports_and_empty_hosts_are_invalid() {
        assert_eq!(parsed(""), None);
        assert_eq!(parsed(":25565"), None);
        assert_eq!(parsed("host:65536"), None);
        assert_eq!(parsed("host:-1"), None);
        assert_eq!(parsed("host:+1"), None);
        assert_eq!(parsed("host:abc"), None);
        assert_eq!(parsed("host:99999999999"), None);
        assert_eq!(parsed("host:0"), Some(("host".into(), 0)));
    }

    #[test]
    fn nothing_is_trimmed_or_lowercased() {
        assert_eq!(parsed(" host"), Some((" host".into(), 25565)));
        assert_eq!(parsed("Play.Example.COM"), Some(("Play.Example.COM".into(), 25565)));
    }

    #[test]
    fn hosts_go_through_idn() {
        assert_eq!(ServerAddress::new("bücher.example", 25565).ascii_host(), "xn--bcher-kva.example");
        assert_eq!(ServerAddress::new("host.", 25565).ascii_host(), "host.");
        assert_eq!(ServerAddress::new("a..b", 25565).ascii_host(), "");
        assert_eq!(ServerAddress::new("x".repeat(64), 25565).ascii_host(), "");
        assert_eq!(ServerAddress::new("a_b.example", 25565).ascii_host(), "a_b.example");
    }
}
