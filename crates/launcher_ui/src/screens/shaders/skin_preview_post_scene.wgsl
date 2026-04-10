struct Globals_std140_0
{
    @align(16) screen_size_points_0 : vec2<f32>,
    @align(8) _pad_0 : vec2<f32>,
};

@binding(0) @group(1) var<uniform> globals_0 : Globals_std140_0;
struct Scalars_std140_0
{
    @align(16) value_0 : vec4<f32>,
};

@binding(0) @group(2) var<uniform> scalars_0 : Scalars_std140_0;
@binding(0) @group(0) var preview_tex_0 : texture_2d<f32>;

@binding(1) @group(0) var preview_sampler_0 : sampler;

struct VertexOut_0
{
    @builtin(position) pos_0 : vec4<f32>,
    @location(0) uv_0 : vec2<f32>,
    @location(1) color_0 : vec4<f32>,
};

struct vertexInput_0
{
    @location(0) pos_points_0 : vec2<f32>,
    @location(1) camera_z_0 : f32,
    @location(2) uv_1 : vec2<f32>,
    @location(3) color_1 : vec4<f32>,
};

@vertex
fn vs_main( _S1 : vertexInput_0) -> VertexOut_0
{
    var _S2 : f32 = max(_S1.camera_z_0, 1.50010001659393311f);
    var output_0 : VertexOut_0;
    output_0.pos_0 = vec4<f32>((_S1.pos_points_0.x / globals_0.screen_size_points_0.x * 2.0f - 1.0f) * _S2, (1.0f - _S1.pos_points_0.y / globals_0.screen_size_points_0.y * 2.0f) * _S2, _S2 - 1.5f, _S2);
    output_0.uv_0 = _S1.uv_1;
    output_0.color_0 = _S1.color_1;
    return output_0;
}

fn load_preview_texel_0( texel_0 : vec2<i32>,  dims_i_0 : vec2<u32>) -> vec4<f32>
{
    var _S3 : vec3<i32> = vec3<i32>(clamp(texel_0, vec2<i32>(i32(0), i32(0)), vec2<i32>(dims_i_0) - vec2<i32>(i32(1))), i32(0));
    return (textureLoad((preview_tex_0), ((_S3)).xy, ((_S3)).z));
}

fn sample_preview_texel_border_aa_0( uv_2 : vec2<f32>) -> vec4<f32>
{
    var dims_i_1 : vec2<u32>;
    var _S4 : u32 = dims_i_1[i32(0)];
    var _S5 : u32 = dims_i_1[i32(1)];
    {var dim = textureDimensions((preview_tex_0));((_S4)) = dim.x;((_S5)) = dim.y;};
    dims_i_1[i32(0)] = _S4;
    dims_i_1[i32(1)] = _S5;
    var _S6 : vec2<f32> = vec2<f32>(dims_i_1);
    var texel_1 : vec2<f32> = vec2<f32>(0.5f) / _S6;
    var _S7 : vec2<f32> = vec2<f32>(0.5f);
    var p_0 : vec2<f32> = clamp(uv_2, texel_1, vec2<f32>(1.0f) - texel_1) * _S6 - _S7;
    var _S8 : vec2<i32> = vec2<i32>(floor(p_0));
    var edge_width_0 : vec2<f32> = clamp(max((fwidth((p_0))), vec2<f32>(0.00009999999747379f)) * vec2<f32>(0.75f), vec2<f32>(0.0f), _S7);
    var t_0 : vec2<f32> = smoothstep(_S7 - edge_width_0, _S7 + edge_width_0, fract(p_0));
    var _S9 : vec4<f32> = vec4<f32>(t_0.x);
    return mix(mix(load_preview_texel_0(_S8, dims_i_1), load_preview_texel_0(_S8 + vec2<i32>(i32(1), i32(0)), dims_i_1), _S9), mix(load_preview_texel_0(_S8 + vec2<i32>(i32(0), i32(1)), dims_i_1), load_preview_texel_0(_S8 + vec2<i32>(i32(1), i32(1)), dims_i_1), _S9), vec4<f32>(t_0.y));
}

fn sample_preview_pixel_art_0( uv_3 : vec2<f32>) -> vec4<f32>
{
    var dims_i_2 : vec2<u32>;
    var _S10 : u32 = dims_i_2[i32(0)];
    var _S11 : u32 = dims_i_2[i32(1)];
    {var dim = textureDimensions((preview_tex_0));((_S10)) = dim.x;((_S11)) = dim.y;};
    dims_i_2[i32(0)] = _S10;
    dims_i_2[i32(1)] = _S11;
    var _S12 : vec2<f32> = vec2<f32>(dims_i_2);
    var texel_2 : vec2<f32> = vec2<f32>(0.5f) / _S12;
    var clamped_uv_0 : vec2<f32> = clamp(uv_3, texel_2, vec2<f32>(1.0f) - texel_2);
    var uv_grad_x_0 : vec2<f32> = dpdx(clamped_uv_0);
    var uv_grad_y_0 : vec2<f32> = dpdy(clamped_uv_0);
    var texel_grad_x_0 : vec2<f32> = uv_grad_x_0 * _S12;
    var texel_grad_y_0 : vec2<f32> = uv_grad_y_0 * _S12;
    var _S13 : vec3<i32> = vec3<i32>(clamp(vec2<i32>(clamped_uv_0 * _S12), vec2<i32>(i32(0), i32(0)), vec2<i32>(dims_i_2) - vec2<i32>(i32(1))), i32(0));
    return mix((textureLoad((preview_tex_0), ((_S13)).xy, ((_S13)).z)), (textureSampleGrad((preview_tex_0), (preview_sampler_0), (clamped_uv_0), (uv_grad_x_0), (uv_grad_y_0))), vec4<f32>(smoothstep(0.85000002384185791f, 1.35000002384185791f, max(max(abs(texel_grad_x_0.x), abs(texel_grad_x_0.y)), max(abs(texel_grad_y_0.x), abs(texel_grad_y_0.y))))));
}

struct pixelOutput_0
{
    @location(0) output_1 : vec4<f32>,
};

struct pixelInput_0
{
    @location(0) uv_4 : vec2<f32>,
    @location(1) color_2 : vec4<f32>,
};

@fragment
fn fs_main( _S14 : pixelInput_0, @builtin(position) pos_1 : vec4<f32>) -> pixelOutput_0
{
    var _S15 : vec4<f32>;
    if((scalars_0.value_0.x) > 0.5f)
    {
        _S15 = sample_preview_texel_border_aa_0(_S14.uv_4);
    }
    else
    {
        _S15 = sample_preview_pixel_art_0(_S14.uv_4);
    }
    var sampled_0 : vec4<f32> = _S15 * _S14.color_2;
    if((sampled_0.w) <= 0.00100000004749745f)
    {
        discard;
    }
    var _S16 : pixelOutput_0 = pixelOutput_0( sampled_0 );
    return _S16;
}

