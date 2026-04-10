// @vertex3d.target handle=present type=lighting lifecycle=transient
struct FullscreenOut
{
    float4 pos : SV_Position;
};

[[vk::binding(0, 0)]] Texture2D<float4> source_tex;

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
    uint2 dims;
    source_tex.GetDimensions(dims.x, dims.y);
    int2 pixel = clamp(int2(pos.xy), int2(0, 0), int2(dims) - 1);
    return source_tex.Load(int3(pixel, 0));
}
