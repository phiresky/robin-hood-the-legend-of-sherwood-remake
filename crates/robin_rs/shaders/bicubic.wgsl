// Bicubic upscaler — Mitchell–Netravali B=C=0 (Catmull-Rom) kernel,
// sampled via 4 bilinear fetches for performance (Reinders, 2010).
//
// Produces a smoother result than linear but avoids the halation /
// ringing of Lanczos.  Good general-purpose choice for pixel art at
// fractional scales where nearest / pixelart look too blocky.

fn cubic_weights(v: f32) -> vec4<f32> {
    let n = vec4<f32>(1.0, 2.0, 3.0, 4.0) - v;
    let s = n * n * n;
    let x = s.x;
    let y = s.y - 4.0 * s.x;
    let z = s.z - 4.0 * s.y + 6.0 * s.x;
    let w = 6.0 - x - y - z;
    return vec4<f32>(x, y, z, w) * (1.0 / 6.0);
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let pos = in.uv * src_size() - 0.5;
    let fxy = fract(pos);
    let int_pos = pos - fxy;

    let cx = cubic_weights(fxy.x);
    let cy = cubic_weights(fxy.y);

    let s = vec4<f32>(cx.x + cx.y, cx.z + cx.w, cy.x + cy.y, cy.z + cy.w);
    let offset = vec4<f32>(
        int_pos.x - 0.5 + cx.y / s.x,
        int_pos.x + 1.5 + cx.w / s.y,
        int_pos.y - 0.5 + cy.y / s.z,
        int_pos.y + 1.5 + cy.w / s.w,
    ) * inv_src_size().xxyy;

    let sample0 = textureSample(src_tex, src_samp, offset.xz);
    let sample1 = textureSample(src_tex, src_samp, offset.yz);
    let sample2 = textureSample(src_tex, src_samp, offset.xw);
    let sample3 = textureSample(src_tex, src_samp, offset.yw);

    let sx = s.x / (s.x + s.y);
    let sy = s.z / (s.z + s.w);
    return mix(
        mix(sample3, sample2, sx),
        mix(sample1, sample0, sx),
        sy,
    ) * in.color;
}
