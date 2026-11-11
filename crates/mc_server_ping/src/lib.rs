//! Pinging a Minecraft server exactly as the game's server list does, decompiled from the client: address parsing,
//! `IDN`, the single-address lookup, the `_minecraft._tcp` SRV redirect, the handshake, the status and ping requests, and
//! the legacy fallback.

mod address;
mod dns;
mod ping;
mod resolve;

pub use address::{DEFAULT_PORT, ServerAddress, to_ascii};
pub use dns::SystemSrv;
pub use ping::{GAME_TIMEOUT, Outcome, Timeouts, parse_legacy_response, ping, ping_with};
pub use resolve::{HostResolver, SrvRecord, SrvResolver, SystemHosts, parse_srv_text, resolve_address};
