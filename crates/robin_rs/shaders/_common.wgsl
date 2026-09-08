// Shared vertex-stage inputs and resource bindings for the upscale
// fragment shaders.  Prepended by build.rs to every other `.wgsl`
// before naga compiles it — WGSL has no `#include`.
//
// The layout matches the vertex output produced by `gpu_upscale.rs`:
//
//   PSInput {
//     v_color : COLOR0       →  @location(0) vec4<f32>
//     v_uv    : TEXCOORD0    →  @location(1) vec2<f32>
//   };
//
// The upscale pipeline reserves groups 0 and 1 for the renderer's standard
// layouts. Group 2 holds the source texture and sampler; group 3 holds the
// frame uniforms.

struct FsIn {
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

// `vec2<f32>` in `var<uniform>` has a *16-byte* minimum alignment —
// WGSL rounds non-scalar uniform-struct members up to the host-shareable
// boundary.  Packing pairs into a single `vec4<f32>` sidesteps the rule
// (vec4 is naturally 16-byte aligned), matches `[f32; 4]` in Rust, and
// keeps the total block at 32 bytes.
struct FrameUniforms {
    src: vec4<f32>, // xy = src_size, zw = 1 / src_size
    dst: vec4<f32>, // xy = dst_size, zw = 1 / dst_size
};

@group(3) @binding(0) var<uniform> frame: FrameUniforms;

fn src_size() -> vec2<f32> { return frame.src.xy; }
fn inv_src_size() -> vec2<f32> { return frame.src.zw; }
fn dst_size() -> vec2<f32> { return frame.dst.xy; }
fn inv_dst_size() -> vec2<f32> { return frame.dst.zw; }
@group(2) @binding(0) var src_tex: texture_2d<f32>;
@group(2) @binding(1) var src_samp: sampler;
