// Rasterize a binary building-occlusion mask into the stencil buffer.
// The same pipeline also clears the affected stencil rectangle by sampling
// the renderer's solid-white texture with stencil reference zero.

struct ScreenUniform {
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> screen: ScreenUniform;
@group(1) @binding(0) var mask_alpha: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(vin: VsIn) -> VsOut {
    let ndc_x = (vin.pos.x / screen.screen_size.x) * 2.0 - 1.0;
    let ndc_y = -(vin.pos.y / screen.screen_size.y) * 2.0 + 1.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.uv = vin.uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) {
    // RuntimeMask bitmaps are binary. Nearest sampling would visibly jump at
    // non-integral zoom, so retain the renderer's linear sampler and choose
    // the nearest binary result at the fragment boundary.
    if (textureSample(mask_alpha, samp, in.uv).r < 0.5) {
        discard;
    }
}
