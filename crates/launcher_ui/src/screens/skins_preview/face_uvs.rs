use super::*;

#[derive(Clone, Copy)]
pub(crate) struct FaceUvs {
    pub(crate) top: Rect,
    pub(crate) bottom: Rect,
    pub(crate) left: Rect,
    pub(crate) right: Rect,
    pub(crate) front: Rect,
    pub(crate) back: Rect,
}
