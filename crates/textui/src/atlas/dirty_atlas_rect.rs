#[derive(Clone, Copy, Debug)]
pub(super) struct DirtyAtlasRect {
    pub(super) min: [usize; 2],
    pub(super) max: [usize; 2],
}

impl DirtyAtlasRect {
    pub(super) fn new(pos: [usize; 2], size: [usize; 2]) -> Self {
        Self {
            min: pos,
            max: [
                pos[0].saturating_add(size[0]),
                pos[1].saturating_add(size[1]),
            ],
        }
    }

    pub(super) fn union(self, other: Self) -> Self {
        Self {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }

    pub(super) fn size(self) -> [usize; 2] {
        [
            self.max[0].saturating_sub(self.min[0]),
            self.max[1].saturating_sub(self.min[1]),
        ]
    }
}
