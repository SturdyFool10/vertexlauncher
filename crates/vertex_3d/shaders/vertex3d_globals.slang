struct Vertex3DGlobals
{
    float4x4 view_projection;
    float4x4 view;
    float4x4 projection;
    float4 camera_world;
    float4 viewport_size_time;
};

[[vk::binding(0, 0)]] ConstantBuffer<Vertex3DGlobals> vertex3d_globals;

float3 vertex3d_camera_position()
{
    return vertex3d_globals.camera_world.xyz;
}

float2 vertex3d_viewport_size()
{
    return vertex3d_globals.viewport_size_time.xy;
}

float vertex3d_time_seconds()
{
    return vertex3d_globals.viewport_size_time.z;
}
