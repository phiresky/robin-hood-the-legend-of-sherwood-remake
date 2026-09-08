// xBR level 1 (Hyllian, 2014) — single-pass version.
//
// libretro's xbr shader family produces edge-preserving smoothed
// upscales; level 1 is the simplest variant and fits into one
// fragment pass.  This is a port of the public-domain xbr-lv1-noblend
// preset from https://github.com/libretro/slang-shaders tree
// `edge-smoothing/xbr/shaders/xbr-lv1-noblend.slang`, rewritten in
// WGSL.
//
// The algorithm inspects a 5×5 neighborhood around the current texel
// and interpolates along detected edges.  Unlike Scale2x, it does
// gradient-aware blending so it works at arbitrary destination ratios
// (not just integer multiples).  The "noblend" variant skips the
// alpha cross-over pass level 2/3 add for runtime cost.

fn rgb_brightness(c: vec4<f32>) -> f32 {
    return dot(c.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn df(a: vec4<f32>, b: vec4<f32>) -> f32 {
    return abs(rgb_brightness(a) - rgb_brightness(b));
}

fn eq_xbr(a: vec4<f32>, b: vec4<f32>) -> bool {
    return df(a, b) < 0.04;
}

fn weighted_distance(a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>,
                     e: vec4<f32>, f: vec4<f32>, g: vec4<f32>, h: vec4<f32>) -> f32 {
    return 4.0 * df(a, b) + df(c, d) + df(e, f) + df(g, h);
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let src_xy = in.uv * src_size();
    let base = floor(src_xy);
    let sub = fract(src_xy);
    let inv = inv_src_size();

    // Read the 3×3 neighborhood (A B C / D E F / G H I) — the full xBR
    // reference uses 5×5; the noblend lv1 variant cares about the
    // corner quadrant only, so 3×3 suffices for the edge tests.
    let A = textureSample(src_tex, src_samp, (base + vec2<f32>(-0.5, -0.5)) * inv);
    let B = textureSample(src_tex, src_samp, (base + vec2<f32>( 0.5, -0.5)) * inv);
    let C = textureSample(src_tex, src_samp, (base + vec2<f32>( 1.5, -0.5)) * inv);
    let D = textureSample(src_tex, src_samp, (base + vec2<f32>(-0.5,  0.5)) * inv);
    let E = textureSample(src_tex, src_samp, (base + vec2<f32>( 0.5,  0.5)) * inv);
    let F = textureSample(src_tex, src_samp, (base + vec2<f32>( 1.5,  0.5)) * inv);
    let G = textureSample(src_tex, src_samp, (base + vec2<f32>(-0.5,  1.5)) * inv);
    let H = textureSample(src_tex, src_samp, (base + vec2<f32>( 0.5,  1.5)) * inv);
    let I = textureSample(src_tex, src_samp, (base + vec2<f32>( 1.5,  1.5)) * inv);

    var out = E;

    // Upper-left corner (sub < 0.5, sub < 0.5): compare diagonal A↘
    // against off-diagonal.  Rough port of
    //   FX(DB, EA, DB, EA) ? D : E; in the reference shader.
    if sub.x < 0.5 && sub.y < 0.5 {
        let db = weighted_distance(D, B, E, A, D, B, E, A);
        let ef = weighted_distance(E, A, D, B, E, A, D, B);
        if db < ef && !eq_xbr(E, B) && !eq_xbr(E, D) {
            out = mix(D, B, 0.5);
        }
    }
    // Upper-right
    if sub.x >= 0.5 && sub.y < 0.5 {
        let bf = weighted_distance(B, F, E, C, B, F, E, C);
        let ce = weighted_distance(E, C, B, F, E, C, B, F);
        if bf < ce && !eq_xbr(E, B) && !eq_xbr(E, F) {
            out = mix(B, F, 0.5);
        }
    }
    // Lower-left
    if sub.x < 0.5 && sub.y >= 0.5 {
        let dh = weighted_distance(D, H, E, G, D, H, E, G);
        let ge = weighted_distance(E, G, D, H, E, G, D, H);
        if dh < ge && !eq_xbr(E, D) && !eq_xbr(E, H) {
            out = mix(D, H, 0.5);
        }
    }
    // Lower-right
    if sub.x >= 0.5 && sub.y >= 0.5 {
        let hf = weighted_distance(H, F, E, I, H, F, E, I);
        let ie = weighted_distance(E, I, H, F, E, I, H, F);
        if hf < ie && !eq_xbr(E, H) && !eq_xbr(E, F) {
            out = mix(H, F, 0.5);
        }
    }

    return out * in.color;
}
