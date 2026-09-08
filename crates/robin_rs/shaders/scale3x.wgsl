// Scale3x — integer 3× companion of Scale2x, with a full 3×3 output
// pattern instead of 2×2.  Preserves horizontal / vertical / diagonal
// lines; no blending.  Best at integer 3× multiples of the logical
// resolution; at non-integer scales the window blit is bilinear over
// the 3× result.
//
// Reference: https://www.scale2x.it/algorithm (the Scale3x section).

fn eq3(a: vec4<f32>, b: vec4<f32>) -> bool {
    let d = abs(a - b);
    return d.r < 1e-3 && d.g < 1e-3 && d.b < 1e-3 && d.a < 1e-3;
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let src_xy = in.uv * src_size();
    let base = floor(src_xy) + 0.5;
    let sub = fract(src_xy);

    // 8-neighborhood + center.  Naming matches the Scale2x doc's
    // A-I figure: A B C / D E F / G H I.
    let inv = inv_src_size();
    let A = textureSample(src_tex, src_samp, (base + vec2<f32>(-1.0, -1.0)) * inv);
    let B = textureSample(src_tex, src_samp, (base + vec2<f32>( 0.0, -1.0)) * inv);
    let C = textureSample(src_tex, src_samp, (base + vec2<f32>( 1.0, -1.0)) * inv);
    let D = textureSample(src_tex, src_samp, (base + vec2<f32>(-1.0,  0.0)) * inv);
    let E = textureSample(src_tex, src_samp,  base                            * inv);
    let F = textureSample(src_tex, src_samp, (base + vec2<f32>( 1.0,  0.0)) * inv);
    let G = textureSample(src_tex, src_samp, (base + vec2<f32>(-1.0,  1.0)) * inv);
    let H = textureSample(src_tex, src_samp, (base + vec2<f32>( 0.0,  1.0)) * inv);
    let I = textureSample(src_tex, src_samp, (base + vec2<f32>( 1.0,  1.0)) * inv);

    let tb_equal = eq3(B, H);
    let lr_equal = eq3(D, F);
    // Early out — Scale3x preserves the center when two opposite
    // neighbor pairs are identical (no meaningful edge).
    var out = E;
    if !(tb_equal || lr_equal) {
        // 3×3 grid of output cells; index into via (sub.x, sub.y)
        // thirds.
        let col = i32(floor(sub.x * 3.0));
        let row = i32(floor(sub.y * 3.0));
        // Use the algorithm's established neighborhood pattern.
        if row == 0 {
            if col == 0 {
                if eq3(D, B) { out = D; }
            } else if col == 1 {
                if (eq3(D, B) && !eq3(E, C)) || (eq3(B, F) && !eq3(E, A)) {
                    out = B;
                }
            } else {
                if eq3(B, F) { out = F; }
            }
        } else if row == 1 {
            if col == 0 {
                if (eq3(D, B) && !eq3(E, G)) || (eq3(D, H) && !eq3(E, A)) {
                    out = D;
                }
            } else if col == 1 {
                out = E;
            } else {
                if (eq3(B, F) && !eq3(E, I)) || (eq3(H, F) && !eq3(E, C)) {
                    out = F;
                }
            }
        } else { // row == 2
            if col == 0 {
                if eq3(D, H) { out = D; }
            } else if col == 1 {
                if (eq3(D, H) && !eq3(E, I)) || (eq3(H, F) && !eq3(E, G)) {
                    out = H;
                }
            } else {
                if eq3(H, F) { out = F; }
            }
        }
    }
    return out * in.color;
}
