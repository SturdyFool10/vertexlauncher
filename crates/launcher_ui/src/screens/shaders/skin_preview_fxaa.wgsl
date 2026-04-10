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

fn load_rgba_0( pixel_0 : vec2<i32>) -> vec4<f32>
{
    var dims_0 : vec2<u32>;
    var _S1 : u32 = dims_0[i32(0)];
    var _S2 : u32 = dims_0[i32(1)];
    {var dim = textureDimensions((source_tex_0));((_S1)) = dim.x;((_S2)) = dim.y;};
    dims_0[i32(0)] = _S1;
    dims_0[i32(1)] = _S2;
    var _S3 : vec3<i32> = vec3<i32>(clamp(pixel_0, vec2<i32>(i32(0), i32(0)), vec2<i32>(dims_0) - vec2<i32>(i32(1))), i32(0));
    return (textureLoad((source_tex_0), ((_S3)).xy, ((_S3)).z));
}

fn rgb_luma_0( rgb_0 : vec3<f32>) -> f32
{
    return dot(rgb_0, vec3<f32>(0.29899999499320984f, 0.58700001239776611f, 0.11400000005960464f));
}

fn sample_linear_0( pixel_1 : vec2<f32>) -> vec4<f32>
{
    var dims_1 : vec2<u32>;
    var _S4 : u32 = dims_1[i32(0)];
    var _S5 : u32 = dims_1[i32(1)];
    {var dim = textureDimensions((source_tex_0));((_S4)) = dim.x;((_S5)) = dim.y;};
    dims_1[i32(0)] = _S4;
    dims_1[i32(1)] = _S5;
    var _S6 : vec2<i32> = vec2<i32>(i32(1));
    var p_0 : vec2<f32> = clamp(pixel_1, vec2<f32>(0.0f), vec2<f32>(vec2<i32>(dims_1) - _S6));
    var _S7 : vec2<i32> = vec2<i32>(floor(p_0));
    var p1_0 : vec2<i32> = min(_S7 + vec2<i32>(i32(1), i32(1)), vec2<i32>(dims_1) - _S6);
    var f_0 : vec2<f32> = fract(p_0);
    var _S8 : vec4<f32> = vec4<f32>(f_0.x);
    return mix(mix(load_rgba_0(_S7), load_rgba_0(vec2<i32>(p1_0.x, _S7.y)), _S8), mix(load_rgba_0(vec2<i32>(_S7.x, p1_0.y)), load_rgba_0(p1_0), _S8), vec4<f32>(f_0.y));
}

struct pixelOutput_0
{
    @location(0) output_1 : vec4<f32>,
};

@fragment
fn fs_main(@builtin(position) pos_1 : vec4<f32>) -> pixelOutput_0
{
    var _S9 : vec2<f32> = pos_1.xy;
    var _S10 : vec2<i32> = vec2<i32>(_S9);
    var m_0 : vec4<f32> = load_rgba_0(_S10);
    var luma_nw_0 : f32 = rgb_luma_0(load_rgba_0(_S10 + vec2<i32>(i32(-1), i32(-1))).xyz);
    var luma_ne_0 : f32 = rgb_luma_0(load_rgba_0(_S10 + vec2<i32>(i32(1), i32(-1))).xyz);
    var luma_sw_0 : f32 = rgb_luma_0(load_rgba_0(_S10 + vec2<i32>(i32(-1), i32(1))).xyz);
    var luma_se_0 : f32 = rgb_luma_0(load_rgba_0(_S10 + vec2<i32>(i32(1), i32(1))).xyz);
    var luma_m_0 : f32 = rgb_luma_0(m_0.xyz);
    var _S11 : f32 = min(luma_m_0, min(min(luma_nw_0, luma_ne_0), min(luma_sw_0, luma_se_0)));
    var _S12 : f32 = max(luma_m_0, max(max(luma_nw_0, luma_ne_0), max(luma_sw_0, luma_se_0)));
    if((_S12 - _S11) < (max(0.0625f, _S12 * 0.125f)))
    {
        var _S13 : pixelOutput_0 = pixelOutput_0( m_0 );
        return _S13;
    }
    var _S14 : f32 = luma_nw_0 + luma_ne_0;
    var _S15 : f32 = - (_S14 - (luma_sw_0 + luma_se_0));
    var _S16 : f32 = luma_nw_0 + luma_sw_0 - (luma_ne_0 + luma_se_0);
    var _S17 : vec2<f32> = vec2<f32>(8.0f);
    var dir_0 : vec2<f32> = clamp(vec2<f32>(_S15, _S16) * vec2<f32>((1.0f / (min(abs(_S15), abs(_S16)) + max((_S14 + luma_sw_0 + luma_se_0) * 0.25f * 0.125f, 0.0078125f)))), (vec2<f32>(0) - _S17), _S17);
    var _S18 : vec3<f32> = vec3<f32>(0.5f);
    var rgb_a_0 : vec3<f32> = _S18 * (sample_linear_0(_S9 + dir_0 * vec2<f32>(-0.1666666567325592f)).xyz + sample_linear_0(_S9 + dir_0 * vec2<f32>(0.16666668653488159f)).xyz);
    var rgb_b_0 : vec3<f32> = rgb_a_0 * _S18 + vec3<f32>(0.25f) * (sample_linear_0(_S9 + dir_0 * vec2<f32>(-0.5f)).xyz + sample_linear_0(_S9 + dir_0 * vec2<f32>(0.5f)).xyz);
    var luma_b_0 : f32 = rgb_luma_0(rgb_b_0);
    var _S19 : bool;
    if(luma_b_0 < _S11)
    {
        _S19 = true;
    }
    else
    {
        _S19 = luma_b_0 > _S12;
    }
    var final_rgb_0 : vec3<f32>;
    if(_S19)
    {
        final_rgb_0 = rgb_a_0;
    }
    else
    {
        final_rgb_0 = rgb_b_0;
    }
    var _S20 : pixelOutput_0 = pixelOutput_0( vec4<f32>(final_rgb_0, m_0.w) );
    return _S20;
}

