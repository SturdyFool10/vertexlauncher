use super::*;

#[derive(Debug, Clone, Default)]
pub struct DiscoverOutput {
    pub requested_screen: Option<AppScreen>,
    pub install_requested: Option<DiscoverInstallRequest>,
}
