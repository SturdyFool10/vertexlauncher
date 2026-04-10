@binding(0) @group(0) var current_tex_0 : texture_2d<f32>;

@binding(0) @group(1) var history_tex_0 : texture_2d<f32>;

struct Scalar_std140_0
{
    @align(16) value_0 : vec4<f32>,
};

@binding(0) @group(2) var<uniform> scalar_0 : Scalar_std140_0;
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

fn load_current_0( pixel_0 : vec2<i32>) -> vec4<f32>
{
    var dims_0 : vec2<u32>;
    var _S1 : u32 = dims_0[i32(0)];
    var _S2 : u32 = dims_0[i32(1)];
    {var dim = textureDimensions((current_tex_0));((_S1)) = dim.x;((_S2)) = dim.y;};
    dims_0[i32(0)] = _S1;
    dims_0[i32(1)] = _S2;
    var _S3 : vec3<i32> = vec3<i32>(clamp(pixel_0, vec2<i32>(i32(0), i32(0)), vec2<i32>(dims_0) - vec2<i32>(i32(1))), i32(0));
    return (textureLoad((current_tex_0), ((_S3)).xy, ((_S3)).z));
}

fn load_history_0( pixel_1 : vec2<i32>) -> vec4<f32>
{
    var dims_1 : vec2<u32>;
    var _S4 : u32 = dims_1[i32(0)];
    var _S5 : u32 = dims_1[i32(1)];
    {var dim = textureDimensions((history_tex_0));((_S4)) = dim.x;((_S5)) = dim.y;};
    dims_1[i32(0)] = _S4;
    dims_1[i32(1)] = _S5;
    var _S6 : vec3<i32> = vec3<i32>(clamp(pixel_1, vec2<i32>(i32(0), i32(0)), vec2<i32>(dims_1) - vec2<i32>(i32(1))), i32(0));
    return (textureLoad((history_tex_0), ((_S6)).xy, ((_S6)).z));
}

struct pixelOutput_0
{
    @location(0) output_1 : vec4<f32>,
};

@fragment
fn fs_main(@builtin(position) pos_1 : vec4<f32>) -> pixelOutput_0
{
    var _S7 : vec2<i32> = vec2<i32>(pos_1.xy);
    var current_0 : vec4<f32> = load_current_0(_S7);
    var lo_0 : vec4<f32> = current_0;
    var hi_0 : vec4<f32> = current_0;
    var y_0 : i32 = i32(-1);
    for(;;)
    {
        if(y_0 <= i32(1))
        {
        }
        else
        {
            break;
        }
        var x_0 : i32 = i32(-1);
        for(;;)
        {
            if(x_0 <= i32(1))
            {
            }
            else
            {
                break;
            }
            var sample_value_0 : vec4<f32> = load_current_0(_S7 + vec2<i32>(x_0, y_0));
            var _S8 : vec4<f32> = min(lo_0, sample_value_0);
            var _S9 : vec4<f32> = max(hi_0, sample_value_0);
            var x_1 : i32 = x_0 + i32(1);
            lo_0 = _S8;
            hi_0 = _S9;
            x_0 = x_1;
        }
        y_0 = y_0 + i32(1);
    }
    var _S10 : pixelOutput_0 = pixelOutput_0( mix(clamp(load_history_0(_S7), lo_0, hi_0), current_0, vec4<f32>(clamp(scalar_0.value_0.x, 0.05000000074505806f, 1.0f))) );
    return _S10;
}

