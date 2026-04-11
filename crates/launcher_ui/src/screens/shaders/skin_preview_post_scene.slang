// @vertex3d.target handle=scene_color type=lighting lifecycle=transient
// @vertex3d.target handle=scene_depth_linear type=lighting lifecycle=transient
struct VertexIn
{
    float2 pos_points : LOC0;
    float camera_z : LOC1;
    float2 uv : LOC2;
    float4 color : LOC3;
};

struct Globals
{
    float2 screen_size_points;
    float2 _pad;
};

struct Scalars
{
    float4 value;
};

struct VertexOut
{
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
};

[[vk::binding(0, 0)]] Texture2D<float4> preview_tex;
[[vk::binding(1, 0)]] SamplerState preview_sampler;
[[vk::binding(0, 1)]] ConstantBuffer<Globals> globals;
[[vk::binding(0, 2)]] ConstantBuffer<Scalars> scalars;

float4 sample_preview_pixel_art(float2 uv)
{
    uint2 dims_i;
    preview_tex.GetDimensions(dims_i.x, dims_i.y);
    float2 dims = float2(dims_i);
    float2 texel = 0.5 / dims;
    float2 clamped_uv = clamp(uv, texel, 1.0.xx - texel);
    float2 uv_grad_x = ddx(clamped_uv);
    float2 uv_grad_y = ddy(clamped_uv);
    float2 texel_grad_x = uv_grad_x * dims;
    float2 texel_grad_y = uv_grad_y * dims;
    float footprint = max(
        max(abs(texel_grad_x.x), abs(texel_grad_x.y)),
        max(abs(texel_grad_y.x), abs(texel_grad_y.y)));
    int2 pixel = clamp(int2(clamped_uv * dims), int2(0, 0), int2(dims_i) - 1);
    float4 nearest = preview_tex.Load(int3(pixel, 0));
    float4 filtered = preview_tex.SampleGrad(
        preview_sampler,
        clamped_uv,
        uv_grad_x,
        uv_grad_y);
    float filtered_mix = smoothstep(0.85, 1.35, footprint);
    return lerp(nearest, filtered, filtered_mix);
}

float4 load_preview_texel(int2 texel, uint2 dims_i)
{
    int2 max_texel = int2(dims_i) - 1;
    return preview_tex.Load(int3(clamp(texel, int2(0, 0), max_texel), 0));
}

float4 sample_preview_texel_border_aa(float2 uv)
{
    uint2 dims_i;
    preview_tex.GetDimensions(dims_i.x, dims_i.y);
    float2 dims = float2(dims_i);
    float2 texel = 0.5 / dims;
    float2 clamped_uv = clamp(uv, texel, 1.0.xx - texel);
    float2 p = clamped_uv * dims - 0.5.xx;
    int2 base = int2(floor(p));
    float2 f = frac(p);
    float2 edge_width = clamp(max(fwidth(p), 0.0001.xx) * 0.75, 0.0.xx, 0.5.xx);
    float2 t = smoothstep(0.5.xx - edge_width, 0.5.xx + edge_width, f);
    float4 c00 = load_preview_texel(base, dims_i);
    float4 c10 = load_preview_texel(base + int2(1, 0), dims_i);
    float4 c01 = load_preview_texel(base + int2(0, 1), dims_i);
    float4 c11 = load_preview_texel(base + int2(1, 1), dims_i);
    float4 cx0 = lerp(c00, c10, t.x);
    float4 cx1 = lerp(c01, c11, t.x);
    return lerp(cx0, cx1, t.y);
}

[shader("vertex")]
VertexOut vs_main(VertexIn input)
{
    VertexOut output;
    float x_ndc = (input.pos_points.x / globals.screen_size_points.x) * 2.0 - 1.0;
    float y_ndc = 1.0 - (input.pos_points.y / globals.screen_size_points.y) * 2.0;
    float z_cam = max(input.camera_z, 1.5 + 0.0001);
    float clip_w = z_cam;
    float clip_z = z_cam - 1.5;
    output.pos = float4(x_ndc * clip_w, y_ndc * clip_w, clip_z, clip_w);
    output.uv = input.uv;
    output.color = input.color;
    return output;
}

struct FragOut
{
    float4 color        : SV_Target0;
    float4 depth_linear : SV_Target1;
};

[shader("fragment")]
FragOut fs_main(VertexOut input)
{
    float4 sampled = (scalars.value.x > 0.5
            ? sample_preview_texel_border_aa(input.uv)
            : sample_preview_pixel_art(input.uv))
        * input.color;
    if (sampled.a <= 0.001)
    {
        discard;
    }
    FragOut output;
    output.color        = sampled;
    output.depth_linear = float4(input.pos.z, 0.0, 0.0, 1.0);
    return output;
}
