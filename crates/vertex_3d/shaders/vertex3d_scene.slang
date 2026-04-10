import vertex3d_globals;
import vertex3d_material;

float4x4 vertex3d_model_matrix(
    float4 row0,
    float4 row1,
    float4 row2,
    float4 row3)
{
    return float4x4(row0, row1, row2, row3);
}

float4 vertex3d_world_to_clip(float3 world_position)
{
    return mul(vertex3d_globals.view_projection, float4(world_position, 1.0));
}
