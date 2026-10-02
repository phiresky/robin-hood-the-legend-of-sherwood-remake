# Browser BC7f measurements

This harness decodes the editor's existing AVIF images, reads unpremultiplied
pixels through WebGL, encodes BC7 plus a complete mip chain in a WASM worker,
uploads every mip to WebGL, and compares base-level GPU-decoded pixels with the
input. No WebGPU is used. It records stage timings, encoder linear-memory size,
browser-process RSS samples, GPU identity, and representative comparison crops.

## Reproduce

Requirements: Node with global WebSocket, Chromium, Python/Pillow, Emscripten,
and `wasm-strip` (WABT). From the repository root:

```sh
mkdir -p level-editor/work/bc7f-benchmark
git clone https://github.com/BinomialLLC/basis_universal.git level-editor/work/bc7f-benchmark/basis_universal
git -C level-editor/work/bc7f-benchmark/basis_universal checkout 99f52d63aa6799cbdaecfe977111dc5ec3b31d47
python3 level-editor/benchmarks/bc7f/prepare.py
bash level-editor/benchmarks/bc7f/build.sh
cp level-editor/work/bc7f-benchmark/bc7f.js level-editor/work/bc7f-benchmark/bc7f-scalar.js
cp level-editor/work/bc7f-benchmark/bc7f.wasm level-editor/work/bc7f-benchmark/bc7f-scalar.wasm
BC7F_SIMD=1 bash level-editor/benchmarks/bc7f/build.sh
BENCH_GPU=hardware BENCH_BUILD=scalar BENCH_VARIANT=scalar-hardware node level-editor/benchmarks/bc7f/run.mjs
BENCH_GPU=hardware BENCH_VARIANT=simd-hardware node level-editor/benchmarks/bc7f/run.mjs
python3 level-editor/benchmarks/bc7f/prepare.py york
BENCH_GPU=hardware BENCH_MAP=york BENCH_VARIANT=york-hardware node level-editor/benchmarks/bc7f/run.mjs
```

Set `EMXX` to select a compatible Emscripten compiler. The recorded build used
Emscripten 4.0.22, LLVM 22, Binaryen 132. Hardware mode disables software fallback
and rejects a reported software GPU. Omitting `BENCH_GPU` selects SwiftShader.
Each run uses a fresh Chromium profile and local HTTP server, and closes both.
Artifacts and extracted images stay in the ignored `work/bc7f-benchmark` directory.

## Recorded results

`results.json` contains timings from Chromium 153 on a Ryzen 7 7840U. Hardware
runs used Radeon 780M, Mesa 26.2.3, ANGLE/OpenGL ES. These are small experiments
(one first pass and one repeat per variant), not statistically robust benchmarks.

Representative repeat timings on hardware, in milliseconds:

| Atlas | Scalar encode | SIMD encode | SIMD mip generation | Full SIMD texture path |
| --- | ---: | ---: | ---: | ---: |
| York timber house, 688² | 15.0 | 13.5 | 4.6 | 33.6 |
| York terrain, 3136×2318 | 349.1 | 326.3 | 70.6 | 607.2 |
| Watermill, 4096² | 235.6 | 209.1 | 150.0 | 732.1 |
| Cottage tree, 4096² | 246.8 | 225.4 | 139.1 | 714.1 |

Full path includes decode, pixel extraction, alignment, worker execution, result
transfer and upload; it excludes HTTP fetch, quality comparison and crop export.
SIMD reduced aggregate encoding time about 9% in the matched hardware repeats
(about 19% in the earlier SwiftShader comparison). Base-level quality metrics and
comparison crops were identical between scalar/SIMD and hardware/software runs.

The sequential York batch processed 249 textures / 136.4 million base pixels in
11.4 seconds, including localhost requests, logging and a warm-up image. Of that,
3.70 seconds was BC7 encoding and 1.34 seconds mip generation. AVIF delivery was
23.4 MB; BC7 mip payloads were 182.0 MB. GPU RGBA8 equivalents are roughly four
times larger. The actual editor overlaps two encoders with eight asset-loading
tasks, and conservatively skips alpha-sensitive, unaligned and small images.
Initial integrated York loads were 6.18 seconds without encoding and 7.75 seconds
with encoding in otherwise matching fresh hardware-browser runs.

The matching Wychford editor runs took 8.43 seconds without encoding and 14.78
seconds with it. Of 148 textures, 91 were compressed. Estimated GPU texture
payload fell from 3.31 GB to 1.15 GB (720 MB BC7 plus 430 MB unchanged textures).
The GPU driver's resident GTT + VRAM counters fell from 2.62 GiB to 1.23 GiB.
Main renderer RSS rose from 0.69 GiB to 1.32 GiB: compressed mip buffers remain
on the CPU for context restoration and reupload. These counters overlap on this
integrated GPU and must not be summed as unique process memory. Measurements
were taken immediately after load, before forced garbage collection. The JS heap
itself stayed around 124 MB; its backing storage increased from 151 MB to 871 MB.

The editor runs used fresh profiles and a local server, with the small codec
regression run before timing. They are not cold-network loading predictions.
`integratedEditor` in the results file records these measurements; the initial
York encoded texture counter excluded the ground and is explicitly marked.

## Quality and memory limits

Visible-pixel RGB PSNR ranged from 33 to 52 dB across the samples. Black atlas
padding inflates whole-image PSNR. The large foliage sample had 9,339 base-level
alpha-cutoff changes (0.056%); this is why the editor leaves opacity and ownership
alpha textures uncompressed. Crops show input, BC7 output, and 8× RGB difference.
Mipmaps use a linear-light area box filter, with independent alpha. They do not
preserve alpha coverage; the production path only accepts fully opaque pixels.

The test pads non-multiple-of-four dimensions to satisfy WebGL BC7 restrictions,
but production skips those images to preserve UV mapping. Base-level comparison
does not prove full-scene or all-mip visual equivalence.

One focused encoder worker grew to 120.75 MiB of WASM linear memory on the large
samples, versus the general encoder's 128 MiB initial allocation. WASM pages are
released by terminating the batch workers. The comparison harness peaked around
1.0–1.1 GiB renderer RSS because it also holds originals, encoded outputs, decoded
comparison buffers and temporary render targets. These are diagnostic workload
peaks, not the editor's steady-state memory. GPU process RSS excludes some GPU
allocations on hardware; process RSS values must not be summed as unique memory.

Basis's existing WASM `encodeToDDS()` API already exposes BC7f, but its shipped
32-bit build caps inputs at 12 megapixels and rejects 4096² images. The retained
`upstream-worker.js` illustrates that API; the focused adapter avoids this cap
and packages only the encoder needed here (about 215 KB WASM, 93 KB gzipped).

## Integration validation

Typechecking, lint, production build and 20 focused asset/material tests passed.
The production browser fixture with `?resources-only` passed the codec tests and
34 map loads across four mounts, including stale loads, replacement, undo/redo,
save-during-edit and unmount-during-load. Final tracked GPU resources, listeners,
observers and animation frames were zero. The default full fixture currently
fails its interactive river assertion (`River control points were not saved`);
that fixture remains enabled in the default run.
