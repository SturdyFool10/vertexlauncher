//! SRV lookups through the system's DNS configuration, like the game's JNDI DNS client: one retry, and the name is queried
//! exactly as written (the search list is not applied).

use std::time::Duration;

use hickory_resolver::TokioResolver;
use hickory_resolver::config::ResolverOpts;
use hickory_resolver::proto::rr::RData;

use crate::resolve::{SrvRecord, SrvResolver};

/// The first SRV record in the answer, in the order the server sent them.
pub struct SystemSrv;

impl SrvResolver for SystemSrv {
    fn first_srv(&self, name: &str) -> Option<SrvRecord> {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
        runtime.block_on(async {
            let mut builder = TokioResolver::builder_tokio().ok()?;
            let options: &mut ResolverOpts = builder.options_mut();
            options.timeout = Duration::from_secs(1);
            options.attempts = 2;
            options.ndots = 0;
            // No search domains: `_minecraft._tcp.host.` is fully qualified.
            options.cache_size = 0;
            let resolver = builder.build();
            let lookup = match resolver.srv_lookup(name).await {
                Ok(lookup) => lookup,
                Err(err) => {
                    tracing::debug!(target: "mc_server_ping", name, error = %err, "SRV lookup found nothing");
                    return None;
                }
            };
            // The records as received: `srv_lookup` would sort them by priority and weight, which the game does not do.
            lookup.as_lookup().record_iter().find_map(|record| match record.data() {
                RData::SRV(srv) => Some(SrvRecord {
                    priority: srv.priority(),
                    weight: srv.weight(),
                    port: srv.port(),
                    target: srv.target().to_string(),
                }),
                _ => None,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs network access: `cargo test -p mc_server_ping -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn the_system_resolver_reads_srv_records() {
        for name in ["_imaps._tcp.gmail.com.", "_minecraft._tcp.mc.hypixel.net.", "_minecraft._tcp.play.cubecraft.net.", "_minecraft._tcp.example.com."] {
            println!("{name} -> {:?}", SystemSrv.first_srv(name));
        }
    }
}
