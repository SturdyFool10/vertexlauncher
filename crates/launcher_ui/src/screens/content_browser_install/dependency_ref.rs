#[derive(Clone, Debug)]
pub(crate) enum DependencyRef {
    ModrinthProject(String),
    CurseForgeProject(u64),
}
