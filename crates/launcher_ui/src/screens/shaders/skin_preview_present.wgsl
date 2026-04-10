@binding(0) @group(0) var source_tex_0 : texture_2d<f32>;

struct FullscreenOut_0
{
    @builtin(position) pos_0 : vec4<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index_0 : u32) -> FullscreenOut_0
{
    var output_0 : FullscreenOut_0;
    output_0.pos_0 = vec4<f32>(f32((((vertex_index_0 << (u32(1)))) & (u32(2)))) * 2.0f - 1.0f, 1.0f - f32((vertex_index_0 & (u32(2)))) * 2.0f, 0.0f, 1.0f);
    return output_0;
}

struct pixelOutput_0
{
    @location(0) output_1 : vec4<f32>,
};

@fragment
fn fs_main(@builtin(position) pos_1 : vec4<f32>) -> pixelOutput_0
{
    var dims_0 : vec2<u32>;
    var _S1 : u32 = dims_0[i32(0)];
    var _S2 : u32 = dims_0[i32(1)];
    {var dim = textureDimensions((source_tex_0));((_S1)) = dim.x;((_S2)) = dim.y;};
    dims_0[i32(0)] = _S1;
    dims_0[i32(1)] = _S2;
    var _S3 : vec3<i32> = vec3<i32>(clamp(vec2<i32>(pos_1.xy), vec2<i32>(i32(0), i32(0)), vec2<i32>(dims_0) - vec2<i32>(i32(1))), i32(0));
    var _S4 : pixelOutput_0 = pixelOutput_0( (textureLoad((source_tex_0), ((_S3)).xy, ((_S3)).z)) );
    return _S4;
}

