# Rendering notes

Design decisions and measurements for the wgpu renderer. Newest entry
first.

---

## 2026-08-29 — Sprite atlas, stage 1

### Where sprites were

Compositing was already on the GPU, but every decoded sprite frame owned
a *whole texture*:

- `GpuResources::ensure_sprite_cached` decompressed one frame
  (`FrameHolder::uncompress_frame`), converted RGB565 → RGBA8
  (`rgb565_to_rgba_with_key`), then created a `wgpu::Texture`, a
  `TextureView` and a `BindGroup` for that one sprite
  (`upload_rgba_texture` + `make_tex_bg`).
- The cache key was `(bank_id, variant, shadow_color, shadow_alpha)`,
  held in `SpriteTextureCache`, and was **never cleared** — decoded RGBA
  for every sprite ever drawn stayed resident for the life of the
  process.
- At draw time `render_cached_sprite` cloned that sprite's bind group
  into the per-frame `frame_texture_bgs` list, always as a *fresh*
  index. Because `encode_pass1_range_to_rt` elides a texture rebind only
  when consecutive draws carry the same `TextureRef::Frame(idx)`, a
  fresh index per draw meant **one `set_bind_group` and one `draw` per
  sprite**, every frame.

### What changed

`crates/robin_rs/src/renderer/atlas.rs` packs decoded frames into a
small number of 2048² `Rgba8UnormSrgb` layers with a shelf packer. A
sprite becomes a sub-rect: `SpriteTextureCache` now maps its key to an
`AtlasSlot { layer, uv, width, height }` rather than to a texture.

Three things follow:

1. **No per-sprite texture/view/bind-group.** Mission load stops
   allocating one GPU texture per sprite frame.
2. **Real bind batching.** `Renderer::queue_atlas_layer` memoizes
   layer → per-frame bind-group index for the frame, so every sprite
   drawn from a layer resolves to *one* index and the existing rebind
   elision finally fires.
3. **Draw-call merging.** `encode_pass1_range_to_rt` now accumulates
   consecutive draws that need no state change into a single
   `pass.draw` over a contiguous vertex range. Quads are laid out in
   queue order at `i * 6`, so a run is always contiguous; the run is
   flushed before every pipeline / bind-group / stencil change and
   before every skipped draw.

The `QueuedDraw` struct already carried a `uv: [f32; 4]` sub-rect (it
was simply always `[0,0,1,1]` for sprites), so the draw path itself
needed no structural change.

### Why this is pixel-identical

The atlas is deliberately *only* a packing change:

- The bytes written into a layer are byte-for-byte the RGBA that
  `sprite_rgba_for_upload` produced for the per-sprite texture. The
  atlas performs **no colour conversion of its own** — no shadow
  handling, no format change. Same `Rgba8UnormSrgb`, same NEAREST
  sampler, same `fs_main` (`textureSample(...) * tint`), same blend
  pipelines.
- UVs are an exact remap. For a `w`-wide sprite at integral layer
  origin `x`, the old path sampled texel `floor(u * w)` and the new one
  samples `floor(x + u * w) - x`. Fragment sampling points sit at texel
  centres — half a texel from any boundary — so float error at layer
  scale (~1e-4) cannot move them across a boundary.
- Every sub-rect carries a 1-texel transparent gutter. NEAREST sampling
  of an exact sub-rect never reads it; it exists so that no sampling
  mistake can bleed a *neighbouring sprite* in. It fails to transparent
  instead.
- Draw merging changes only how many `draw` calls record the same
  vertices under the same state.

`ROBIN_SPRITE_ATLAS=0` restores the per-sprite texture path. This is
temporary A/B scaffolding: it lets **one binary** render both halves of
a comparison with data, build, driver and scene held fixed, which is a
stronger test than diffing two builds. It goes away with the legacy
path.

### Instrumentation

The `fps` tracing target now reports `binds/f` alongside `draws/f`, plus
an atlas summary (`layers / MiB / occupancy / sprites`):

```
RUST_LOG=fps=debug cargo run --bin robin -- …
… fps  draws/f=…  binds/f=…  uploads/f=…  present=…ms  atlas=3L/48MiB/71%occ/1842spr …
```

`binds/f` is the number the atlas is meant to move: before it, that
figure tracked `draws/f` almost exactly.

### Memory

Worth stating plainly, because "atlases hold decoded pixels" sounds like
a regression and is not one here: the pre-atlas cache *already* held
decoded RGBA8 for every sprite ever drawn, in one never-evicted texture
each. The atlas holds the same pixels with fewer, larger allocations —
no per-texture size rounding, no per-texture view/bind-group — at the
cost of reserving whole layers, so the final partly-filled layer is the
only new waste. `AtlasStats::occupancy` reports how much of the reserved
area actually carries sprite pixels.

### Validation

`render_mission_map` is a deterministic one-shot capture (mission
`Initialize`, no tick, then screenshot and exit), which makes it the
right A/B instrument. The box has no display server, so captures run
under `xvfb-run` — the example still needs a GPU-backed window even in
its hidden `--headless` mode.

```sh
ROBIN_SPRITE_ATLAS=0 xvfb-run -a target/debug/examples/render_mission_map \
    Dem_Lei_MP --frame 0 --headless --data-dir datadirs/demo_leicester_ecoste -o legacy.png
ROBIN_SPRITE_ATLAS=1 xvfb-run -a target/debug/examples/render_mission_map \
    Dem_Lei_MP --frame 0 --headless --data-dir datadirs/demo_leicester_ecoste -o atlas.png
compare -metric AE legacy.png atlas.png null:     # expect 0
```

### Measurements

<!-- filled in once the A/B and FPS runs complete -->

### Still to do

- **Shadow in the shader.** `decompress_rle_arno_law` rewrites
  `SHADOW_KEY` to the ambience `shadow_color`, and
  `rgb565_to_rgba_with_key` then maps both to black at `shadow_alpha`
  — so shadow is *already* just "black at alpha", and `shadow_color` is
  effectively a marker. Moving the classification into the fragment
  shader collapses `shadow_color`/`shadow_alpha` out of the cache key
  and makes ambience changes free. Two collision cases must be measured
  before this can claim exactness: a source pixel equal to
  `shadow_color` (arno_law bumps it by +1, an ambience-dependent colour
  change) and a fog-blended pixel that *lands* on `shadow_color` (and is
  then classified as shadow). Both are rare; neither may be assumed
  absent without counting them over real data.
- **Hit testing.** `rle_pixel_at` is O(rows) per query — every mouse-over
  rewalks the sprite from row 0. `crate::ui::AlphaMask` is already the
  1-bit-per-pixel structure needed, used today only for UI widgets.
  **It takes two bits per pixel, not one**: `is_pixel_opaque` has two
  modes, and blipped entities pass `blue_pixels_are_in = true`, which
  makes shadow pixels hittable. So the mask needs two planes — "not
  transparent" and "not transparent and not shadow" — i.e. the same
  three-class encoding the atlas alpha wants. That is 1/8 the memory of
  the packed `u16` pixels and O(1) per query, not the 1/16 a single
  plane would give.

  Both planes are ambience-independent and can be built once at load:
  the RLE path tests raw `SHADOW_KEY` in the packed data, and the VQ
  path tests the dictionary's *own* `shadow_color()` — deliberately not
  the `Weather` colour, so a partially-published ambiance generation
  cannot turn shadow pixels opaque. Any change here must keep that
  property and be gated on an exhaustive equality check against the
  current `is_pixel_opaque` over every sprite × every pixel × both
  modes.
- **Deletions** once the above land: the `ROBIN_SPRITE_ATLAS` flag and
  `SpriteResidency::Legacy`; `uncompress_frame_wipe_shadow` /
  `uncompress_frame_into_shadow` and their `decompress_rle_*` helpers,
  which already have **no production callers** (tests only).
