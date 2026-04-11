// @vertex3d.target handle=present type=lighting lifecycle=transient
//
// Reads from a linear Rgba16Float intermediate buffer (scene-referred, HDR
// capable) and outputs display-ready sRGB values to the final surface.
//
// Two-stage transform applied per pixel:
//   1. Hermite-spline tonemapper  — maps HDR scene luminance to display [0, 1]
//   2. sRGB gamma encode          — display-linear → perceptual sRGB
//
// The spline is C¹ everywhere and is defined by three segments:
//   Toe      : cubic Hermite, zero slope at black (gentle shadow lift)
//   Linear   : straight ramp through midtones (slope ≈ 1 for SDR content)
//   Shoulder : cubic Hermite, zero slope at white (soft roll-off / no hard clip)

[[vk::binding(0, 0)]] Texture2D<float4> source_tex;

// ── Hermite spline helpers ────────────────────────────────────────────────────

// Evaluates a cubic Hermite segment over t ∈ [0, 1].
// p0, p1 : endpoint values.
// m0, m1 : endpoint tangents SCALED to the segment length.
float hermite_seg(float t, float p0, float m0, float p1, float m1)
{
    float t2 = t * t;
    float t3 = t2 * t;
    return p0 * ( 2.0*t3 - 3.0*t2 + 1.0)
         + m0 * (     t3 - 2.0*t2 + t  )
         + p1 * (-2.0*t3 + 3.0*t2      )
         + m1 * (     t3 -     t2      );
}

// ── Tonemapper curve parameters ───────────────────────────────────────────────
static const float HDR_WHITE = 2.0;
static const float TOE_X     = 0.04;
static const float TOE_Y     = 0.036;
static const float SHL_X     = 1.05;
static const float SHL_Y     = 0.95;

float lin_slope()
{
    return (SHL_Y - TOE_Y) / (SHL_X - TOE_X);
}

float tonemap_channel(float x)
{
    float v = max(x, 0.0);
    float m = lin_slope();

    if (v < TOE_X)
    {
        float t  = v / TOE_X;
        float m1 = m * TOE_X;
        return hermite_seg(t, 0.0, 0.0, TOE_Y, m1);
    }
    else if (v < SHL_X)
    {
        return TOE_Y + (v - TOE_X) * m;
    }
    else
    {
        float seg = HDR_WHITE - SHL_X;
        float t   = min((v - SHL_X) / seg, 1.0);
        float m0  = m * seg;
        return clamp(hermite_seg(t, SHL_Y, m0, 1.0, 0.0), 0.0, 1.0);
    }
}

float3 tonemap(float3 rgb)
{
    return float3(
        tonemap_channel(rgb.x),
        tonemap_channel(rgb.y),
        tonemap_channel(rgb.z));
}

// ── sRGB gamma encode ─────────────────────────────────────────────────────────

float linear_to_srgb_channel(float c)
{
    if (c <= 0.0031308)
        return c * 12.92;
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

float3 linear_to_srgb(float3 rgb)
{
    return float3(
        linear_to_srgb_channel(rgb.x),
        linear_to_srgb_channel(rgb.y),
        linear_to_srgb_channel(rgb.z));
}

// ── Vertex stage ──────────────────────────────────────────────────────────────

struct FullscreenOut
{
    float4 pos : SV_Position;
};

[shader("vertex")]
FullscreenOut vs_fullscreen(uint vertex_index : SV_VertexID)
{
    FullscreenOut output;
    float x = float((vertex_index << 1) & 2);
    float y = float(vertex_index & 2);
    output.pos = float4(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return output;
}

// ── Fragment stage ────────────────────────────────────────────────────────────

[shader("fragment")]
float4 fs_main(float4 pos : SV_Position) : SV_Target
{
    uint2 dims;
    source_tex.GetDimensions(dims.x, dims.y);
    int2 pixel = clamp(int2(pos.xy), int2(0, 0), int2(dims) - 1);
    float4 src = source_tex.Load(int3(pixel, 0));

    // 1. Hermite-spline tonemap: HDR scene-linear → display-linear [0, 1]
    float3 display_linear = tonemap(src.xyz);

    // 2. sRGB encode: display-linear → perceptual sRGB for SDR surface output.
    return float4(linear_to_srgb(display_linear), src.w);
}
