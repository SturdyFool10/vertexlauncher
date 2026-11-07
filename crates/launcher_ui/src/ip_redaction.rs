//! Keeps IP addresses out of the console, whatever wrote the line: the launcher's own log, or a Minecraft session (which
//! prints the server it connects to, a player's address in a disconnect message, and so on). The console is what gets
//! screenshotted, copied and shared, so an address is replaced by [`PLACEHOLDER`] before a line is stored.
//!
//! Both IPv4 and IPv6 are found by regular expression and then checked in code, because a regex alone would also catch
//! things that only look like addresses: a four-part version (`v1.2.3.4`, `10.0.0.1.5`), an octet over 255, a Rust or
//! C++ path (`Cache::Entry`, `std::io`), a clock (`12:34:56`) or a MAC address (`aa:bb:cc:dd:ee:ff`). A port written after
//! an address (`127.0.0.1:25565`, `[::1]:25565`) goes with it.

use std::borrow::Cow;
use std::sync::OnceLock;

use regex::Regex;

/// What an address is replaced by.
pub const PLACEHOLDER: &str = "[IP hidden]";

fn ipv4() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d{1,3}(?:\.\d{1,3}){3}").expect("the IPv4 pattern is valid"))
}

/// IPv6 in its written forms: eight groups, any `::` compression, an embedded IPv4 tail (`::ffff:192.168.0.1`), and an
/// optional zone (`%eth0`). The first alternative that matches wins, so they are listed longest first: with a shorter
/// compression tried first, `fe80::1ff:fe23:4567:890a` would end at `fe80::1ff`.
fn ipv6() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let h = "[0-9A-Fa-f]{1,4}";
        let v4 = r"\d{1,3}(?:\.\d{1,3}){3}";
        // `n` hex groups joined by colons ("" for none).
        let groups = |n: usize| match n {
            0 => String::new(),
            1 => h.to_owned(),
            _ => format!("{h}(?::{h}){{{}}}", n - 1),
        };
        let mut alternatives: Vec<(usize, String)> = Vec::new();
        // Eight groups, and six groups then an IPv4 tail.
        alternatives.push((100, format!("{}:{v4}", groups(6))));
        alternatives.push((99, format!("{}", groups(8))));
        // `p` groups, `::`, `s` groups (p + s <= 7), optionally ending in an IPv4 address instead of the last two groups.
        for p in 0..=7usize {
            for s in 0..=7usize {
                if p + s > 7 || (p == 0 && s == 0) {
                    continue;
                }
                alternatives.push((p + s, format!("{}::{}", groups(p), groups(s))));
            }
        }
        for p in 0..=5usize {
            for s in 0..=(5 - p) {
                let tail = if s == 0 { v4.to_owned() } else { format!("{}:{v4}", groups(s)) };
                // Tried before the plain forms: it runs on past the hex groups into the dotted tail.
                alternatives.push((200 + p + s, format!("{}::{tail}", groups(p))));
            }
        }
        alternatives.sort_by(|a, b| b.0.cmp(&a.0));
        let joined = alternatives.into_iter().map(|(_, pattern)| pattern).collect::<Vec<_>>().join("|");
        Regex::new(&format!("(?:{joined})(?:%[0-9A-Za-z._~-]+)?")).expect("the IPv6 pattern is valid")
    })
}

fn char_before(text: &str, at: usize) -> Option<char> {
    text[..at].chars().next_back()
}

fn char_at(text: &str, at: usize) -> Option<char> {
    text[at..].chars().next()
}

/// Whether `ipv4` is four octets of 0-255.
fn octets_valid(candidate: &str) -> bool {
    candidate.split('.').all(|octet| octet.parse::<u16>().is_ok_and(|value| value <= 255))
}

/// Whether the match at `start..end` stands alone rather than inside a longer token.
fn stands_alone(text: &str, start: usize, end: usize) -> bool {
    let before_ok = char_before(text, start).is_none_or(|c| !c.is_alphanumeric() && c != '.' && c != '_');
    let after_ok = match char_at(text, end) {
        None => true,
        Some(c) if c.is_alphanumeric() || c == '_' => false,
        // "1.2.3.4.5" is not an address; "1.2.3.4." at the end of a sentence is.
        Some('.') => !char_at(text, end + 1).is_some_and(|c| c.is_ascii_digit()),
        Some(_) => true,
    };
    before_ok && after_ok
}

/// `text` with every IP address (and the port after it) replaced by [`PLACEHOLDER`]; the text itself, borrowed, when it has none.
pub fn redact_ip_addresses(text: &str) -> Cow<'_, str> {
    if !text.contains('.') && !text.contains(':') {
        return Cow::Borrowed(text);
    }
    // (start, end, may be followed by a port)
    let mut found: Vec<(usize, usize, bool)> = Vec::new();
    for m in ipv4().find_iter(text) {
        if octets_valid(m.as_str()) && stands_alone(text, m.start(), m.end()) {
            found.push((m.start(), m.end(), true));
        }
    }
    for m in ipv6().find_iter(text) {
        // "::" alone, or a hex-less run of colons, is punctuation; and a short `a::b` is far more likely a path than an
        // address (a real one has longer groups, or is the loopback `::1`).
        let looks_like_a_path = m.as_str().len() <= 4 && !m.as_str().starts_with(':') && !m.as_str().ends_with(':');
        if !m.as_str().chars().any(|c| c.is_ascii_hexdigit())
            || looks_like_a_path
            || !stands_alone(text, m.start(), m.end())
        {
            continue;
        }
        let bracketed = char_before(text, m.start()) == Some('[') && char_at(text, m.end()) == Some(']');
        found.push((m.start(), m.end(), bracketed));
    }
    if found.is_empty() {
        return Cow::Borrowed(text);
    }
    found.sort_unstable();

    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (start, end, port_follows) in found {
        if start < copied {
            // Inside one already replaced: the IPv4 tail of an IPv6 address.
            continue;
        }
        let (mut from, mut to) = (start, end);
        // A bracketed IPv6 address takes its brackets with it.
        if char_before(text, from) == Some('[') && char_at(text, to) == Some(']') {
            from -= 1;
            to += 1;
        }
        // Then the port: `:25565`.
        if port_follows && char_at(text, to) == Some(':') {
            let digits = text[to + 1..].chars().take_while(char::is_ascii_digit).count();
            if (1..=5).contains(&digits) && !char_at(text, to + 1 + digits).is_some_and(char::is_alphanumeric) {
                to += 1 + digits;
            }
        }
        out.push_str(&text[copied..from.max(copied)]);
        out.push_str(PLACEHOLDER);
        copied = to;
    }
    out.push_str(&text[copied..]);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redacted(text: &str) -> String {
        redact_ip_addresses(text).into_owned()
    }

    #[test]
    fn ipv4_addresses_and_their_ports_are_hidden() {
        assert_eq!(redacted("Connecting to /127.0.0.1:25565"), "Connecting to /[IP hidden]");
        assert_eq!(redacted("lost connection: 203.0.113.7."), "lost connection: [IP hidden].");
        assert_eq!(redacted("a 10.0.0.1 b 192.168.1.20:8080 c"), "a [IP hidden] b [IP hidden] c");
        assert_eq!(redacted("ip=255.255.255.255"), "ip=[IP hidden]");
    }

    #[test]
    fn ipv6_addresses_in_every_written_form_are_hidden() {
        for address in [
            "::1",
            "fe80::1ff:fe23:4567:890a",
            "fe80::1ff:fe23:4567:890a%eth0",
            "2001:db8::8a2e:370:7334",
            "2001:0db8:0000:0000:0000:ff00:0042:8329",
            "2001:db8::",
            "::ffff:192.168.0.1",
            "64:ff9b::192.0.2.33",
        ] {
            assert_eq!(redacted(&format!("peer {address} gone")), "peer [IP hidden] gone", "{address}");
        }
        assert_eq!(redacted("server [2001:db8::1]:25565 refused"), "server [IP hidden] refused");
        assert_eq!(redacted("ip:2001:db8::1"), "ip:[IP hidden]");
    }

    #[test]
    fn things_that_only_look_like_addresses_are_left_alone() {
        for text in [
            "Minecraft 1.20.4 on Java 21.0.5",
            "version v1.2.3.4",
            "10.0.0.1.5",
            "999.1.1.1",
            "256.256.256.256",
            "launcher_ui::console::Cache::Entry",
            "std::io::Error",
            "at 12:34:56 and 12:34:56.789",
            "mac aa:bb:cc:dd:ee:ff",
            "[13:45:01] [Render thread/INFO]: Reloading",
            "x::y and a::b",
            "nothing to see",
        ] {
            assert_eq!(redacted(text), text, "{text}");
        }
    }

    #[test]
    fn an_address_that_repeats_with_different_values_is_the_same_line_afterwards() {
        assert_eq!(redacted("Connecting to 10.0.0.1:25565"), redacted("Connecting to 10.9.9.9:25566"));
    }

    #[test]
    fn a_line_without_an_address_is_not_copied() {
        assert!(matches!(redact_ip_addresses("plain text"), Cow::Borrowed(_)));
        assert!(matches!(redact_ip_addresses("only 1.2.3 here"), Cow::Borrowed(_)));
    }
}
