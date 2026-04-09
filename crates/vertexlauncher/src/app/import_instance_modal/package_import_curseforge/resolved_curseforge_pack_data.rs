use super::*;

#[derive(Debug)]
pub(crate) struct ResolvedCurseForgePackData {
    pub(super) dependency_info: MrpackDependencyInfo,
    pub(super) files: HashMap<u64, curseforge::File>,
    pub(super) projects: HashMap<u64, curseforge::Project>,
}
