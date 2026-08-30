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

While the migration was in flight, `ROBIN_SPRITE_ATLAS=0` restored the
per-sprite texture path, so **one binary** could render both halves of a
comparison with data, build, driver and scene held fixed — a stronger
test than diffing two builds. That scaffolding has been removed now the
comparison is signed off; the commands below are kept as the record of
how the evidence was produced, and no longer run as written.

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
`Initialize`, `--frame N` normal game frames, then screenshot and
exit), which makes it the right A/B instrument. It is also the only
reliable way to render a scene headlessly here: an interactive
`--mission` run stops on the opening modal dialogue, which suspends the
tick — the game draws ~18 HUD quads and caches no sprites at all, so
any measurement taken that way is measuring nothing.

The box has no display server, so captures run under `xvfb-run`; the
example still needs a GPU-backed window even in its hidden
`--headless` mode, and rasterises through lavapipe. Packages needed on
a bare box: `xvfb`, `xauth`, `libxkbcommon-x11-0`.

Note that `capture_frame_rgba` encodes its own pass rather than going
through `present`, so it does **not** reach `log_fps`; the counters for
a captured scene come from the `capture …` line that readback logs.

```sh
# How the identity evidence was produced, while the flag still existed.
ROBIN_SPRITE_ATLAS=0 xvfb-run -a target/debug/examples/render_mission_map \
    Dem_Lei_MP --frame 0 --headless --data-dir datadirs/demo_leicester_ecoste -o legacy.png
ROBIN_SPRITE_ATLAS=1 xvfb-run -a target/debug/examples/render_mission_map \
    Dem_Lei_MP --frame 0 --headless --data-dir datadirs/demo_leicester_ecoste -o atlas.png
compare -metric AE legacy.png atlas.png null:     # expect 0
```

### Measurements

**Pixel identity.** Eight full-map captures, three missions, two
datadirs, frames 0 / 25 / 300 / 600. In every case the legacy and atlas
PNGs are **byte-identical** (same SHA-256, same file size) and
`compare -metric AE` reports **0 differing pixels**. These are not
small images: `Dem_Lei_MP` is 3136×2064 with 13 260 distinct colours.

| mission | datadir | frame | capture | differing px |
|---|---|---|---|---|
| `Dem_Lei_MP` | demo_leicester_ecoste | 0 | 3136×1984 | 0 |
| `Dem_Lei_MP` | demo_leicester_ecoste | 25 | 3136×1984 | 0 |
| `Dem_Lei_MP` | demo_leicester_ecoste | 600 | 3136×2064 | 0 |
| `S02_Lei_MP` | fullgame_linux | 0 | — | 0 |
| `H01_Lin_VL` | fullgame_linux | 0 | 2944×2256 | 0 |
| `H01_Lin_VL` | fullgame_linux | 300 | 2944×2256 | 0 |

**Draw calls and binds.** Counted at the scene capture, which for the
full-map exporter *is* the whole scene. `quads` is the queue length;
`drawcalls` is actual `pass.draw` calls after coalescing; `binds` is
`set_bind_group(1, …)` calls.

| scene | quads | drawcalls | binds | Δ |
|---|---|---|---|---|
| `Dem_Lei_MP` f0 legacy | 217 | 217 | 217 | |
| `Dem_Lei_MP` f0 atlas | 217 | **161** | **161** | −26% |
| `Dem_Lei_MP` f600 legacy | 208 | 208 | 208 | |
| `Dem_Lei_MP` f600 atlas | 208 | **153** | **153** | −26% |
| `H01_Lin_VL` f300 legacy | 149 | 149 | 149 | |
| `H01_Lin_VL` f300 atlas | 149 | **99** | **99** | −34% |

Legacy `binds == quads` exactly, which is the predicted pathology:
every sprite owned a texture, so every sprite forced a rebind. The
residual atlas binds are the non-sprite draws (background, masks, HUD,
stencil) that legitimately switch texture.

**Layer size.** Layers are reserved whole, so their size is a
memory-vs-binds dial. Measured on `Dem_Lei_MP` at frame 600 (106
sprites cached):

| policy | layers | reserved | occupancy | binds |
|---|---|---|---|---|
| fixed 2048² | 1 | 16.0 MiB | 13% | 157 |
| doubling 512²→2048² | 3 | 21.0 MiB | 20% | 157 |
| **uniform 1024²** | 2 | **8.0 MiB** | **53%** | **153** |

Uniform 1024² wins on all three axes and is what shipped. Doubling
loses because spilling into the next size step over-reserves badly.

**Memory, honestly.** For `Dem_Lei_MP` at frame 600 the atlas reserves
8.0 MiB to hold ~4.2 MiB of actual sprite pixels. The pre-atlas cache
allocated exactly `w×h×4` per sprite, so it held those ~4.2 MiB plus
per-texture overhead — meaning the atlas costs roughly **2× the pixel
bytes** at 53% occupancy. That is the real trade, and it is worth
stating rather than claiming a memory win: what the atlas removes is
one `wgpu::Texture` + `TextureView` + `BindGroup` object per sprite
(86–106 of them here, and far more over a long session), not bytes.
Neither cache evicts today; if a sprite-heavy mission makes the
reservation matter, the fix is per-group paging, and
`AtlasStats::occupancy` is the number to watch.

**Simulation is untouched.** Two independent checks:

- Replaying the same recording under both paths produces the *same*
  engine state hash at frame 0 — `c7de02c707bd0b7d` in both arms. (That
  recording desyncs against its own stored `517c97920814a7c2` because
  it was made against a different build/datadir; the point here is that
  the two arms agree with each other exactly.)
- The frame-600 captures are byte-identical, which means 600 simulated
  frames put every entity on exactly the same pixel. Any RNG or state
  divergence would have moved something.

**Wall time.** The full-map export is dominated by mission load and
software (lavapipe) rasterisation, so it is not a clean frame-rate
measurement: `Dem_Lei_MP` frame 600 took 155 s legacy vs 151 s atlas,
`H01_Lin_VL` frame 0 15.1 s vs 14.5 s. Directionally favourable,
within noise, and not offered as an FPS result — a real FPS comparison
needs a release build on real hardware, which this headless box cannot
provide.

**In the browser.** The threaded wasm build
(`scripts/build-wasm-threads.sh`, then
`node scripts/wasm_mission_install_chrome.mjs <converted-datadir>
--wait-ingame`) installs `H01_Lin_VL` 7.6 s after `wasm_boot` and
reaches in-game at 8.4 s, with no atlas diagnostics and no panics. The
three `ListDefault/ListFocused/ListSelected.tfn` font errors in that log
predate this work and are unrelated to sprites.

### The memory trade, stated plainly

The atlas reserves whole layers, so it holds *more* bytes than the
per-sprite path it replaced, which allocated exactly `w × h × 4`. Two
different numbers matter here and conflating them flatters the result:

| scene | layers | reserved | occupancy | packing efficiency |
|---|---|---|---|---|
| `Dem_Lei_MP` f600 | 2 | 8 MiB | 53% | 64% |
| `H01_Lin_VL` f300 | 2 | 8 MiB | 34% | 50% |

*Occupancy* is sprite pixels over all reserved texels; it counts the
newest layer's unused tail, which is simply capacity the next sprites
will take. *Packing efficiency* (`AtlasStats::packing_efficiency`) is
sprite pixels over what the packer actually committed to shelves, and is
the number to judge the packer by. Roughly 10 points of the shortfall is
the mandatory 1-texel gutter (a 31×49 sprite pays 9.7%); the rest is
shelf-height slack from packing in draw order rather than sorted by
height.

So the honest trade is ~4 MiB of texture memory for a 26–34% cut in
bind calls and the removal of one texture, view and bind-group object
per sprite frame. Worth it on desktop; the occupancy and packing dials
exist if a memory-constrained target ever disagrees.

An attempt to close the gap **failed and was reverted**: opening a fresh
right-height shelf whenever best-fit would waste more than a quarter of
the sprite's height. It sounds better and measures worse — eagerly
starting shelves burns the layer's vertical budget, taking the demo
scene from 2 layers/8 MiB/64% packed to 3 layers/12 MiB/51%, and adding
a bind. Filling an imperfect shelf beats reserving a perfect one. A real
improvement would need to defer packing until sprite sizes are known,
which conflicts with decoding sprites on first draw.

### Closed questions

- **Shadow stays on the CPU.** Moving the shadow resolve into the
  fragment shader was investigated and rejected: it buys nothing
  measurable. Ambience is set once per mission
  (`level_loading_host::initialize_sprite_variants`, called only from
  `game_session/setup.rs`), so `shadow_color` / `shadow_alpha` never
  change during play and no sprite is ever re-baked for them. And the
  cache carries no duplicates to collapse — the capture line reports
  `cache=106e/106f` and `90e/90f`, i.e. entries exactly equal distinct
  `(bank_id, variant)` frames.

  The collision counting done first is recorded in the probe
  (`sprite_compression_probe --shadow-collisions day|fog|night`). Over
  the fullgame bank's 2,015,716 dictionary pixel slots: day (`0x2964`)
  has 299 collisions across 61 of 134 dictionaries, fog and night have
  none. All 299 are defused — after `apply_arno_law` exactly 212,633
  slots equal the night colour in every ambiance, precisely the genuine
  `SHADOW_KEY` population, so no art pixel is left indistinguishable
  from shadow. That works because the load order runs the law *after*
  the fog/night blend, so its `+1` bump covers blend results too. The
  one residual hazard is a night colour of `0x001E`, where the bump
  lands on `SHADOW_KEY` and is remapped straight back; none of the three
  shipping ambiances is that, and the probe flags it if one ever is.

- **No hit-test masks.** A 2-bit-per-pixel opacity mask was planned and
  is not worth building. Shipping character data is entirely VQ
  (`--stats` reports `0 RLE` for every character sampled; RLE sprites
  come only from `append_runtime_sprite`, i.e. modded `.rhs.d` PNG
  overlays), and the VQ branch of `is_pixel_opaque` is already O(1) —
  one index read plus a dictionary lookup. The O(rows) `rle_pixel_at`
  walk that motivated the mask is not on the shipping path at all.
  Against that, the mask would cost roughly 177 MB for the fullgame
  bank's ~708 M canvas pixels (404,855 sprites averaging ~1,750 px), and
  building it eagerly at load would fight the lazy sprite streaming in
  `robin_assets::late_sprites`, where grids arrive *after* mission
  start. The hit test is also called only after an AABB rejection, so it
  runs a handful of times per mouse move, not per pixel per frame.

