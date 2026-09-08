// CUT3 — Cheap Upscaling via Triangulation (swordfish90,
// https://swordfish90.github.io/cheap-upscaling-triangulation/).
//
// Sample the 2×2 source neighbourhood, pick the diagonal whose
// endpoints have the closer luma (so edges along that diagonal stay
// sharp), and barycentrically interpolate the three corners of the
// triangle the sub-pixel falls into.  Gives anti-aliased
// diagonal/curved lines without the blur of bilinear or the ringing
// of bicubic.
//
// Branchless barycentric — weights computed via `min`/`max` so the
// same expression covers both triangles of each diagonal.

fn luma(c: vec4<f32>) -> f32 {
    return dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let pos = in.uv * src_size() - 0.5;
    let base = floor(pos);
    let f = pos - base;

    let uv_tl = (base + vec2<f32>(0.5, 0.5)) * inv_src_size();
    let uv_tr = (base + vec2<f32>(1.5, 0.5)) * inv_src_size();
    let uv_bl = (base + vec2<f32>(0.5, 1.5)) * inv_src_size();
    let uv_br = (base + vec2<f32>(1.5, 1.5)) * inv_src_size();

    let tl = textureSample(src_tex, src_samp, uv_tl);
    let tr = textureSample(src_tex, src_samp, uv_tr);
    let bl = textureSample(src_tex, src_samp, uv_bl);
    let br = textureSample(src_tex, src_samp, uv_br);

    // Main diagonal (TL–BR) barycentric — both triangles in one formula.
    let m_tl = 1.0 - max(f.x, f.y);
    let m_tr = max(f.x - f.y, 0.0);
    let m_bl = max(f.y - f.x, 0.0);
    let m_br = min(f.x, f.y);
    let c_main = tl * m_tl + tr * m_tr + bl * m_bl + br * m_br;

    // Anti-diagonal (TR–BL) barycentric — both triangles in one formula.
    let a_tl = max(1.0 - f.x - f.y, 0.0);
    let a_tr = min(f.x, 1.0 - f.y);
    let a_bl = min(f.y, 1.0 - f.x);
    let a_br = max(f.x + f.y - 1.0, 0.0);
    let c_anti = tl * a_tl + tr * a_tr + bl * a_bl + br * a_br;

    let d_main = abs(luma(tl) - luma(br));
    let d_anti = abs(luma(tr) - luma(bl));
    let col = select(c_anti, c_main, d_main < d_anti);

    return col * in.color;
}
