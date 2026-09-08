// Sharp-bilinear (blargg, popularised via RetroArch).  Scales the
// source to the nearest integer multiple with nearest-neighbor, then
// finishes with a single bilinear tap on the sub-pixel remainder.
// Result: pixel-perfect crispness on axis-aligned edges without the
// "wobble" plain nearest shows at fractional scales, and no blur on
// the interior of each source texel.
//
// At 1:1 (dst == src) this degrades to plain bilinear — nothing to
// sharpen.  Single `textureSample`, branch-free.

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let texel = in.uv * src_size();
    // `ceil` instead of the classic `floor`: at 1.8× upscale we still
    // want visible sharpening (floor(1.8)=1 collapses to plain
    // bilinear).  `ceil` snaps to the next integer multiple so the
    // transition band stays narrow at any non-trivial upscale.
    let scale = max(ceil(dst_size() * inv_src_size()), vec2<f32>(1.0));

    let texel_floor = floor(texel);
    let s = fract(texel);
    let region_range = 0.5 - 0.5 / scale;
    let center_dist = s - 0.5;
    let f = (center_dist - clamp(center_dist, -region_range, region_range)) * scale + 0.5;

    let uv = (texel_floor + f) * inv_src_size();
    return textureSample(src_tex, src_samp, uv) * in.color;
}
