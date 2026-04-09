use super::*;

pub(crate) enum HomeEntryRef<'a> {
    World(&'a WorldEntry),
    Server(&'a ServerEntry),
}

impl HomeEntryRef<'_> {
    pub(crate) fn last_used_at_ms(&self) -> Option<u64> {
        match self {
            Self::World(world) => world.last_used_at_ms,
            Self::Server(server) => server.last_used_at_ms,
        }
    }

    pub(crate) fn primary_label(&self) -> &str {
        match self {
            Self::World(world) => world.world_name.as_str(),
            Self::Server(server) => server.server_name.as_str(),
        }
    }
}
