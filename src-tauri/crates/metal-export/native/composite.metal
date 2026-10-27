#include <metal_stdlib>
using namespace metal;

// Skia supplies BGRA8Unorm with premultiplied alpha. Texture reads expose RGBA.
// Source/output are BT.709 limited-range NV12 with one chroma value per 2x2 block.
kernel void composite_nv12(
    texture2d<float, access::read> source_y [[texture(0)]],
    texture2d<float, access::read> source_uv [[texture(1)]],
    texture2d<float, access::read> overlay [[texture(2)]],
    texture2d<float, access::write> output_y [[texture(3)]],
    texture2d<float, access::write> output_uv [[texture(4)]],
    uint2 chroma [[thread_position_in_grid]]) {
    if (any(chroma >= uint2(source_uv.get_width(), source_uv.get_height()))) return;
    constexpr float3 y_coeff = float3(.2126, .7152, .0722) * (219.0 / 255.0);
    constexpr float3 u_coeff = float3(-.114572, -.385428, .5) * (224.0 / 255.0);
    constexpr float3 v_coeff = float3(.5, -.454153, -.045847) * (224.0 / 255.0);
    float4 average = 0;
    for (uint y = 0; y < 2; ++y) {
        for (uint x = 0; x < 2; ++x) {
            uint2 pixel = chroma * 2 + uint2(x, y);
            float4 color = overlay.read(pixel);
            float value = source_y.read(pixel).r * (1.0 - color.a)
                + dot(color.rgb, y_coeff) + (16.0 / 255.0) * color.a;
            output_y.write(float4(value, 0, 0, 1), pixel);
            average += color * .25;
        }
    }
    float2 uv = source_uv.read(chroma).rg * (1.0 - average.a)
        + float2(dot(average.rgb, u_coeff), dot(average.rgb, v_coeff))
        + (128.0 / 255.0) * average.a;
    output_uv.write(float4(uv, 0, 1), chroma);
}
