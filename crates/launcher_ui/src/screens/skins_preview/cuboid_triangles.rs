use super::*;

pub(in super::super) fn add_cuboid_triangles(
    out: &mut Vec<RenderTriangle>,
    texture: TriangleTexture,
    spec: CuboidSpec,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    light_dir: Vec3,
) {
    add_cuboid_triangles_with_y(
        out,
        texture,
        spec,
        camera,
        projection,
        rect,
        light_dir,
        0.0,
        Vec3::new(0.0, 0.0, 0.0),
    );
}

/// Adds a lit cuboid to the projected triangle list, optionally applying an extra local
/// Y rotation and local-space offset before projection.
///
/// The function skips faces clipped behind the near plane and culls backfaces when
/// requested by `spec`.
///
/// This function does not panic.
pub(in super::super) fn add_cuboid_triangles_with_y(
    out: &mut Vec<RenderTriangle>,
    texture: TriangleTexture,
    spec: CuboidSpec,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    light_dir: Vec3,
    rotate_y_radians: f32,
    local_offset: Vec3,
) {
    let w = spec.size.x;
    let h = spec.size.y;
    let d = spec.size.z;
    let x0 = -w * 0.5;
    let x1 = w * 0.5;
    let y0 = 0.0;
    let y1 = -h;
    let z0 = -d * 0.5;
    let z1 = d * 0.5;

    let faces = [
        (
            [
                Vec3::new(x0, y0, z1),
                Vec3::new(x1, y0, z1),
                Vec3::new(x1, y1, z1),
                Vec3::new(x0, y1, z1),
            ],
            spec.uv.front,
            Vec3::new(0.0, 0.0, 1.0),
        ),
        (
            [
                Vec3::new(x1, y0, z0),
                Vec3::new(x0, y0, z0),
                Vec3::new(x0, y1, z0),
                Vec3::new(x1, y1, z0),
            ],
            spec.uv.back,
            Vec3::new(0.0, 0.0, -1.0),
        ),
        (
            [
                Vec3::new(x0, y0, z0),
                Vec3::new(x0, y0, z1),
                Vec3::new(x0, y1, z1),
                Vec3::new(x0, y1, z0),
            ],
            spec.uv.left,
            Vec3::new(-1.0, 0.0, 0.0),
        ),
        (
            [
                Vec3::new(x1, y0, z1),
                Vec3::new(x1, y0, z0),
                Vec3::new(x1, y1, z0),
                Vec3::new(x1, y1, z1),
            ],
            spec.uv.right,
            Vec3::new(1.0, 0.0, 0.0),
        ),
        (
            [
                Vec3::new(x0, y0, z0),
                Vec3::new(x1, y0, z0),
                Vec3::new(x1, y0, z1),
                Vec3::new(x0, y0, z1),
            ],
            spec.uv.top,
            Vec3::new(0.0, 1.0, 0.0),
        ),
        (
            [
                Vec3::new(x0, y1, z1),
                Vec3::new(x1, y1, z1),
                Vec3::new(x1, y1, z0),
                Vec3::new(x0, y1, z0),
            ],
            spec.uv.bottom,
            Vec3::new(0.0, -1.0, 0.0),
        ),
    ];

    for (quad, uv_rect, normal) in faces {
        let world_normal = rotate_z(
            rotate_y(rotate_x(normal, spec.rotate_x), rotate_y_radians),
            spec.rotate_z,
        )
        .normalized();
        let brightness = 0.58 + world_normal.dot(light_dir).max(0.0) * 0.42;
        let tint = color_with_brightness(Color32::WHITE, brightness);

        let transformed = quad.map(|vertex| {
            let vertex = vertex + local_offset;
            rotate_z(
                rotate_y(rotate_x(vertex, spec.rotate_x), rotate_y_radians),
                spec.rotate_z,
            ) + spec.pivot_top_center
        });
        let camera_vertices = transformed.map(|v| camera.world_to_camera(v));
        if camera_vertices.iter().any(|v| v.z <= projection.near) {
            continue;
        }
        if spec.cull_backfaces {
            let normal_camera = Vec3::new(
                world_normal.dot(camera.right),
                world_normal.dot(camera.up),
                world_normal.dot(camera.forward),
            );
            let center_camera =
                (camera_vertices[0] + camera_vertices[1] + camera_vertices[2] + camera_vertices[3])
                    * 0.25;
            let to_camera = (Vec3::new(0.0, 0.0, 0.0) - center_camera).normalized();
            if normal_camera.dot(to_camera) <= 0.0 {
                continue;
            }
        }
        let projected = camera_vertices.map(|v| project_point(v, projection, rect));
        if projected.iter().any(Option::is_none) {
            continue;
        }
        let projected = projected.map(Option::unwrap);

        let uv0 = uv_rect.left_top();
        let uv1 = uv_rect.right_top();
        let uv2 = uv_rect.right_bottom();
        let uv3 = uv_rect.left_bottom();
        out.push(RenderTriangle {
            texture,
            pos: [projected[0], projected[1], projected[2]],
            uv: [uv0, uv1, uv2],
            depth: [
                camera_vertices[0].z,
                camera_vertices[1].z,
                camera_vertices[2].z,
            ],
            color: tint,
        });
        out.push(RenderTriangle {
            texture,
            pos: [projected[0], projected[2], projected[3]],
            uv: [uv0, uv2, uv3],
            depth: [
                camera_vertices[0].z,
                camera_vertices[2].z,
                camera_vertices[3].z,
            ],
            color: tint,
        });
    }
}

fn rotate_x(point: Vec3, radians: f32) -> Vec3 {
    let (sin, cos) = radians.sin_cos();
    Vec3::new(
        point.x,
        point.y * cos - point.z * sin,
        point.y * sin + point.z * cos,
    )
}

fn rotate_y(point: Vec3, radians: f32) -> Vec3 {
    let (sin, cos) = radians.sin_cos();
    Vec3::new(
        point.x * cos + point.z * sin,
        point.y,
        -point.x * sin + point.z * cos,
    )
}

fn rotate_z(point: Vec3, radians: f32) -> Vec3 {
    let (sin, cos) = radians.sin_cos();
    Vec3::new(
        point.x * cos - point.y * sin,
        point.x * sin + point.y * cos,
        point.z,
    )
}

fn project_point(camera_space: Vec3, projection: Projection, rect: Rect) -> Option<Pos2> {
    if camera_space.z <= projection.near {
        return None;
    }

    let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
    let tan_half_fov = (projection.fov_y_radians * 0.5).tan().max(0.01);
    let x_ndc = camera_space.x / (camera_space.z * tan_half_fov * aspect);
    let y_ndc = camera_space.y / (camera_space.z * tan_half_fov);
    let x = rect.center().x + x_ndc * (rect.width() * 0.5);
    let y = rect.center().y - y_ndc * (rect.height() * 0.5);
    Some(Pos2::new(x, y))
}

fn color_with_brightness(base: Color32, brightness: f32) -> Color32 {
    let b = brightness.clamp(0.0, 1.0);
    Color32::from_rgba_premultiplied(
        ((base.r() as f32) * b).round() as u8,
        ((base.g() as f32) * b).round() as u8,
        ((base.b() as f32) * b).round() as u8,
        base.a(),
    )
}
