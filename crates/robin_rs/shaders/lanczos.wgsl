// Lanczos-2 (a=2) upscaler — 4×4 tap grid, hand-unrolled for broader
// driver compatibility.  Sharper than bicubic with some ringing; no
// sinc-of-sinc term past `a=2` to keep the inner loop free of branches
// that confuse some Vulkan drivers' SPIR-V control-flow validator.
// We dropped the 6×6 version (Lanczos-3) — it compiled but triggered
// a GPU reset on some Mesa/RADV builds.

const PI: f32 = 3.14159265358979323846;
const LANCZOS_A: f32 = 2.0;

fn sinc(x: f32) -> f32 {
    if x < 1e-4 {
        return 1.0;
    }
    let px = PI * x;
    return sin(px) / px;
}

fn lanczos_w(x: f32) -> f32 {
    let ax = abs(x);
    if ax >= LANCZOS_A {
        return 0.0;
    }
    return sinc(ax) * sinc(ax / LANCZOS_A);
}

fn tap(base: vec2<f32>, ox: f32, oy: f32) -> vec4<f32> {
    let uv = (base + vec2<f32>(ox + 0.5, oy + 0.5)) * inv_src_size();
    return textureSample(src_tex, src_samp, uv);
}

@fragment
fn fs_main(in: FsIn) -> @location(0) vec4<f32> {
    let pos = in.uv * src_size() - 0.5;
    let fxy = fract(pos);
    let base = pos - fxy;

    let wx0 = lanczos_w(-1.0 - fxy.x);
    let wx1 = lanczos_w( 0.0 - fxy.x);
    let wx2 = lanczos_w( 1.0 - fxy.x);
    let wx3 = lanczos_w( 2.0 - fxy.x);
    let wy0 = lanczos_w(-1.0 - fxy.y);
    let wy1 = lanczos_w( 0.0 - fxy.y);
    let wy2 = lanczos_w( 1.0 - fxy.y);
    let wy3 = lanczos_w( 2.0 - fxy.y);

    var acc = vec4<f32>(0.0);
    acc = acc + tap(base, -1.0, -1.0) * (wx0 * wy0);
    acc = acc + tap(base,  0.0, -1.0) * (wx1 * wy0);
    acc = acc + tap(base,  1.0, -1.0) * (wx2 * wy0);
    acc = acc + tap(base,  2.0, -1.0) * (wx3 * wy0);

    acc = acc + tap(base, -1.0,  0.0) * (wx0 * wy1);
    acc = acc + tap(base,  0.0,  0.0) * (wx1 * wy1);
    acc = acc + tap(base,  1.0,  0.0) * (wx2 * wy1);
    acc = acc + tap(base,  2.0,  0.0) * (wx3 * wy1);

    acc = acc + tap(base, -1.0,  1.0) * (wx0 * wy2);
    acc = acc + tap(base,  0.0,  1.0) * (wx1 * wy2);
    acc = acc + tap(base,  1.0,  1.0) * (wx2 * wy2);
    acc = acc + tap(base,  2.0,  1.0) * (wx3 * wy2);

    acc = acc + tap(base, -1.0,  2.0) * (wx0 * wy3);
    acc = acc + tap(base,  0.0,  2.0) * (wx1 * wy3);
    acc = acc + tap(base,  1.0,  2.0) * (wx2 * wy3);
    acc = acc + tap(base,  2.0,  2.0) * (wx3 * wy3);

    let wsum = (wx0 + wx1 + wx2 + wx3) * (wy0 + wy1 + wy2 + wy3);
    if wsum < 1e-6 {
        return vec4<f32>(0.0);
    }
    return (acc / wsum) * in.color;
}
