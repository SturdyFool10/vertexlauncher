//! `ServerNameResolver.resolveAddress`, step for step.
//!
//! 1. The address's own host is resolved first (`InetAddress.getByName`, which yields one address, IPv4 preferred). If that
//!    fails the address is unresolvable, even when an SRV record exists.
//! 2. Only an address whose port is exactly 25565 gets an SRV lookup, for `_minecraft._tcp.<host>`, and only the first
//!    record the DNS server returned is used (no priority or weight ordering). Its port is `Integer.parseInt`, else 25565.
//! 3. When an SRV record exists, the connection goes to its target, resolved the same way. If the target does not resolve
//!    the address is unresolvable: there is no fall back to step 1's result.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};

use crate::address::{DEFAULT_PORT, ServerAddress};

/// Turns a host name into one address (Java's `InetAddress.getByName`).
pub trait HostResolver {
    fn resolve(&self, host: &str) -> Option<IpAddr>;
}

/// Looks up the SRV records of a name and returns the first record as DNS gave it.
pub trait SrvResolver {
    fn first_srv(&self, name: &str) -> Option<SrvRecord>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SrvRecord {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

/// The operating system's resolver, like `InetAddress.getByName`.
pub struct SystemHosts;

impl HostResolver for SystemHosts {
    fn resolve(&self, host: &str) -> Option<IpAddr> {
        // `getByName("")` is the loopback address.
        if host.is_empty() {
            return Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
        }
        let addresses: Vec<SocketAddr> = (host, 0).to_socket_addrs().ok()?.collect();
        // Java lists IPv4 addresses first unless told to prefer IPv6.
        addresses.iter().find(|address| address.is_ipv4()).or_else(|| addresses.first()).map(SocketAddr::ip)
    }
}

/// The address to connect to for `address`, or `None` where Minecraft says it can't resolve the hostname.
pub fn resolve_address(address: &ServerAddress, hosts: &dyn HostResolver, srv: &dyn SrvResolver) -> Option<SocketAddr> {
    let own = hosts.resolve(&address.ascii_host())?;
    let redirect = if address.port == DEFAULT_PORT { redirect_for(address, srv) } else { None };
    match redirect {
        Some(target) => hosts.resolve(&target.ascii_host()).map(|ip| SocketAddr::new(ip, target.port)),
        None => Some(SocketAddr::new(own, address.port)),
    }
}

/// `ServerRedirectHandler`'s SRV redirect: `Some` only when the first SRV record is usable.
fn redirect_for(address: &ServerAddress, srv: &dyn SrvResolver) -> Option<ServerAddress> {
    let host = address.ascii_host();
    // The name is queried as written: no search domains are tried.
    let name = if host.ends_with('.') { format!("_minecraft._tcp.{host}") } else { format!("_minecraft._tcp.{host}.") };
    let record = srv.first_srv(&name)?;
    Some(ServerAddress::new(record.target, record.port))
}

/// The target of an SRV record as Minecraft reads it from the JNDI text `"priority weight port target"` (split into four).
pub fn parse_srv_text(text: &str) -> Option<SrvRecord> {
    let mut parts = text.splitn(4, ' ');
    let priority = parts.next()?;
    let weight = parts.next()?;
    let port = parts.next()?;
    let target = parts.next()?;
    Some(SrvRecord {
        priority: priority.trim().parse().unwrap_or(0),
        weight: weight.trim().parse().unwrap_or(0),
        // `ServerAddress.parsePort`: any failure means 25565.
        port: port.trim().parse::<i32>().ok().and_then(|port| u16::try_from(port).ok()).unwrap_or(DEFAULT_PORT),
        target: target.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Hosts(HashMap<&'static str, IpAddr>);
    impl HostResolver for Hosts {
        fn resolve(&self, host: &str) -> Option<IpAddr> {
            self.0.get(host).copied()
        }
    }

    struct Srv(HashMap<&'static str, SrvRecord>);
    impl SrvResolver for Srv {
        fn first_srv(&self, name: &str) -> Option<SrvRecord> {
            self.0.get(name).cloned()
        }
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn setup() -> (Hosts, Srv) {
        let hosts = Hosts(HashMap::from([("play.example.com", ip("1.1.1.1")), ("node.host.net", ip("2.2.2.2")), ("dead.example.com", ip("3.3.3.3"))]));
        let srv = Srv(HashMap::from([
            ("_minecraft._tcp.play.example.com.", SrvRecord { priority: 0, weight: 5, port: 25789, target: "node.host.net.".into() }),
            ("_minecraft._tcp.dead.example.com.", SrvRecord { priority: 0, weight: 5, port: 25789, target: "missing.host.net".into() }),
        ]));
        (hosts, srv)
    }

    #[test]
    fn a_default_port_address_follows_its_srv_record() {
        let (hosts, srv) = setup();
        let address = ServerAddress::parse("play.example.com").unwrap();
        // The target is looked up with its trailing dot, as the DNS answer wrote it.
        let hosts = Hosts(hosts.0.into_iter().chain([("node.host.net.", ip("2.2.2.2"))]).collect());
        assert_eq!(resolve_address(&address, &hosts, &srv), Some("2.2.2.2:25789".parse().unwrap()));
    }

    #[test]
    fn an_explicit_other_port_skips_srv() {
        let (hosts, srv) = setup();
        let address = ServerAddress::parse("play.example.com:25566").unwrap();
        assert_eq!(resolve_address(&address, &hosts, &srv), Some("1.1.1.1:25566".parse().unwrap()));
    }

    #[test]
    fn an_explicit_25565_still_follows_srv() {
        let (hosts, srv) = setup();
        let hosts = Hosts(hosts.0.into_iter().chain([("node.host.net.", ip("2.2.2.2"))]).collect());
        let address = ServerAddress::parse("play.example.com:25565").unwrap();
        assert_eq!(resolve_address(&address, &hosts, &srv), Some("2.2.2.2:25789".parse().unwrap()));
    }

    #[test]
    fn an_unresolvable_srv_target_fails_without_falling_back() {
        let (hosts, srv) = setup();
        let address = ServerAddress::parse("dead.example.com").unwrap();
        assert_eq!(resolve_address(&address, &hosts, &srv), None);
    }

    #[test]
    fn a_host_that_does_not_resolve_fails_even_with_srv() {
        let (hosts, srv) = setup();
        let srv = Srv(srv.0.into_iter().chain([("_minecraft._tcp.only-srv.example.", SrvRecord { priority: 0, weight: 0, port: 1, target: "node.host.net".into() })]).collect());
        let address = ServerAddress::parse("only-srv.example").unwrap();
        assert_eq!(resolve_address(&address, &hosts, &srv), None);
    }

    #[test]
    fn srv_text_follows_minecrafts_parsing() {
        let record = parse_srv_text("10 5 25789 node.host.net.").unwrap();
        assert_eq!((record.port, record.target.as_str()), (25789, "node.host.net."));
        assert_eq!(parse_srv_text("0 0 notaport target.example").unwrap().port, 25565);
        assert_eq!(parse_srv_text("0 0 99999 target.example").unwrap().port, 25565);
        assert!(parse_srv_text("0 0 25565").is_none());
    }
}
