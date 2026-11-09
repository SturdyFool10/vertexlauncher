// The skin preview's material: a textured, nearest-sampled surface lit the way the launcher has always lit its
// preview, with no engine lighting at all.
//
//     colour = texel * (0.58 + 0.42 * max(dot(worldNormal, light), 0))
//
// The multiplication is done on the texture's own (display-encoded) bytes, like the CPU preview, and the result is
// linearised for the engine, which re-encodes it for display: so the picture comes out exactly as the bytes say.
// The texture is bound as a *linear* texture so the sampler does not decode it first. Everything the deferred
// lighting pass could add is switched off by writing a black albedo, no specular and the colour as emissive light.
//
// The vertex stage and the G-buffer layout are the engine's own (Shaders/gbuffer_geometry.slang).

import sturdy_common;
import sturdy_space;
import sturdy_lens;

struct VertexInput {
    float3 position : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
    float4 tangent : TANGENT;
};

struct VertexOutput {
    float4 position : SV_Position;
    float3 worldNormal : NORMAL;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
    // Not used here, but it must be there: the engine pairs this fragment stage with its own vertex stage when it draws
    // with per-object motion history (Shaders/gbuffer_geometry_history.slang), and the two are matched by the order of
    // their outputs. Without this slot, currentClip/previousClip below would be read from the wrong outputs, and the
    // motion vectors (and so the motion blur) would be garbage.
    float4 worldTangent : TANGENT;
    float4 currentClip : TEXCOORD1;
    float4 previousClip : TEXCOORD2;
};

struct GBufferOutput {
    float4 albedo : SV_Target0;
    float2 normal : SV_Target1;
    float4 material : SV_Target2;
    float4 emissive : SV_Target3;
    float2 motion : SV_Target4;
};

struct SceneDrawConstants {
    float4x4 view_projection;
    float4x4 model;
};

#ifdef SFT_EMULATE_PUSH_CONSTANTS
[[vk::binding(0, 3)]] ConstantBuffer<SceneDrawConstants> scene_draw;
#else
[[push_constant]] ConstantBuffer<SceneDrawConstants> scene_draw;
#endif

// Pixels with less alpha than this are not drawn (the skin's transparent texels). Set by the host.
float alpha_cutoff;
// 1: smooth the seams between texels ("texel boundary" anti-aliasing); 0: plain nearest texels. Set by the host.
float texel_aa;

Sampler2D base_color_texture;

static const float3 kLightDirection = normalize(float3(0.35, 1.0, 0.2));

[shader("vertex")]
VertexOutput vertexMain(VertexInput input) {
    VertexOutput output;
    float3 world_position = sturdySpaceModelToWorld(input.position, scene_draw.model);
    output.position = sturdySpaceCameraWorldToClip(world_position, scene_draw.view_projection,
                                                   sturdySpaceLensStrength(scene_draw.model));
    output.worldNormal = sturdySpaceTransformDirection(input.normal, scene_draw.model);
    output.uv = input.uv;
    output.color = input.color;
    output.worldTangent = float4(sturdySpaceTransformDirection(input.tangent.xyz, scene_draw.model), input.tangent.w);
    output.currentClip = output.position;
    output.previousClip = output.position;
    return output;
}

// The face's texel rectangle (min u, min v, max u, max v) comes in as the vertex colour, which is the same for all four
// vertices of a quad so it interpolates to itself.

// The face's texel rectangle (min u, min v, max u, max v) arrives as the vertex colour: the same for all four vertices of a
// quad, so it interpolates to itself.

// A texel of the face's own rectangle, clamped into it so a neighbouring face can never bleed in. The texture is bound
// with a nearest sampler, so sampling a texel's centre returns exactly that texel.
float4 fetchTexel(int2 texel, int2 regionMin, int2 regionMax, float2 dims) {
    int2 clamped = clamp(clamp(texel, regionMin, regionMax), int2(0, 0), int2(dims) - int2(1, 1));
    return base_color_texture.SampleLevel((float2(clamped) + 0.5) / dims, 0.0);
}

float4 premultiplied(float4 c) {
    return float4(c.rgb * c.a, c.a);
}

// The launcher's "texel boundary" anti-aliasing: a bilinear fetch whose blend zone is as wide as a screen pixel, so the
// seam between two texels is smoothed to about one pixel and the texels stay square otherwise. Blended premultiplied, so
// transparent texels (stored black) do not darken the edge of a cut-out.
float4 sampleTexelBoundaryAa(float2 uv, float4 uvBounds, float2 dims, float2 footprint) {
    int2 regionMin = int2(floor(uvBounds.xy * dims + 0.01));
    int2 regionMax = max(int2(ceil(uvBounds.zw * dims - 0.01)) - int2(1, 1), regionMin);
    float2 p = uv * dims - 0.5;
    int2 base = int2(floor(p));
    float2 f = frac(p);
    float2 edgeWidth = clamp(max(footprint, float2(0.0001)) * 0.75, float2(0.0), float2(0.5));
    float2 t = smoothstep(float2(0.5) - edgeWidth, float2(0.5) + edgeWidth, f);
    float4 c00 = premultiplied(fetchTexel(base, regionMin, regionMax, dims));
    float4 c10 = premultiplied(fetchTexel(base + int2(1, 0), regionMin, regionMax, dims));
    float4 c01 = premultiplied(fetchTexel(base + int2(0, 1), regionMin, regionMax, dims));
    float4 c11 = premultiplied(fetchTexel(base + int2(1, 1), regionMin, regionMax, dims));
    float4 blended = lerp(lerp(c00, c10, t.x), lerp(c01, c11, t.x), t.y);
    if (blended.a <= 0.0001) {
        return float4(0.0);
    }
    return float4(blended.rgb / blended.a, blended.a);
}

// The surface colour of a fragment, shared by the depth prepass and the G-buffer pass so both agree on what is drawn.
float4 surfaceTexel(VertexOutput input) {
    uint width;
    uint height;
    base_color_texture.GetDimensions(width, height);
    float2 dims = float2(max(width, 1u), max(height, 1u));
    // Derivatives outside any branch: the sample positions must be the same for every pixel of a quad.
    float2 footprint = fwidth(input.uv * dims);
    float4 nearest = base_color_texture.Sample(input.uv);
    return texel_aa > 0.5 ? sampleTexelBoundaryAa(input.uv, input.color, dims, footprint) : nearest;
}

[shader("fragment")]
GBufferOutput fragmentMain(VertexOutput input) {
    GBufferOutput output;
    float4 texel = surfaceTexel(input);
    if (texel.a < alpha_cutoff) {
        discard;
    }
    float3 normal = normalize(input.worldNormal);
    float shade = 0.58 + max(dot(normal, kLightDirection), 0.0) * 0.42;
    float3 displayColor = saturate(texel.rgb * shade);

    output.albedo = float4(0.0, 0.0, 0.0, 1.0);
    output.normal = encodeOctahedralNormal(normal);
    // roughness 1, metallic 0, no occlusion darkening, and no specular reflectance at all.
    output.material = float4(1.0, 0.0, 1.0, 0.0);
    output.emissive = float4(srgbToLinear(displayColor), 1.0);
    float2 currentNdc = input.currentClip.xy * safeRcp(input.currentClip.w);
    float2 previousNdc = input.previousClip.xy * safeRcp(input.previousClip.w);
    output.motion = (currentNdc - previousNdc) * 0.5;
    return output;
}

[shader("fragment")]
void depthOnlyMain(VertexOutput input) {
    if (surfaceTexel(input).a < alpha_cutoff) {
        discard;
    }
}
