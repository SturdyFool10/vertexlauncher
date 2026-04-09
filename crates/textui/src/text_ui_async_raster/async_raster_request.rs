use super::*;

#[derive(Clone, Debug)]
pub(crate) struct AsyncRasterRequest {
    pub(crate) key_hash: u64,
    pub(crate) kind: AsyncRasterKind,
    pub(crate) options: LabelOptions,
    pub(crate) width_points_opt: Option<f32>,
    pub(crate) scale: f32,
    pub(crate) typography: TypographySnapshot,
}
