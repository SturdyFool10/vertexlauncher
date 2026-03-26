use modprovider::ContentSource;

#[derive(Clone, Debug)]
pub struct InstalledContentIdentity {
    pub name: String,
    pub file_path: String,
    pub pack_managed: bool,
    pub source: ContentSource,
    pub modrinth_project_id: Option<String>,
    pub curseforge_project_id: Option<u64>,
    pub selected_version_id: String,
}
