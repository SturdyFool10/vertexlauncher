// @vertex3d.target handle=post_process type=lighting lifecycle=transient
//
// Screen-space ambient occlusion pass.
// Reads scene color from group 0 and linearised depth (R channel of an
// Rgba16Float color attachment) from group 1.  Compares depth values in a
// 3×3 neighbourhood and darkens pixels that are occluded by nearby geometry.

[[vk::binding(0, 0)]] Texture2D<float4> source_tex;
[[vk::binding(0, 1)]] Texture2D<float4> depth_tex;

struct FullscreenOut
{
    float4 pos : SV_Position;
};

float4 load_color(int2 pixel)
{
    uint2 dims;
    source_tex.GetDimensions(dims.x, dims.y);
    int2 clamped = clamp(pixel, int2(0, 0), int2(dims) - 1);
    return source_tex.Load(int3(clamped, 0));
}

float load_depth(int2 pixel)
{
    uint2 dims;
    depth_tex.GetDimensions(dims.x, dims.y);
    int2 clamped = clamp(pixel, int2(0, 0), int2(dims) - 1);
    return depth_tex.Load(int3(clamped, 0)).r;
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
    int2 pixel = int2(pos.xy);
    float4 base = load_color(pixel);
    float center_depth = load_depth(pixel);

    float occlusion = 0.0;
    for (int y = -1; y <= 1; ++y)
    {
        for (int x = -1; x <= 1; ++x)
        {
            if (x == 0 && y == 0) continue;
            float sample_depth = load_depth(pixel + int2(x, y));
            float contribution = smoothstep(0.0005, 0.025, max(0.0, center_depth - sample_depth));
            occlusion += contribution;
        }
    }

    float ao = clamp(1.0 - (occlusion / 8.0) * 0.55, 0.35, 1.0);
    return float4(base.rgb * ao, base.a);
}
