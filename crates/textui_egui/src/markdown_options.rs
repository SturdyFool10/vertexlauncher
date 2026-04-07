use crate::{CodeBlockOptions, LabelOptions};

#[derive(Clone, Debug)]
pub struct MarkdownOptions {
    pub body: LabelOptions,
    pub heading_scale: f32,
    pub paragraph_spacing: f32,
    pub code: CodeBlockOptions,
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        Self {
            body: LabelOptions::default(),
            heading_scale: 1.28,
            paragraph_spacing: 8.0,
            code: CodeBlockOptions::default(),
        }
    }
}
