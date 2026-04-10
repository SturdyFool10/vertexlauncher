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
    float4 nw = load_rgba(p + int2(-1, -1));
    float4 ne = load_rgba(p + int2(1, -1));
    float4 sw = load_rgba(p + int2(-1, 1));
    float4 se = load_rgba(p + int2(1, 1));
    float4 m = load_rgba(p);

    float luma_nw = rgb_luma(nw.rgb);
    float luma_ne = rgb_luma(ne.rgb);
    float luma_sw = rgb_luma(sw.rgb);
    float luma_se = rgb_luma(se.rgb);
    float luma_m = rgb_luma(m.rgb);

    float luma_min = min(luma_m, min(min(luma_nw, luma_ne), min(luma_sw, luma_se)));
    float luma_max = max(luma_m, max(max(luma_nw, luma_ne), max(luma_sw, luma_se)));
    float luma_range = luma_max - luma_min;
    float threshold = max(1.0 / 16.0, luma_max * (1.0 / 8.0));
    if (luma_range < threshold)
    {
        return m;
    }

    float2 dir = float2(
        -((luma_nw + luma_ne) - (luma_sw + luma_se)),
        (luma_nw + luma_sw) - (luma_ne + luma_se));
    float dir_reduce = max(
        ((luma_nw + luma_ne + luma_sw + luma_se) * 0.25) * (1.0 / 8.0),
        1.0 / 128.0);
    float rcp_dir_min = 1.0 / (min(abs(dir.x), abs(dir.y)) + dir_reduce);
    dir = clamp(dir * rcp_dir_min, -8.0.xx, 8.0.xx);

    float2 fp = pos.xy;
    float3 rgb_a = 0.5 * (
        sample_linear(fp + dir * (1.0 / 3.0 - 0.5)).rgb +
        sample_linear(fp + dir * (2.0 / 3.0 - 0.5)).rgb);
    float3 rgb_b = rgb_a * 0.5 + 0.25 * (
        sample_linear(fp + dir * -0.5).rgb +
        sample_linear(fp + dir * 0.5).rgb);

    float luma_b = rgb_luma(rgb_b);
    float3 final_rgb = rgb_b;
    if (luma_b < luma_min || luma_b > luma_max)
    {
        final_rgb = rgb_a;
    }
    return float4(final_rgb, m.a);
}
