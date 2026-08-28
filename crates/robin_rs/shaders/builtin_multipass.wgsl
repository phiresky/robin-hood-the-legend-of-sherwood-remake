// Clean-room portable presentation shader suite.
//
// The edge scalers below are implementations from the published algorithmic
// descriptions (neighbour equality/colour distance and diagonal gradients),
// not translations of GPL/LGPL shader source. The Anime4K mode arrangement
// follows the upstream MIT-licensed v4 documentation (restore/soft-restore/
// denoise followed by upscale), while the compact kernels are purpose-built
// for Robin Hood's low-resolution painted sprites.

struct PassUniforms {
    // xy = source size, zw = reciprocal source size
    src: vec4<f32>,
    // xy = destination size, zw = reciprocal destination size
    dst: vec4<f32>,
    // x = strength, y = edge threshold, z = artifact removal, w = mode
    upscale: vec4<f32>,
    // x = scanlines, y = phosphor mask, z = bloom, w = curvature
    effect: vec4<f32>,
    // x = temporal flicker, y = presentation frame modulo 4096
    temporal: vec4<f32>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(1) @binding(0) var<uniform> params: PassUniforms;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var out: VsOut;
    out.clip_pos = vec4<f32>(positions[vertex], 0.0, 1.0);
    out.uv = uvs[vertex];
    return out;
}

fn clamped_uv(uv: vec2<f32>) -> vec2<f32> {
    return clamp(uv, params.src.zw * 0.5, vec2<f32>(1.0) - params.src.zw * 0.5);
}

fn sample_offset(uv: vec2<f32>, offset: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src_tex, src_samp, clamped_uv(uv + offset * params.src.zw), 0.0);
}

fn luma(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2627, 0.6780, 0.0593));
}

fn colour_distance(a: vec3<f32>, b: vec3<f32>) -> f32 {
    let delta = a - b;
    let y = dot(delta, vec3<f32>(0.2627, 0.6780, 0.0593));
    let co = delta.r - delta.b;
    let cg = delta.g - (delta.r + delta.b) * 0.5;
    return abs(y) * 0.55 + abs(co) * 0.25 + abs(cg) * 0.20;
}

fn similar(a: vec3<f32>, b: vec3<f32>) -> bool {
    return colour_distance(a, b) <= mix(0.015, 0.22, params.upscale.y);
}

fn source_pixel_uv(uv: vec2<f32>) -> vec2<f32> {
    return (floor(uv * params.src.xy) + vec2<f32>(0.5)) * params.src.zw;
}

@fragment
fn fs_copy(in: VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(src_tex, src_samp, clamped_uv(in.uv), 0.0);
}

@fragment
fn fs_nearest(in: VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(src_tex, src_samp, source_pixel_uv(in.uv), 0.0);
}

@fragment
fn fs_sharp_bilinear(in: VsOut) -> @location(0) vec4<f32> {
    let texel = in.uv * params.src.xy;
    let scale = max(ceil(params.dst.xy * params.src.zw), vec2<f32>(1.0));
    let base = floor(texel);
    let subpixel = fract(texel);
    let region = vec2<f32>(0.5) - vec2<f32>(0.5) / scale;
    let center = subpixel - vec2<f32>(0.5);
    let filtered = (center - clamp(center, -region, region)) * scale + vec2<f32>(0.5);
    return textureSampleLevel(src_tex, src_samp, clamped_uv((base + filtered) * params.src.zw), 0.0);
}

fn cubic_weights(value: f32) -> vec4<f32> {
    let n = vec4<f32>(1.0, 2.0, 3.0, 4.0) - value;
    let powers = n * n * n;
    let x = powers.x;
    let y = powers.y - 4.0 * powers.x;
    let z = powers.z - 4.0 * powers.y + 6.0 * powers.x;
    return vec4<f32>(x, y, z, 6.0 - x - y - z) / 6.0;
}

@fragment
fn fs_bicubic(in: VsOut) -> @location(0) vec4<f32> {
    let pos = in.uv * params.src.xy - vec2<f32>(0.5);
    let fractional = fract(pos);
    let integer = pos - fractional;
    let wx = cubic_weights(fractional.x);
    let wy = cubic_weights(fractional.y);
    let sums = vec4<f32>(wx.x + wx.y, wx.z + wx.w, wy.x + wy.y, wy.z + wy.w);
    let offset = vec4<f32>(
        (integer.x - 0.5 + wx.y / sums.x) * params.src.z,
        (integer.x + 1.5 + wx.w / sums.y) * params.src.z,
        (integer.y - 0.5 + wy.y / sums.z) * params.src.w,
        (integer.y + 1.5 + wy.w / sums.w) * params.src.w,
    );
    let a = textureSampleLevel(src_tex, src_samp, clamped_uv(offset.xz), 0.0);
    let b = textureSampleLevel(src_tex, src_samp, clamped_uv(offset.yz), 0.0);
    let c = textureSampleLevel(src_tex, src_samp, clamped_uv(offset.xw), 0.0);
    let d = textureSampleLevel(src_tex, src_samp, clamped_uv(offset.yw), 0.0);
    return mix(mix(d, c, sums.x / (sums.x + sums.y)),
               mix(b, a, sums.x / (sums.x + sums.y)),
               sums.z / (sums.z + sums.w));
}

fn sinc(value: f32) -> f32 {
    let absolute = abs(value);
    if (absolute < 0.0001) { return 1.0; }
    return sin(3.14159265 * absolute) / (3.14159265 * absolute);
}

fn lanczos_weight(value: f32) -> f32 {
    let absolute = abs(value);
    if (absolute >= 2.0) { return 0.0; }
    return sinc(absolute) * sinc(absolute * 0.5);
}

@fragment
fn fs_lanczos(in: VsOut) -> @location(0) vec4<f32> {
    let pos = in.uv * params.src.xy - vec2<f32>(0.5);
    let base = floor(pos);
    let fractional = pos - base;
    var total = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var y = -1; y <= 2; y = y + 1) {
        for (var x = -1; x <= 2; x = x + 1) {
            let weight = lanczos_weight(f32(x) - fractional.x)
                * lanczos_weight(f32(y) - fractional.y);
            let tap_uv = (base + vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) * params.src.zw;
            total += textureSampleLevel(src_tex, src_samp, clamped_uv(tap_uv), 0.0) * weight;
            weight_sum += weight;
        }
    }
    return total / max(weight_sum, 0.000001);
}

@fragment
fn fs_cut3(in: VsOut) -> @location(0) vec4<f32> {
    let pos = in.uv * params.src.xy - vec2<f32>(0.5);
    let base = floor(pos);
    let f = pos - base;
    let tl = textureSampleLevel(src_tex, src_samp, clamped_uv((base + vec2<f32>(0.5, 0.5)) * params.src.zw), 0.0);
    let tr = textureSampleLevel(src_tex, src_samp, clamped_uv((base + vec2<f32>(1.5, 0.5)) * params.src.zw), 0.0);
    let bl = textureSampleLevel(src_tex, src_samp, clamped_uv((base + vec2<f32>(0.5, 1.5)) * params.src.zw), 0.0);
    let br = textureSampleLevel(src_tex, src_samp, clamped_uv((base + vec2<f32>(1.5, 1.5)) * params.src.zw), 0.0);
    let main = tl * (1.0 - max(f.x, f.y)) + tr * max(f.x - f.y, 0.0)
        + bl * max(f.y - f.x, 0.0) + br * min(f.x, f.y);
    let anti = tl * max(1.0 - f.x - f.y, 0.0) + tr * min(f.x, 1.0 - f.y)
        + bl * min(f.y, 1.0 - f.x) + br * max(f.x + f.y - 1.0, 0.0);
    return select(anti, main, abs(luma(tl.rgb) - luma(br.rgb)) < abs(luma(tr.rgb) - luma(bl.rgb)));
}

// HQx-style colour-difference edge interpolation. The centre is kept exact
// away from corners, which is important for tiny UI-like world sprites.
@fragment
fn fs_hqx(in: VsOut) -> @location(0) vec4<f32> {
    let uv = source_pixel_uv(in.uv);
    let c = sample_offset(uv, vec2<f32>(0.0));
    let n = sample_offset(uv, vec2<f32>(0.0, -1.0));
    let s = sample_offset(uv, vec2<f32>(0.0,  1.0));
    let w = sample_offset(uv, vec2<f32>(-1.0, 0.0));
    let e = sample_offset(uv, vec2<f32>( 1.0, 0.0));
    let cell = fract(in.uv * params.src.xy);
    var candidate = c;
    var corner_weight = 0.0;
    if (cell.x < 0.5 && cell.y < 0.5 && similar(n.rgb, w.rgb) && !similar(c.rgb, n.rgb)) {
        candidate = (n + w) * 0.5;
        corner_weight = (0.5 - max(cell.x, cell.y)) * 2.0;
    } else if (cell.x >= 0.5 && cell.y < 0.5 && similar(n.rgb, e.rgb) && !similar(c.rgb, n.rgb)) {
        candidate = (n + e) * 0.5;
        corner_weight = (min(cell.x, 1.0 - cell.y) - 0.5) * 2.0;
    } else if (cell.x < 0.5 && cell.y >= 0.5 && similar(s.rgb, w.rgb) && !similar(c.rgb, s.rgb)) {
        candidate = (s + w) * 0.5;
        corner_weight = (min(1.0 - cell.x, cell.y) - 0.5) * 2.0;
    } else if (similar(s.rgb, e.rgb) && !similar(c.rgb, s.rgb)) {
        candidate = (s + e) * 0.5;
        corner_weight = (min(cell.x, cell.y) - 0.5) * 2.0;
    }
    let amount = clamp(corner_weight, 0.0, 1.0) * params.upscale.x * 0.75;
    return mix(c, candidate, amount);
}

// ScaleNX generalises the published Scale2x corner rule to the fractional
// output cell. It remains useful at arbitrary presentation ratios.
@fragment
fn fs_scalenx(in: VsOut) -> @location(0) vec4<f32> {
    let uv = source_pixel_uv(in.uv);
    let c = sample_offset(uv, vec2<f32>(0.0));
    let n = sample_offset(uv, vec2<f32>(0.0, -1.0));
    let s = sample_offset(uv, vec2<f32>(0.0,  1.0));
    let w = sample_offset(uv, vec2<f32>(-1.0, 0.0));
    let e = sample_offset(uv, vec2<f32>( 1.0, 0.0));
    let cell = fract(in.uv * params.src.xy);
    var replacement = c;
    if (!similar(n.rgb, s.rgb) && !similar(w.rgb, e.rgb)) {
        if (cell.x < 0.5 && cell.y < 0.5 && similar(w.rgb, n.rgb)) {
            replacement = (w + n) * 0.5;
        } else if (cell.x >= 0.5 && cell.y < 0.5 && similar(n.rgb, e.rgb)) {
            replacement = (n + e) * 0.5;
        } else if (cell.x < 0.5 && cell.y >= 0.5 && similar(w.rgb, s.rgb)) {
            replacement = (w + s) * 0.5;
        } else if (cell.x >= 0.5 && cell.y >= 0.5 && similar(s.rgb, e.rgb)) {
            replacement = (s + e) * 0.5;
        }
    }
    return mix(c, replacement, params.upscale.x);
}

// Free-scale xBRZ-style diagonal analysis. A 3x3 gradient chooses the less
// discontinuous diagonal; blending is constrained to the source-pixel corner.
@fragment
fn fs_xbrz(in: VsOut) -> @location(0) vec4<f32> {
    let uv = source_pixel_uv(in.uv);
    let c = sample_offset(uv, vec2<f32>(0.0));
    let nw = sample_offset(uv, vec2<f32>(-1.0, -1.0));
    let n  = sample_offset(uv, vec2<f32>( 0.0, -1.0));
    let ne = sample_offset(uv, vec2<f32>( 1.0, -1.0));
    let w  = sample_offset(uv, vec2<f32>(-1.0,  0.0));
    let e  = sample_offset(uv, vec2<f32>( 1.0,  0.0));
    let sw = sample_offset(uv, vec2<f32>(-1.0,  1.0));
    let s  = sample_offset(uv, vec2<f32>( 0.0,  1.0));
    let se = sample_offset(uv, vec2<f32>( 1.0,  1.0));
    let slash = colour_distance(nw.rgb, c.rgb) + colour_distance(c.rgb, se.rgb)
        + colour_distance(n.rgb, e.rgb) + colour_distance(w.rgb, s.rgb);
    let backslash = colour_distance(ne.rgb, c.rgb) + colour_distance(c.rgb, sw.rgb)
        + colour_distance(n.rgb, w.rgb) + colour_distance(e.rgb, s.rgb);
    let cell = fract(in.uv * params.src.xy) - vec2<f32>(0.5);
    let edge = smoothstep(0.05, 0.45, abs(abs(cell.x) - abs(cell.y)));
    var neighbour = c;
    if (slash < backslash) {
        neighbour = select((w + s) * 0.5, (n + e) * 0.5, cell.x > cell.y);
    } else {
        neighbour = select((n + w) * 0.5, (e + s) * 0.5, cell.x + cell.y > 0.0);
    }
    let confidence = smoothstep(0.0, 0.35, abs(slash - backslash));
    return mix(c, neighbour, edge * confidence * params.upscale.x * 0.62);
}

fn box_blur(uv: vec2<f32>) -> vec4<f32> {
    let n = sample_offset(uv, vec2<f32>(0.0, -1.0));
    let s = sample_offset(uv, vec2<f32>(0.0,  1.0));
    let w = sample_offset(uv, vec2<f32>(-1.0, 0.0));
    let e = sample_offset(uv, vec2<f32>( 1.0, 0.0));
    return (n + s + w + e) * 0.25;
}

@fragment
fn fs_anime_restore(in: VsOut) -> @location(0) vec4<f32> {
    let c = sample_offset(in.uv, vec2<f32>(0.0));
    let blur = box_blur(in.uv);
    let detail = c - blur;
    let gate = smoothstep(params.upscale.y * 0.15, 0.35, abs(luma(detail.rgb)));
    return vec4<f32>(clamp(c.rgb + detail.rgb * gate * params.upscale.x * 1.35, 0.0, 1.0), c.a);
}

@fragment
fn fs_anime_restore_soft(in: VsOut) -> @location(0) vec4<f32> {
    let c = sample_offset(in.uv, vec2<f32>(0.0));
    let blur = box_blur(in.uv);
    let softened = mix(c, blur, 0.16 + params.upscale.z * 0.12);
    let detail = c - blur;
    return vec4<f32>(clamp(softened.rgb + detail.rgb * params.upscale.x * 0.65, 0.0, 1.0), c.a);
}

@fragment
fn fs_anime_denoise(in: VsOut) -> @location(0) vec4<f32> {
    let c = sample_offset(in.uv, vec2<f32>(0.0));
    let n = sample_offset(in.uv, vec2<f32>(0.0, -1.0));
    let s = sample_offset(in.uv, vec2<f32>(0.0, 1.0));
    let w = sample_offset(in.uv, vec2<f32>(-1.0, 0.0));
    let e = sample_offset(in.uv, vec2<f32>(1.0, 0.0));
    var sum = c;
    var weight = 1.0;
    let threshold = 0.035 + params.upscale.y * 0.20;
    if (colour_distance(c.rgb, n.rgb) < threshold) { sum += n; weight += 1.0; }
    if (colour_distance(c.rgb, s.rgb) < threshold) { sum += s; weight += 1.0; }
    if (colour_distance(c.rgb, w.rgb) < threshold) { sum += w; weight += 1.0; }
    if (colour_distance(c.rgb, e.rgb) < threshold) { sum += e; weight += 1.0; }
    return mix(c, sum / weight, 0.55 * params.upscale.x);
}

@fragment
fn fs_anime_upscale(in: VsOut) -> @location(0) vec4<f32> {
    let linear = textureSampleLevel(src_tex, src_samp, clamped_uv(in.uv), 0.0);
    let px = source_pixel_uv(in.uv);
    let n = sample_offset(px, vec2<f32>(0.0, -1.0));
    let s = sample_offset(px, vec2<f32>(0.0, 1.0));
    let w = sample_offset(px, vec2<f32>(-1.0, 0.0));
    let e = sample_offset(px, vec2<f32>(1.0, 0.0));
    let horizontal = colour_distance(w.rgb, e.rgb);
    let vertical = colour_distance(n.rgb, s.rgb);
    let directed = select((w + e) * 0.5, (n + s) * 0.5, horizontal > vertical);
    let confidence = smoothstep(0.02, 0.35, abs(horizontal - vertical));
    return mix(linear, directed, confidence * params.upscale.x * 0.28);
}

// Ringing/artifact removal clamps a mild unsharp result to its immediate
// neighbourhood. It doubles as super-xBR's final anti-ringing pass.
@fragment
fn fs_artifact_remove(in: VsOut) -> @location(0) vec4<f32> {
    let c = sample_offset(in.uv, vec2<f32>(0.0));
    let n = sample_offset(in.uv, vec2<f32>(0.0, -1.0));
    let s = sample_offset(in.uv, vec2<f32>(0.0, 1.0));
    let w = sample_offset(in.uv, vec2<f32>(-1.0, 0.0));
    let e = sample_offset(in.uv, vec2<f32>(1.0, 0.0));
    let lo = min(c.rgb, min(min(n.rgb, s.rgb), min(w.rgb, e.rgb)));
    let hi = max(c.rgb, max(max(n.rgb, s.rgb), max(w.rgb, e.rgb)));
    let blur = (n.rgb + s.rgb + w.rgb + e.rgb) * 0.25;
    let sharpened = clamp(c.rgb + (c.rgb - blur) * params.upscale.x * 0.32, lo, hi);
    return vec4<f32>(mix(c.rgb, sharpened, params.upscale.z), c.a);
}

@fragment
fn fs_super_finish(in: VsOut) -> @location(0) vec4<f32> {
    let c = sample_offset(in.uv, vec2<f32>(0.0));
    let nw = sample_offset(in.uv, vec2<f32>(-1.0, -1.0));
    let ne = sample_offset(in.uv, vec2<f32>(1.0, -1.0));
    let sw = sample_offset(in.uv, vec2<f32>(-1.0, 1.0));
    let se = sample_offset(in.uv, vec2<f32>(1.0, 1.0));
    let diagonal = (nw + ne + sw + se) * 0.25;
    return vec4<f32>(clamp(c.rgb + (c.rgb - diagonal.rgb) * params.upscale.x * 0.18, 0.0, 1.0), c.a);
}

fn curved_uv(uv: vec2<f32>) -> vec2<f32> {
    let p = uv * 2.0 - vec2<f32>(1.0);
    let bend = params.effect.w * 0.11;
    let warped = p * (vec2<f32>(1.0) + bend * vec2<f32>(p.y * p.y, p.x * p.x));
    return warped * 0.5 + vec2<f32>(0.5);
}

fn crt_base(uv: vec2<f32>, royale: bool) -> vec4<f32> {
    let warped = curved_uv(uv);
    if (any(warped < vec2<f32>(0.0)) || any(warped > vec2<f32>(1.0))) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let c = textureSampleLevel(src_tex, src_samp, warped, 0.0);
    let dx = params.src.zw * vec2<f32>(select(1.5, 2.5, royale), 0.0);
    let glow = (textureSampleLevel(src_tex, src_samp, clamped_uv(warped - dx), 0.0)
        + textureSampleLevel(src_tex, src_samp, clamped_uv(warped + dx), 0.0)) * 0.5;
    var rgb = mix(c.rgb, glow.rgb, params.effect.z * select(0.10, 0.24, royale));
    let output_y = warped.y * params.dst.y;
    let scan = 1.0 - params.effect.x * select(0.28, 0.38, royale)
        * (0.5 + 0.5 * cos(output_y * 3.14159265));
    rgb *= scan;
    let output_x = u32(max(warped.x * params.dst.x, 0.0));
    let phase = output_x % 3u;
    let mask_dark = 1.0 - params.effect.y * select(0.20, 0.32, royale);
    var mask = vec3<f32>(mask_dark);
    if (phase == 0u) { mask.r = 1.0; }
    if (phase == 1u) { mask.g = 1.0; }
    if (phase == 2u) { mask.b = 1.0; }
    if (royale && (u32(max(warped.y * params.dst.y, 0.0)) % 2u == 1u)) {
        mask = mask.brg;
    }
    rgb *= mask;
    let flicker = 1.0 - params.temporal.x * 0.025
        * (0.5 + 0.5 * sin(params.temporal.y * 2.39996323));
    return vec4<f32>(clamp(rgb * flicker, 0.0, 1.0), c.a);
}

@fragment
fn fs_crt_guest(in: VsOut) -> @location(0) vec4<f32> {
    return crt_base(in.uv, false);
}

@fragment
fn fs_crt_royale(in: VsOut) -> @location(0) vec4<f32> {
    return crt_base(in.uv, true);
}
