use super::*;

#[derive(Clone, Debug)]
pub(crate) struct DiscoverMasonryLayout {
    pub(crate) columns: Vec<DiscoverMasonryColumn>,
    pub(crate) content_width: f32,
    pub(crate) column_width: f32,
}
