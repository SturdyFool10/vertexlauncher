// ── Text instanced renderer shader ────────────────────────────────────────────
//
// Always uses FP16 atlas textures for HDR capability (colors > 1.0 supported).
// Conditionally applies tone mapping based on output surface:
//   - SDR surface: Hermite-spline tonemap + sRGB encode
//   - HDR surface: Linear passthrough (scene-referred, no tone mapping)

struct ScreenUniform {
    screen_size_points: vec2<f32>,
    output_is_hdr: f32,  // 1.0 = HDR surface (linear passthrough), 0.0 = SDR surface (tonemap + sRGB)
    _pad: vec2<f32>,
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

// ── Hermite spline helpers for tone mapping ───────────────────────────────────

// Evaluates a cubic Hermite segment over t ∈ [0, 1].
fn hermite_seg(t: f32, p0: f32, m0: f32, p1: f32, m1: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    return p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
         + m0 * (t3 - 2.0 * t2 + t)
         + p1 * (-2.0 * t3 + 3.0 * t2)
         + m1 * (t3 - t2);
}

// ── Tonemapper curve parameters ───────────────────────────────────────────────

const HDR_WHITE: f32 = 2.0;
const TOE_X: f32 = 0.04;
const TOE_Y: f32 = 0.036;
const SHL_X: f32 = 1.05;
const SHL_Y: f32 = 0.95;

fn lin_slope() -> f32 {
    return (SHL_Y - TOE_Y) / (SHL_X - TOE_X);
}

// Per-channel Hermite-spline tonemap for HDR → SDR conversion.
fn tonemap_channel(x: f32) -> f32 {
    let v = max(x, 0.0);
    let m = lin_slope();

    if v < TOE_X {
        let t = v / TOE_X;
        let m1 = m * TOE_X;
        return hermite_seg(t, 0.0, 0.0, TOE_Y, m1);
    } else if v < SHL_X {
        return TOE_Y + (v - TOE_X) * m;
    } else {
        let seg = HDR_WHITE - SHL_X;
        let t = min((v - SHL_X) / seg, 1.0);
        let m0 = m * seg;
        return clamp(hermite_seg(t, SHL_Y, m0, 1.0, 0.0), 0.0, 1.0);
    }
}

fn tonemap(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(tonemap_channel(rgb.r), tonemap_channel(rgb.g), tonemap_channel(rgb.b));
}

// ── sRGB gamma encode (display-linear → perceptual) ───────────────────────────

fn srgb_encode_channel(c: f32) -> f32 {
    if c <= 0.0031308 {
        return c * 12.92;
    }
    return 1.055 * pow(max(c, 0.0), 1.0 / 2.4) - 0.055;
}

fn srgb_encode(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(srgb_encode_channel(rgb.r), srgb_encode_channel(rgb.g), srgb_encode_channel(rgb.b));
}

// ── Final colorspace conversion based on output surface type ───────────────────

fn apply_output_transform(rgb: vec3<f32>) -> vec3<f32> {
    // If outputting to HDR surface, pass through in scene-linear space (no tone mapping)
    if u_screen.output_is_hdr > 0.5 {
        return rgb;
    }

    // SDR surface: apply Hermite-spline tonemap then sRGB encode
    let toned = tonemap(rgb);
    return srgb_encode(toned);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let sample = textureSample(u_atlas, u_sampler, input.uv);
    var out_color: vec4<f32>;

    if input.decode_mode < 0.5 {
        // Alpha mask mode (standard rasterized glyphs)
        out_color = sample * input.color;
    } else {
        // SDF/MSDF mode with field-based alpha reconstruction
        let distance = select(sample.r, median3(sample.rgb), input.decode_mode > 1.5);
        let width = max(fwidth(distance), 1.0 / 255.0);
        let alpha = smoothstep(0.5 - width, 0.5 + width, distance);
        out_color = vec4<f32>(input.color.rgb * alpha, input.color.a * alpha);
    }

    // Apply output transform (HDR passthrough or SDR tonemap + sRGB)
    let final_rgb = apply_output_transform(out_color.rgb);
    return vec4<f32>(final_rgb, out_color.a);
}
