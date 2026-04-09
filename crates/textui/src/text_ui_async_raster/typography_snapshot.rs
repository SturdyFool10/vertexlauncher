#[derive(Clone, Debug)]
pub(crate) struct TypographySnapshot {
    pub(crate) ui_font_family: Option<String>,
    pub(crate) ui_font_size_scale: f32,
    pub(crate) ui_font_weight: i32,
    pub(crate) open_type_feature_tags: Vec<[u8; 4]>,
}
