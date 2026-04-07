struct ScreenUniform {
    screen_size_points: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> u_screen: ScreenUniform;

@group(1) @binding(0)
var u_atlas: texture_2d<f32>;

@group(1) @binding(1)
var u_sampler: sampler;

struct VertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @location(0) pos0: vec2<f32>,
    @location(1) pos1: vec2<f32>,
    @location(2) pos2: vec2<f32>,
    @location(3) pos3: vec2<f32>,
    @location(4) uv0: vec2<f32>,
    @location(5) uv1: vec2<f32>,
    @location(6) uv2: vec2<f32>,
    @location(7) uv3: vec2<f32>,
    @location(8) color: vec4<f32>,
    @location(9) decode_mode: f32,
    @location(10) field_range_px: f32,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) decode_mode: f32,
    @location(3) field_range_px: f32,
};

const QUAD_INDICES: array<u32, 6> = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let positions = array<vec2<f32>, 4>(input.pos0, input.pos1, input.pos2, input.pos3);
    let uvs = array<vec2<f32>, 4>(input.uv0, input.uv1, input.uv2, input.uv3);
    let idx = QUAD_INDICES[input.vertex_index];
    let point = positions[idx];
    let ndc = vec2<f32>(
        (point.x / u_screen.screen_size_points.x) * 2.0 - 1.0,
        1.0 - (point.y / u_screen.screen_size_points.y) * 2.0
    );

    var output: VertexOutput;
    output.position = vec4<f32>(ndc, 0.0, 1.0);
    output.uv = uvs[idx];
    output.color = input.color;
    output.decode_mode = input.decode_mode;
    output.field_range_px = input.field_range_px;
    return output;
}

fn median3(v: vec3<f32>) -> f32 {
    return max(min(v.x, v.y), min(max(v.x, v.y), v.z));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let sample = textureSample(u_atlas, u_sampler, input.uv);
    if (input.decode_mode < 0.5) {
        return sample * input.color;
    }

    let distance = select(sample.r, median3(sample.rgb), input.decode_mode > 1.5);
    let width = max(fwidth(distance), 1.0 / 255.0);
    let alpha = smoothstep(0.5 - width, 0.5 + width, distance);
    return vec4<f32>(input.color.rgb * alpha, input.color.a * alpha);
}
