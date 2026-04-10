//! Lightweight Mat4 wrapper.

use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub glam::Mat4);

impl Mat4 {
    #[inline]
    pub fn identity() -> Self {
        Self(glam::Mat4::IDENTITY)
    }

    #[inline]
    pub fn from_glam(value: glam::Mat4) -> Self {
        Self(value)
    }

    #[inline]
    pub fn to_glam(self) -> glam::Mat4 {
        self.0
    }

    #[inline]
    pub fn translation(offset: Vec3) -> Self {
        Self(glam::Mat4::from_translation(offset))
    }

    #[inline]
    pub fn scaling(scale: Vec3) -> Self {
        Self(glam::Mat4::from_scale(scale))
    }

    #[inline]
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        Self(glam::Mat4::perspective_rh(fov_y, aspect, near, far))
    }

    #[inline]
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        Self(glam::Mat4::look_at_rh(eye, target, up))
    }
}

impl Default for Mat4 {
    fn default() -> Self {
        Self::identity()
    }
}
