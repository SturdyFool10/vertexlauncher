use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct AppliedTextSignature {
    pub(super) family: UiFontFamily,
    pub(super) size: f32,
    pub(super) weight: i32,
    pub(super) open_type_features_enabled: bool,
    pub(super) open_type_features_to_enable: String,
}
