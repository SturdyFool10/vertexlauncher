use super::*;

#[derive(Default)]
pub(super) struct FlattenedOutline {
    pub(super) contours: Vec<Vec<[f32; 2]>>,
    pub(super) segments: Vec<FieldLineSegment>,
    pub(super) min: [f32; 2],
    pub(super) max: [f32; 2],
}

impl FlattenedOutline {
    pub(super) fn new() -> Self {
        Self {
            contours: Vec::new(),
            segments: Vec::new(),
            min: [f32::INFINITY, f32::INFINITY],
            max: [f32::NEG_INFINITY, f32::NEG_INFINITY],
        }
    }

    pub(super) fn include_point(&mut self, point: [f32; 2]) {
        self.min[0] = self.min[0].min(point[0]);
        self.min[1] = self.min[1].min(point[1]);
        self.max[0] = self.max[0].max(point[0]);
        self.max[1] = self.max[1].max(point[1]);
    }
}
