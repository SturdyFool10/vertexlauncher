use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct AppliedFontSignature {
    pub(super) family: UiFontFamily,
    pub(super) size: f32,
    pub(super) weight: i32,
}
