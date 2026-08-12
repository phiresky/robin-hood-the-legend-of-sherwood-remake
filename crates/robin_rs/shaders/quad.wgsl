// Textured-quad pipeline used for every renderer overlay draw.
// Quads are submitted as 6 vertices (two triangles) — vertex pos /
// uv come from a per-frame vertex buffer, screen size comes from a
// uniform so the vertex shader can map pixel-space positions to NDC.

struct ScreenUniform {
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> screen: ScreenUniform;
@group(1) @binding(0) var sampled_tex: texture_2d<f32>;
@group(1) @binding(1) var sampled_samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,   // pixel-space dest coord
    @location(1) uv:  vec2<f32>,   // 0..1 within the source texture
    @location(2) tint: vec4<f32>,  // multiplied with the sampled texel
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs_main(vin: VsIn) -> VsOut {
    // Pixel-space → NDC. Y flipped because (0,0) is top-left in pixel
    // space and bottom-left in NDC.
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
    let tex = textureSample(sampled_tex, sampled_samp, in.uv);
    return tex * in.tint;
}

// HSV-replace colorize used by `Renderer::colorize_framebuffer` /
// `dim_screen`. Mirrors legacy menu dimming behavior: per-pixel sample the frozen scene, pre-scale
// (r*=2*scale, g*=scale, b*=scale), convert RGB→HSV to get S and V,
// then convert back using the desired hue (passed via `tint.x` as
// hue/360, `tint.y` as scale).
@fragment
fn fs_colorize(in: VsOut) -> @location(0) vec4<f32> {
    let src   = textureSample(sampled_tex, sampled_samp, in.uv);
    let hue   = in.tint.x * 360.0;
    let scale = in.tint.y;

    // legacy implementation pre-multiplies before the HSV conversion so the dim picks
    // up a warm bias (red doubled). Clamp to <=1 — V saturates at 1
    // anyway and HSV<->RGB only makes sense in [0,1].
    let pre = vec3<f32>(src.r * 2.0 * scale, src.g * scale, src.b * scale);
    let r = min(pre.r, 1.0);
    let g = min(pre.g, 1.0);
    let b = min(pre.b, 1.0);

    // RGB → HSV; we only need S and V (H is replaced).
    let mx = max(r, max(g, b));
    let mn = min(r, min(g, b));
    let v  = mx;
    var s  = 0.0;
    if (mx > 0.0) { s = (mx - mn) / mx; }

    // HSV → RGB using the desired hue. Standard sextant lookup.
    let h = hue / 60.0;
    let hi = i32(floor(h)) % 6;
    let f  = h - floor(h);
    let p  = v * (1.0 - s);
    let q  = v * (1.0 - s * f);
    let t  = v * (1.0 - s * (1.0 - f));
    var rgb = vec3<f32>(0.0);
    switch hi {
        case 0:        { rgb = vec3<f32>(v, t, p); }
        case 1:        { rgb = vec3<f32>(q, v, p); }
        case 2:        { rgb = vec3<f32>(p, v, t); }
        case 3:        { rgb = vec3<f32>(p, q, v); }
        case 4:        { rgb = vec3<f32>(t, p, v); }
        case 5, default: { rgb = vec3<f32>(v, p, q); }
    }
    return vec4<f32>(rgb, 1.0);
}

// Exact piecewise sRGB transfer curve (12.92 linear toe, 2.4 exponent),
// matching the CPU-side conversions so alphas/colours computed on the
// CPU round-trip through the same curve on the GPU.
fn linear_to_srgb_channel(c: f32) -> f32 {
    let x = clamp(c, 0.0, 1.0);
    if (x <= 0.0031308) {
        return x * 12.92;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

fn srgb_to_linear_channel(c: f32) -> f32 {
    let x = clamp(c, 0.0, 1.0);
    if (x <= 0.04045) {
        return x / 12.92;
    }
    return pow((x + 0.055) / 1.055, 2.4);
}

// Background-sampled alpha polygon fill used by door/patch hover
// highlights. `in.uv` is already background-texture UV. `tint.rgb` is
// the overlay colour in sRGB, `tint.a` is alpha on the legacy implementation 0..256 scale.
@fragment
fn fs_bg_alpha_polygon(in: VsOut) -> @location(0) vec4<f32> {
    let bg_linear = textureSample(sampled_tex, sampled_samp, in.uv).rgb;
    let bg_srgb = vec3<f32>(
        linear_to_srgb_channel(bg_linear.r),
        linear_to_srgb_channel(bg_linear.g),
        linear_to_srgb_channel(bg_linear.b),
    );

    let alpha = clamp(in.tint.a, 0.0, 1.0);
    let inv_alpha = 1.0 - alpha;

    // Match the old CPU path's RGB565-channel blend closely: quantize
    // source and colour into 5/6/5 channels, blend on that scale, then
    // expand by MSB replication before writing to the sRGB render target.
    let sr = floor(bg_srgb.r * 31.0 + 0.0001);
    let sg = floor(bg_srgb.g * 63.0 + 0.0001);
    let sb = floor(bg_srgb.b * 31.0 + 0.0001);
    let cr = floor(clamp(in.tint.r, 0.0, 1.0) * 31.0 + 0.0001);
    let cg = floor(clamp(in.tint.g, 0.0, 1.0) * 63.0 + 0.0001);
    let cb = floor(clamp(in.tint.b, 0.0, 1.0) * 31.0 + 0.0001);

    let r5 = floor(sr * inv_alpha + cr * alpha);
    let g6 = floor(sg * inv_alpha + cg * alpha);
    let b5 = floor(sb * inv_alpha + cb * alpha);

    let r8 = (r5 * 8.0 + floor(r5 / 4.0)) / 255.0;
    let g8 = (g6 * 4.0 + floor(g6 / 16.0)) / 255.0;
    let b8 = (b5 * 8.0 + floor(b5 / 4.0)) / 255.0;

    let out_linear = vec3<f32>(
        srgb_to_linear_channel(r8),
        srgb_to_linear_channel(g8),
        srgb_to_linear_channel(b8),
    );
    return vec4<f32>(out_linear, 1.0);
}

// View-cone overlay. The Rust scanline pass submits one-pixel-high GPU
// spans and stores the interpolated alpha in uv.x; tint.rgb is the alert
// colour. This keeps mask clipping in geometry while the actual fill/blend
// happens on the GPU.
@fragment
fn fs_view_cone_gradient(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.tint.rgb, clamp(in.uv.x, 0.0, 1.0));
}
