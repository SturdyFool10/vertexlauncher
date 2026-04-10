// @vertex3d.target handle=post_process type=lighting lifecycle=transient
struct Scalar
{
    float4 value;
};

struct FullscreenOut
{
    float4 pos : SV_Position;
};

[[vk::binding(0, 0)]] Texture2D<float4> current_tex;
[[vk::binding(0, 1)]] Texture2D<float4> history_tex;
[[vk::binding(0, 2)]] ConstantBuffer<Scalar> scalar;

float4 load_current(int2 pixel)
{
    uint2 dims;
    current_tex.GetDimensions(dims.x, dims.y);
    int2 clamped = clamp(pixel, int2(0, 0), int2(dims) - 1);
    return current_tex.Load(int3(clamped, 0));
}

float4 load_history(int2 pixel)
{
    uint2 dims;
    history_tex.GetDimensions(dims.x, dims.y);
    int2 clamped = clamp(pixel, int2(0, 0), int2(dims) - 1);
    return history_tex.Load(int3(clamped, 0));
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
    float4 current = load_current(pixel);
    float4 lo = current;
    float4 hi = current;
    [unroll]
    for (int y = -1; y <= 1; ++y)
    {
        [unroll]
        for (int x = -1; x <= 1; ++x)
        {
            float4 sample_value = load_current(pixel + int2(x, y));
            lo = min(lo, sample_value);
            hi = max(hi, sample_value);
        }
    }
    float4 history = clamp(load_history(pixel), lo, hi);
    float current_weight = clamp(scalar.value.x, 0.05, 1.0);
    return lerp(history, current, current_weight);
}
