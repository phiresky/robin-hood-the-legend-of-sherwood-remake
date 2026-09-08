// Loading-screen dissolve. Samples the initial image, final image, and
// pre-normalized height mask. tint.x carries the current threshold in 0..1.

struct ScreenUniform {
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> screen: ScreenUniform;
@group(1) @binding(0) var initial_tex: texture_2d<f32>;
@group(1) @binding(1) var final_tex:   texture_2d<f32>;
@group(1) @binding(2) var mask_tex:    texture_2d<f32>;
@group(1) @binding(3) var samp:        sampler;

struct VsIn {
    @location(0) pos:  vec2<f32>,
    @location(1) uv:   vec2<f32>,
    @location(2) tint: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv:   vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs_main(vin: VsIn) -> VsOut {
    let ndc_x =  (vin.pos.x / screen.screen_size.x) * 2.0 - 1.0;
    let ndc_y = -(vin.pos.y / screen.screen_size.y) * 2.0 + 1.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.uv = vin.uv;
    out.tint = vin.tint;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let threshold = clamp(in.tint.x, 0.0, 1.0039216);
    let h = textureSample(mask_tex, samp, in.uv).r;
    let a = textureSample(initial_tex, samp, in.uv);
    let b = textureSample(final_tex, samp, in.uv);
    return select(a, b, h > threshold);
}
