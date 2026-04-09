use super::*;

#[derive(Clone, Debug)]
pub struct TextRenderScene {
    pub quads: Vec<TextAtlasQuad>,
    pub bounds: TextRect,
    pub size_points: TextVector,
}

impl TextRenderScene {
    pub fn atlas_page_indices(&self) -> Vec<usize> {
        let mut page_indices = self
            .quads
            .iter()
            .map(|quad| quad.atlas_page_index)
            .collect::<Vec<_>>();
        page_indices.sort_unstable();
        page_indices.dedup();
        page_indices
    }

    pub fn to_gpu_scene(&self, atlas_pages: Vec<TextAtlasPageData>) -> TextGpuScene {
        TextGpuScene {
            atlas_pages,
            quads: self
                .quads
                .iter()
                .map(|quad| TextGpuQuad {
                    atlas_page_index: quad.atlas_page_index,
                    positions: quad.positions.map(|point| [point.x, point.y]),
                    uvs: quad.uvs.map(|point| [point.x, point.y]),
                    tint_rgba: quad.tint.to_array(),
                })
                .collect(),
            bounds_min: [self.bounds.min.x, self.bounds.min.y],
            bounds_max: [self.bounds.max.x, self.bounds.max.y],
            size_points: [self.size_points.x, self.size_points.y],
            fingerprint: 0,
        }
    }
}
