@binding(0) @group(0) var source_tex_0 : texture_2d<f32>;
@binding(0) @group(1) var depth_tex_0 : texture_depth_2d;

struct FullscreenOut_0 {
    @builtin(position) pos_0 : vec4<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index_0 : u32) -> FullscreenOut_0 {
    var output_0 : FullscreenOut_0;
    output_0.pos_0 = vec4<f32>(
        f32(((vertex_index_0 << 1u) & 2u)) * 2.0 - 1.0,
        1.0 - f32((vertex_index_0 & 2u)) * 2.0,
        0.0,
        1.0
    );
    return output_0;
}

fn load_color_0(pixel_0 : vec2<i32>) -> vec4<f32> {
    let dims = textureDimensions(source_tex_0);
    let clamped = clamp(pixel_0, vec2<i32>(0, 0), vec2<i32>(dims) - vec2<i32>(1, 1));
    return textureLoad(source_tex_0, clamped, 0);
}

fn load_depth_0(pixel_0 : vec2<i32>) -> f32 {
    let dims = textureDimensions(depth_tex_0);
    let clamped = clamp(pixel_0, vec2<i32>(0, 0), vec2<i32>(dims) - vec2<i32>(1, 1));
    return textureLoad(depth_tex_0, clamped, 0);
}

struct pixelOutput_0 {
    @location(0) output_1 : vec4<f32>,
};

@fragment
fn fs_main(@builtin(position) pos_1 : vec4<f32>) -> pixelOutput_0 {
    let pixel = vec2<i32>(pos_1.xy);
    let base = load_color_0(pixel);
    let center_depth = load_depth_0(pixel);

    var occlusion = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            if (x == 0 && y == 0) {
                continue;
            }
            let sample_depth = load_depth_0(pixel + vec2<i32>(x, y));
            let contribution = smoothstep(0.0005, 0.025, max(0.0, center_depth - sample_depth));
            occlusion = occlusion + contribution;
        }
    }

    let ao = clamp(1.0 - (occlusion / 8.0) * 0.55, 0.35, 1.0);
    return pixelOutput_0(vec4<f32>(base.rgb * ao, base.a));
}
