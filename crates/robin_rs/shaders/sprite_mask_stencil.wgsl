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
    @location(1) params: vec2<f32>,
};

@vertex
fn vs_main(vin: VsIn) -> VsOut {
    let ndc_x = (vin.pos.x / screen.screen_size.x) * 2.0 - 1.0;
    let ndc_y = -(vin.pos.y / screen.screen_size.y) * 2.0 + 1.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.uv = vin.uv;
    out.params = vin.tint.rg;
    return out;
}

@fragment
fn fs_main(in: VsOut) {
    // RuntimeMask bitmaps are binary. Nearest sampling would visibly jump at
    // non-integral zoom, so retain the renderer's linear sampler and choose
    // the nearest binary result at the fragment boundary.
    var value = textureSample(mask_alpha, samp, in.uv).r;
    if (in.params.y > 0.5) {
        // Continuous depth is stored as big-endian high/low bytes in RG8.
        // Reconstruct before interpolation: interpolating either byte across
        // a carry boundary would otherwise produce a wildly incorrect depth.
        let dims = vec2<f32>(textureDimensions(mask_alpha));
        let p = in.uv * dims - vec2<f32>(0.5);
        let base = vec2<i32>(floor(p));
        let fraction = fract(p);
        let max_coord = vec2<i32>(textureDimensions(mask_alpha)) - vec2<i32>(1);
        let c00 = clamp(base, vec2<i32>(0), max_coord);
        let c10 = clamp(base + vec2<i32>(1, 0), vec2<i32>(0), max_coord);
        let c01 = clamp(base + vec2<i32>(0, 1), vec2<i32>(0), max_coord);
        let c11 = clamp(base + vec2<i32>(1, 1), vec2<i32>(0), max_coord);
        let d00 = textureLoad(mask_alpha, c00, 0).rg;
        let d10 = textureLoad(mask_alpha, c10, 0).rg;
        let d01 = textureLoad(mask_alpha, c01, 0).rg;
        let d11 = textureLoad(mask_alpha, c11, 0).rg;
        let weights = vec2<f32>(256.0, 1.0) / 65535.0;
        let top = mix(dot(round(d00 * 255.0), weights), dot(round(d10 * 255.0), weights), fraction.x);
        let bottom = mix(dot(round(d01 * 255.0), weights), dot(round(d11 * 255.0), weights), fraction.x);
        value = mix(top, bottom, fraction.y);
    }
    if (value <= in.params.x) {
        discard;
    }
}
