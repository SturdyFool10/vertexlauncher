use super::*;

#[derive(Clone, Debug)]
pub(crate) struct DiscoverProviderRef {
    pub(crate) source: DiscoverSource,
    pub(crate) project_ref: DiscoverProjectRef,
    pub(crate) primary_url: Option<String>,
}
