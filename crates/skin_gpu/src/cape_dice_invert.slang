// The "Random" cape tile's picture: a cape's face with the negative of itself shown wherever the dice icon is drawn.
//
//     colour = lerp(cape, 1 - cape, mask)
//
// `mask` is the alpha of the dice SVG (rasterised by the engine, white on transparent), laid over the picture as a square
// as wide as the picture. Where the dice has no pixels the cape is shown as it is. The surface is written the same way as
// the skin preview's (no engine lighting, the colour as emissive light), so the colours come out as the bytes say.
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
    // Unused, but the engine pairs this fragment stage with its motion-history vertex stage by the order of the outputs.
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

// The picture's height over its width: the dice is a square as wide as the picture, centred. Set by the host.
float mask_scale_y;
// Pixels with less alpha than this are not drawn. Set by the host.
float alpha_cutoff;

Sampler2D cape_texture;
Sampler2D mask_texture;

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

float4 surfaceTexel(VertexOutput input) {
    float4 cape = cape_texture.Sample(input.uv);
    float2 maskUv = float2(input.uv.x, (input.uv.y - 0.5) * mask_scale_y + 0.5);
    float inside = (maskUv.y >= 0.0 && maskUv.y <= 1.0) ? 1.0 : 0.0;
    float mask = mask_texture.Sample(maskUv).a * inside;
    return float4(lerp(cape.rgb, float3(1.0) - cape.rgb, mask), cape.a);
}

[shader("fragment")]
GBufferOutput fragmentMain(VertexOutput input) {
    GBufferOutput output;
    float4 texel = surfaceTexel(input);
    if (texel.a < alpha_cutoff) {
        discard;
    }
    float3 normal = normalize(input.worldNormal);
    output.albedo = float4(0.0, 0.0, 0.0, 1.0);
    output.normal = encodeOctahedralNormal(normal);
    output.material = float4(1.0, 0.0, 1.0, 0.0);
    output.emissive = float4(srgbToLinear(saturate(texel.rgb)), 1.0);
    float2 currentNdc = input.currentClip.xy * safeRcp(input.currentClip.w);
    float2 previousNdc = input.previousClip.xy * safeRcp(input.previousClip.w);
    output.motion = (currentNdc - previousNdc) * 0.5;
    return output;
}

[shader("fragment")]
void depthOnlyMain(VertexOutput input) {
    if (cape_texture.Sample(input.uv).a < alpha_cutoff) {
        discard;
    }
}
