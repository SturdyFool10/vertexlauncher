use super::*;

#[derive(Clone, Debug)]
pub(crate) struct DiscoverMasonryColumn {
    pub(crate) items: Vec<DiscoverMasonryItem>,
    pub(crate) total_height: f32,
}
