mod math;
mod scene;

pub use math::{Camera, Projection, Vec3};
pub use scene::{
    CuboidSpec, FaceUvs, RenderTriangle, SKIN_PREVIEW_NEAR, TriangleTexture, add_cuboid_triangles,
    add_cuboid_triangles_with_y, flip_uv_rect_x, hash_rgba_image, uv_rect_with_inset,
};
