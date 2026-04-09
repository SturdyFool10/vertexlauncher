#[derive(Clone, Debug)]
pub(crate) enum DiscoverProjectRef {
    Modrinth { project_id: String },
    CurseForge { project_id: u64 },
}
