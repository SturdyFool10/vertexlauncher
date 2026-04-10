struct Vertex3DMaterialData
{
    float4 base_color_factor;
    float4 emissive_factor;
    float4 surface_factors;
    uint4 flags;
};

// @vertex3d.resource role=material.uniform
[[vk::binding(0, 1)]] ConstantBuffer<Vertex3DMaterialData> vertex3d_material;

// @vertex3d.resource role=material.base_color_texture
[[vk::binding(1, 1)]] Texture2D<float4> vertex3d_base_color_texture;

// @vertex3d.resource role=material.metallic_roughness_texture
[[vk::binding(2, 1)]] Texture2D<float4> vertex3d_metallic_roughness_texture;

// @vertex3d.resource role=material.normal_texture
[[vk::binding(3, 1)]] Texture2D<float4> vertex3d_normal_texture;

// @vertex3d.resource role=material.emissive_texture
[[vk::binding(4, 1)]] Texture2D<float4> vertex3d_emissive_texture;

// @vertex3d.resource role=material.occlusion_texture
[[vk::binding(5, 1)]] Texture2D<float4> vertex3d_occlusion_texture;

// @vertex3d.resource role=material.sampler
[[vk::binding(6, 1)]] SamplerState vertex3d_material_sampler;

float4 vertex3d_sample_base_color(float2 uv)
{
    return vertex3d_base_color_texture.Sample(vertex3d_material_sampler, uv)
        * vertex3d_material.base_color_factor;
}

float2 vertex3d_sample_metallic_roughness(float2 uv)
{
    float4 sample = vertex3d_metallic_roughness_texture.Sample(vertex3d_material_sampler, uv);
    return sample.xy * vertex3d_material.surface_factors.xy;
}

float3 vertex3d_sample_normal(float2 uv)
{
    float3 normal = vertex3d_normal_texture.Sample(vertex3d_material_sampler, uv).xyz * 2.0 - 1.0;
    return normalize(float3(normal.xy * vertex3d_material.surface_factors.z, normal.z));
}

float3 vertex3d_sample_emissive(float2 uv)
{
    return vertex3d_emissive_texture.Sample(vertex3d_material_sampler, uv).xyz
        * vertex3d_material.emissive_factor.xyz;
}

float vertex3d_sample_occlusion(float2 uv)
{
    return vertex3d_occlusion_texture.Sample(vertex3d_material_sampler, uv).x
        * vertex3d_material.surface_factors.w;
}

bool vertex3d_material_is_pbr()
{
    return vertex3d_material.flags.x != 0;
}
