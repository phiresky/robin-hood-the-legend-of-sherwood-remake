// Scale2x (Andrea Mazzoleni, 2001) single-pass port.
// https://www.scale2x.it/algorithm
//
// For each output pixel, inspect the 4-neighborhood (N, S, E, W) of the
// nearest source pixel and pick one of four "corner" cases based on
// which neighbors match; otherwise fall through to the center pixel.
// Produces integer 2× smoothing of stair-step edges — classic emulator
// look.  At non-integer destinations the output's still nearest-sampled
// in screen space, so it's crispest when the window is an integer
// multiple of the logical size.

fn sample_px(uv: vec2<f32>) -> vec4<f32> {
    // Sample at the center of the nearest texel — Scale2x operates on
    // discrete pixels, not the interpolated continuous signal.
    let texel = floor(uv * src_size()) + 0.5;
    return textureSample(src_tex, src_samp, texel * inv_src_size());
}

fn eq(a: vec4<f32>, b: vec4<f32>) -> bool {
    let d = abs(a - b);
    return d.r < 1e-3 && d.g < 1e-3 && d.b < 1e-3 && d.a < 1e-3;
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let src_xy = in.uv * src_size();
    let base = floor(src_xy) + 0.5;
    let sub = fract(src_xy);

    let t = (base + vec2<f32>( 0.0, -1.0)) * inv_src_size();
    let bo = (base + vec2<f32>( 0.0,  1.0)) * inv_src_size();
    let l = (base + vec2<f32>(-1.0,  0.0)) * inv_src_size();
    let r = (base + vec2<f32>( 1.0,  0.0)) * inv_src_size();
    let e = base * inv_src_size();

    let colT = textureSample(src_tex, src_samp, t);
    let colB = textureSample(src_tex, src_samp, bo);
    let colL = textureSample(src_tex, src_samp, l);
    let colR = textureSample(src_tex, src_samp, r);
    let colE = textureSample(src_tex, src_samp, e);

    // Reject "cross" neighbor matches (all four equal) — Scale2x keeps
    // the center pixel in that case.
    let tb_equal_lr = eq(colT, colB) || eq(colL, colR);
    var out = colE;
    if !tb_equal_lr {
        // Quadrant-based corner test: top-left, top-right,
        // bottom-left, bottom-right of the 2×2 output cell.
        if sub.x < 0.5 && sub.y < 0.5 && eq(colT, colL) {
            out = colT;
        } else if sub.x >= 0.5 && sub.y < 0.5 && eq(colT, colR) {
            out = colT;
        } else if sub.x < 0.5 && sub.y >= 0.5 && eq(colB, colL) {
            out = colB;
        } else if sub.x >= 0.5 && sub.y >= 0.5 && eq(colB, colR) {
            out = colB;
        }
    }
    return out * in.color;
}
