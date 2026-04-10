//! Camera module providing view transformation utilities
//!
//! This module contains:
//! - `Camera` - View orientation and position in 3D space
//! - `Projection` - Projection parameters (FOV, near/far planes)

use crate::{Mat4, Vec3};

/// A camera representing the viewer's position and orientation in 3D space.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Right direction vector (X axis of camera space)
    pub right: Vec3,
    /// Up direction vector (Y axis of camera space)
    pub up: Vec3,
    /// Forward direction vector (Z axis of camera space)
    pub forward: Vec3,
    /// Camera position in world space
    position: Vec3,
}

impl Camera {
    /// Create a camera looking at a target point.
    ///
    /// # Arguments
    /// * `position` - Camera's position in world space
    /// * `target` - Point the camera is looking at
    /// * `world_up` - Up direction in world space (typically `(0, 1, 0)`)
    pub fn look_at(position: Vec3, target: Vec3, world_up: Vec3) -> Self {
        let forward = (target - position).normalize();
        let right = forward.cross(world_up).normalize();
        let up = right.cross(forward).normalize();
        Self {
            position,
            right,
            up,
            forward,
        }
    }

    /// Create a camera with explicit orientation vectors.
    pub fn new(position: Vec3, right: Vec3, up: Vec3, forward: Vec3) -> Self {
        Self {
            position,
            right,
            up,
            forward,
        }
    }

    /// Get the camera's position in world space.
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// Set the camera's position in world space.
    pub fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }

    /// Transform a point from world space to camera/view space.
    pub fn world_to_view(self, world: Vec3) -> Vec3 {
        let rel = world - self.position;
        Vec3::new(rel.dot(self.right), rel.dot(self.up), -rel.dot(self.forward))
    }

    /// Transform a point from camera/view space to world space.
    pub fn view_to_world(self, view: Vec3) -> Vec3 {
        self.position + self.right * view.x + self.up * view.y - self.forward * view.z
    }

    /// Get the view matrix that transforms world coordinates to view coordinates.
    pub fn view_matrix(&self) -> Mat4 {
        let pos = self.position;
        let right = self.right;
        let up = self.up;
        let forward = self.forward;

        // Build the 3x3 rotation part + translation
        Mat4::from_cols_array(&[
            right.x,
            right.y,
            right.z,
            0.0,
            up.x,
            up.y,
            up.z,
            0.0,
            -forward.x,
            -forward.y,
            -forward.z,
            0.0,
            -pos.dot(right),
            -pos.dot(up),
            pos.dot(forward),
            1.0,
        ])
    }

    /// Rotate the camera around its local right axis (pitch).
    pub fn pitch(&mut self, radians: f32) {
        let (sin, cos) = radians.sin_cos();
        let old_forward = self.forward;
        let old_up = self.up;

        self.forward = old_forward * cos + old_up * sin;
        self.up = old_up * cos - old_forward * sin;

        // Recalculate right to maintain orthogonality
        self.right = self.forward.cross(self.up).normalize();
    }

    /// Rotate the camera around world up axis (yaw).
    pub fn yaw(&mut self, radians: f32) {
        let (sin, cos) = radians.sin_cos();
        let old_right = self.right;
        let old_forward = self.forward;

        self.right = old_right * cos + old_forward * sin;
        self.forward = old_forward * cos - old_right * sin;

        // Recalculate up to maintain orthogonality
        self.up = self.right.cross(self.forward).normalize();
    }

    /// Rotate the camera around its forward axis (roll).
    pub fn roll(&mut self, radians: f32) {
        let (sin, cos) = radians.sin_cos();
        let old_right = self.right;
        let old_up = self.up;

        self.right = old_right * cos + old_up * sin;
        self.up = old_up * cos - old_right * sin;
    }

    /// Move the camera forward/backward along its view direction.
    pub fn move_forward(&mut self, distance: f32) {
        self.position += self.forward * distance;
    }

    /// Move the camera left/right along its right direction.
    pub fn move_right(&mut self, distance: f32) {
        self.position += self.right * distance;
    }

    /// Move the camera up/down along its up direction.
    pub fn move_up(&mut self, distance: f32) {
        self.position += self.up * distance;
    }
}

impl Default for Camera {
    fn default() -> Self {
        // Default: positioned at origin, looking down -Z axis, Y is up
        Self::look_at(
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
    }
}

/// Projection parameters for transforming view space to clip space.
#[derive(Clone, Copy, Debug)]
pub struct Projection {
    /// Field of view in Y direction (radians)
    pub fov_y_radians: f32,
    /// Near clipping plane distance
    pub near: f32,
    /// Far clipping plane distance
    pub far: Option<f32>,
}

impl Projection {
    /// Create a new projection with perspective.
    ///
    /// # Arguments
    /// * `fov_y_radians` - Field of view in Y direction (radians)
    /// * `near` - Near clipping plane distance
    pub fn perspective(fov_y_radians: f32, near: f32) -> Self {
        Self {
            fov_y_radians,
            near,
            far: None,
        }
    }

    /// Create a new projection with perspective and far plane.
    pub fn perspective_with_far(fov_y_radians: f32, near: f32, far: f32) -> Self {
        Self {
            fov_y_radians,
            near,
            far: Some(far),
        }
    }

    /// Create an orthographic projection.
    pub fn orthographic(_left: f32, _right: f32, _top: f32, _bottom: f32, near: f32, _far: f32) -> Self {
        // For now, we'll use perspective - orthographic would need a different struct or enum
        Self::perspective(std::f32::consts::FRAC_PI_4, near)
    }

    /// Get the aspect ratio from width and height.
    pub fn aspect_from_size(width: f32, height: f32) -> f32 {
        (width / height.max(1.0)).max(0.01)
    }

    /// Get the projection matrix for a given aspect ratio.
    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        let tan_half_fov = (self.fov_y_radians * 0.5).tan().max(0.01);

        if let Some(far_plane) = self.far {
            // Perspective with far plane
            let n = self.near;
            let f = far_plane;

            Mat4::from_cols_array(&[
                1.0 / (tan_half_fov * aspect),
                0.0,
                0.0,
                0.0,
                0.0,
                1.0 / tan_half_fov,
                0.0,
                0.0,
                0.0,
                0.0,
                -(f + n) / (f - n),
                -2.0 * f * n / (f - n),
                0.0,
                0.0,
                -1.0,
                0.0,
            ])
        } else {
            // Perspective without far plane (infinite far)
            let n = self.near;

            Mat4::from_cols_array(&[
                1.0 / (tan_half_fov * aspect),
                0.0,
                0.0,
                0.0,
                0.0,
                1.0 / tan_half_fov,
                0.0,
                0.0,
                0.0,
                0.0,
                -1.0,
                -2.0 * n,
                0.0,
                0.0,
                -1.0,
                0.0,
            ])
        }
    }

    /// Transform a point from view space to clip space (NDC).
    pub fn project_point(&self, view_space: Vec3, aspect: f32) -> Option<Vec3> {
        let depth = -view_space.z;
        if depth <= self.near {
            return None;
        }

        let tan_half_fov = (self.fov_y_radians * 0.5).tan().max(0.01);

        // Project to NDC (-1 to +1 range)
        let x_ndc = view_space.x / (depth * tan_half_fov * aspect);
        let y_ndc = view_space.y / (depth * tan_half_fov);

        // Depth in NDC space (0 to 1 for OpenGL, -1 to +1 for DirectX)
        // Using OpenGL convention: near=0, far=1
        let z_ndc = if let Some(far_plane) = self.far {
            ((far_plane + self.near) / (far_plane - self.near))
                - (2.0 * far_plane * self.near) / ((far_plane - self.near) * depth)
        } else {
            1.0 - self.near / depth
        };

        Some(Vec3::new(x_ndc, y_ndc, z_ndc))
    }
}

impl Default for Projection {
    fn default() -> Self {
        // Default: 60 degree FOV, near plane at 0.1 units
        Self::perspective(std::f32::consts::FRAC_PI_3, 0.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_look_at() {
        let camera = Camera::look_at(
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        );

        // Forward should point toward target (negative Z)
        assert!((camera.forward.z + 1.0).abs() < 0.000_1);
    }

    #[test]
    fn test_world_to_view() {
        let camera = Camera::look_at(
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        );

        // A point at the origin should be in front of camera (negative Z in view space)
        let view_pos = camera.world_to_view(Vec3::new(0.0, 0.0, 0.0));
        assert!(view_pos.z < 0.0);
    }

    #[test]
    fn test_projection_matrix() {
        let projection = Projection::perspective(std::f32::consts::FRAC_PI_4, 1.0);
        let matrix = projection.projection_matrix(1.0);

        // Just verify it creates a valid matrix (non-zero determinant)
        assert!(matrix.determinant().abs() > 0.000_1);
    }

    #[test]
    fn test_project_point() {
        let projection = Projection::perspective(std::f32::consts::FRAC_PI_4, 1.0);

        // Point in front of camera should project successfully
        let result = projection.project_point(Vec3::new(0.0, 0.0, -5.0), 1.0);
        assert!(result.is_some());

        // Point behind near plane should fail
        let result = projection.project_point(Vec3::new(0.0, 0.0, 0.5), 1.0);
        assert!(result.is_none());
    }
}
