use super::*;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct TextGpuScenePageBatch {
    pub page_index: usize,
    pub quads: Arc<[TextGpuQuad]>,
}
