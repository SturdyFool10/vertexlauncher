use super::*;

#[derive(Clone, Copy)]
pub(crate) struct Camera {
    pub(crate) position: Vec3,
    pub(crate) right: Vec3,
    pub(crate) up: Vec3,
    pub(crate) forward: Vec3,
}

impl Camera {
    pub(crate) fn look_at(position: Vec3, target: Vec3, world_up: Vec3) -> Self {
        let forward = (target - position).normalized();
        let right = forward.cross(world_up).normalized();
        let up = right.cross(forward).normalized();
        Self {
            position,
            right,
            up,
            forward,
        }
    }

    pub(crate) fn world_to_camera(self, world: Vec3) -> Vec3 {
        let rel = world - self.position;
        Vec3::new(rel.dot(self.right), rel.dot(self.up), rel.dot(self.forward))
    }
}
