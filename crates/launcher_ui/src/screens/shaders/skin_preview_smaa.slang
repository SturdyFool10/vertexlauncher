// @vertex3d.target handle=post_process type=lighting lifecycle=transient
struct FullscreenOut
{
    float4 pos : SV_Position;
};

[[vk::binding(0, 0)]] Texture2D<float4> source_tex;

float rgb_luma(float3 rgb)
{
    return dot(rgb, float3(0.299, 0.587, 0.114));
}

float4 load_rgba(int2 pixel)
{
    uint2 dims;
    source_tex.GetDimensions(dims.x, dims.y);
    int2 clamped = clamp(pixel, int2(0, 0), int2(dims) - 1);
    return source_tex.Load(int3(clamped, 0));
}

float4 sample_linear(float2 pixel)
{
    uint2 dims;
    source_tex.GetDimensions(dims.x, dims.y);
    float2 max_pixel = float2(int2(dims) - 1);
    float2 p = clamp(pixel, 0.0.xx, max_pixel);
    int2 p0 = int2(floor(p));
    int2 p1 = min(p0 + int2(1, 1), int2(dims) - 1);
    float2 f = frac(p);
    float4 c00 = load_rgba(p0);
    float4 c10 = load_rgba(int2(p1.x, p0.y));
    float4 c01 = load_rgba(int2(p0.x, p1.y));
    float4 c11 = load_rgba(p1);
    float4 top = lerp(c00, c10, f.x);
    float4 bottom = lerp(c01, c11, f.x);
    return lerp(top, bottom, f.y);
}

[shader("vertex")]
FullscreenOut vs_fullscreen(uint vertex_index : SV_VertexID)
{
    FullscreenOut output;
    float x = float((vertex_index << 1) & 2);
    float y = float(vertex_index & 2);
    output.pos = float4(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return output;
}

[shader("fragment")]
float4 fs_main(float4 pos : SV_Position) : SV_Target
{
    int2 p = int2(pos.xy);
    float4 c = load_rgba(p);
    float4 l = load_rgba(p + int2(-1, 0));
    float4 r = load_rgba(p + int2(1, 0));
    float4 t = load_rgba(p + int2(0, -1));
    float4 b = load_rgba(p + int2(0, 1));

    float luma_c = rgb_luma(c.rgb);
    float luma_l = rgb_luma(l.rgb);
    float luma_r = rgb_luma(r.rgb);
    float luma_t = rgb_luma(t.rgb);
    float luma_b = rgb_luma(b.rgb);

    float edge_h = max(abs(luma_l - luma_c), abs(luma_r - luma_c));
    float edge_v = max(abs(luma_t - luma_c), abs(luma_b - luma_c));
    float threshold = max(0.04, luma_c * 0.12);

    if (max(edge_h, edge_v) < threshold)
    {
        return c;
    }

    float2 center = pos.xy;
    if (edge_h >= edge_v)
    {
        float4 a = sample_linear(center + float2(-0.75, 0.0));
        float4 b_sample = sample_linear(center + float2(0.75, 0.0));
        float4 long_a = sample_linear(center + float2(-1.5, 0.0));
        float4 long_b = sample_linear(center + float2(1.5, 0.0));
        float blend = clamp((edge_h - threshold) * 6.0, 0.0, 1.0);
        float4 neighbor = lerp(0.5 * (a + b_sample), 0.5 * (long_a + long_b), 0.35);
        return lerp(c, float4(neighbor.rgb, c.a), blend * 0.75);
    }

    float4 sample_a = sample_linear(center + float2(0.0, -0.75));
    float4 sample_b = sample_linear(center + float2(0.0, 0.75));
    float4 long_sample_a = sample_linear(center + float2(0.0, -1.5));
    float4 long_sample_b = sample_linear(center + float2(0.0, 1.5));
    float blend = clamp((edge_v - threshold) * 6.0, 0.0, 1.0);
    float4 neighbor = lerp(
        0.5 * (sample_a + sample_b),
        0.5 * (long_sample_a + long_sample_b),
        0.35);
    return lerp(c, float4(neighbor.rgb, c.a), blend * 0.75);
}
