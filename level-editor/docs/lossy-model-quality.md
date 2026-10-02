# Lossy model quality investigation (2026-09-27)

The shared generator is `refinement/blender/lossy_assets.py`. It preserves triangle
counts, rebakes the published textures, encodes AVIF, and optionally quantizes
vertex attributes. The original `model.glb` remains the source of truth.

## Split-asset publication repairs (2026-09-30)

Algorithm version 4 includes all retained meshes, even those outside the scene
hierarchy, so no textured primitive is left with old UVs pointing at a new atlas.
Packed layouts use one uniform transform to fit the texture tile. Collapsed
textured triangles receive separate small charts and are repacked; geometry stays
unchanged. Fractional gutters are capped at five percent, growing small atlases
instead of squeezing their charts almost flat.

Twenty missing or unsuitable derivatives were rebuilt with `--no-quantize` to
preserve transformed mesh nodes. Checks confirmed identical triangle positions,
scene structure and node metadata for all twenty outputs. Examples:

| Asset | Source GLB | Runtime GLB | Atlas |
| --- | ---: | ---: | --- |
| York north display tables remainder | 23,955,916 B | 9,496 B | 160² |
| York market state assembly | 311,529,252 B | 467,732 B | 1360² |
| Nottingham northwest timber house remainder | 1,933,728 B | 360,972 B | 2688² |

Eight-view comparisons at up to 512 pixels measured mean max-channel error of
2.85 for the tables, 2.04 for Nottingham's northern state assembly and 1.72 for
the northwest timber house; worst-view p95 was 7, 7 and 6 respectively (0–255).
Evidence lives in `work/map-compile/repaired-atlas-validation/` and
`work/map-compile/publication-table-gutters/`.

The Nottingham castle assembly requires an approximately 10,946-pixel single
atlas at the configured density. It retains its separate re-encoded textures
instead of losing detail at the 4096 cap (3,131,328-byte runtime GLB). Foliage and
other texture-reuse cases also retain their established layouts.

## Derby south gatehouse

The angle-based relaxation after Smart UV Project collapsed roof charts before
quantization. The old layout requested a 21,301-pixel atlas, hit the 4096 cap,
and achieved only 0.192 texels/map-pixel at the area-weighted median. Its lowest
decile was almost zero. Merely disabling quantization reproduced the black streaks:
mean image error was 12.172 with quantization and 12.167 without it.

Quantization also caused real damage: 66 tiny position triangles collapsed.
The new writer checks triangle area/orientation after rounding. Unsafe positions
remain float for the entire asset to preserve shared boundaries; unsafe UV
accessors remain float individually. Ordinary geometry still uses uint16.

The generator now keeps the projected charts, normalizes island scales, uses
eight-pixel minimum packing gutters, and extends baked edge texels for padding.
This avoids chart collapse and dark seams from adjacent-face gutter sampling.
Reducing padding to two pixels produced visible roof seams during verification.

Eight-view comparisons against the published source (640-pixel maximum render
edge, identical cameras) measured:

| Derby output | Mean max-channel error (0–255) | Worst-view p95 | GLB bytes |
| --- | ---: | ---: | ---: |
| Previous generator | 12.172 | 53 | 1,421,136 |
| Previous generator, no quantization | 12.167 | 53 | 1,943,988 |
| Fixed generator | 2.43 | 7 | 1,468,320 |

The fixed atlas remains 4096² and its density target is still capped by that
limit; the report records this. Validation is evidence, not a universal quality
threshold. Small occlusion-boundary differences can still have high peak errors.

## York versus Lincoln

The live receipts used identical generator settings. None of the inspected York,
Derby, or Lincoln base models used nearest filtering, so the separate nearest
density setting did not explain their differences.

York's unrefined projection-mapped models use a shared 8192² JPEG. That image size
does not describe an individual building's resolution: each building uses only a
small part of it. For example, `york-group-000` has a source area-weighted median
weakest-axis density of about 0.495 texels/map-pixel. Refined Lincoln assets have
separate surface textures around one texel/map-pixel (and Derby's gatehouse has
roughly 1.5–2). Some of the difference therefore exists in the published sources.

The old generator further capped each face's target by its *weakest* source
direction and sized the atlas to satisfy only the area-weighted median. This
discarded detail along better-sampled directions and left nearly half the surface
below target. It now uses the strongest source direction as the cap and targets
95% surface coverage (`--density-coverage`). York group 000 consequently changes
from a 336² live-library atlas to 688², achieving about one texel/map-pixel. Upsampling cannot
recover details absent from the source artwork.

Higher coverage can increase atlas memory, particularly on refined assets with
many projected charts. The 4096 maximum and size/density reports remain in force.

## Verification and rebuilding

The lossy bookkeeping/precision tests and asset-index tests pass (26 tests).
Blender rendered eight source/output view pairs for Derby south gatehouse, York
group 000, and Lincoln south gatehouse. Local evidence and rollback backups are
under `work/lossy-quality-20260927/`; `gatehouse-comparison.png` shows source,
previous lossy, and fixed lossy. The three verified assets and their previews are
updated in the local library.

Receipts now include an explicit algorithm version. Existing derivatives
are rebuilt on their next selected refresh; they are not silently considered
current because their old numeric settings happen to match. The full rollout
below used these defaults. Future full rebuilds can use:

```sh
blender --background --threads 2 --python-exit-code 1 \
  --python refinement/blender/lossy_assets.py -- library \
  --root library/3d-assets --run work/lossy-quality-full-rebuild --apply
```

Run from `level-editor`. Omit `--apply` for the plan, or add `--maps york derby
lincoln` to restrict the rebuild. Source models are never modified.

## Full library rollout

Rebuilt all 1,221 remaining stale assets and their previews across all 10 maps.
The three previously verified examples were already current. All four workers
exited successfully with zero failures. The final audit verified all 1,224 entries:
source GLB hashes unchanged, lossy source/output hashes valid, current algorithm
settings, and current preview source/output hashes and fingerprints. Declared
state/standalone variants all reference their base model, so no separate variant
GLBs needed rebuilding.

The baseline here is the library after the initial three-asset validation.
Active lossy GLBs grew from 161.49 MB to 224.67 MB; previews grew from 36.20 MB to
37.24 MB. Combined: **197.70 MB → 261.91 MB (+64.21 MB, +32.48%)**.
Original models, shared source blobs, backups, and scratch files are excluded.

The sum of decoded base-level RGBA8 texture sizes grew **2.85 GB → 11.13 GB
(3.90×)** across all assets. This is an aggregate footprint, not a measurement of
simultaneously resident GPU memory; it excludes mipmaps, previews, geometry, and
other renderer allocations. Higher surface-density coverage has a substantial
texture-memory cost even where AVIF keeps disk growth modest.

| Map | Current assets | Models + previews before (MB) | After (MB) | Base textures before (MiB) | After (MiB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Croisement01 | 57 | 1.65 | 2.54 | 16.5 | 43.0 |
| Croisement02 | 144 | 2.40 | 3.36 | 17.0 | 109.7 |
| Croisement03 | 92 | 1.47 | 2.06 | 9.8 | 156.5 |
| Derby | 42 | 17.02 | 24.06 | 379.8 | 1217.9 |
| Leicester | 72 | 32.38 | 41.80 | 542.7 | 2032.2 |
| Lincoln | 100 | 31.05 | 41.11 | 823.7 | 2439.0 |
| nottingham | 127 | 28.86 | 39.91 | 638.8 | 2326.7 |
| Sherwood | 128 | 59.21 | 68.23 | 35.3 | 821.2 |
| Wychford | 1 | 8.76 | 8.76 | 100.0 | 100.0 |
| york | 461 | 14.90 | 30.07 | 156.7 | 1370.9 |

Local run records are in `work/lossy-quality-full-rebuild-20260927/`:
`inventory-before.json` records the initial source hashes and sizes, `audit.json`
contains the final per-asset checks and measurements, and each `shard-N/backup/`
retains the previous derivative files and receipts. Source files were not edited.

## Sherwood fixed-padding collapse (2026-09-28)

The central oak exposed a different failure from Derby's earlier relaxation bug.
The original published UVs were valid, but algorithm 2 exported zero-area UVs for
488,078 of its 488,094 triangles. The 4096-square baked texture compressed to
1,777 bytes. This destroyed original-art surfaces as well as synthesized ones;
it was independent of the viewing camera and occurred before quantization.

A controlled Blender 5.2.2 trace measured each operation, restoring the same
pre-pack coordinates before each packing experiment:

| Operation | Zero-area UV triangles |
| --- | ---: |
| Smart UV Project | 0 |
| Average island scale | 1 |
| Pack Islands, FRACTION margin 0.003 | 487,411 |
| Pack Islands, FRACTION margin 0 | 1 |
| Pack Islands, SCALED margin 0.003 | 1 |

The fixed fractional gutter was the mass-collapse trigger. It does not shrink
with island size. Enough islands make the padding itself impossible to fit;
shrinking their textured interiors cannot solve that constraint. Blender still
returns `FINISHED`, with collapsed and out-of-tile UVs. The exact count differs
slightly when switching out of edit mode to inspect intermediate stages, but
the controlled padding comparison isolates the failing operation.

Our optimizer accepted that result because its stopping check only verified
`margin * atlas_size >= pack_margin_px`. At 4096 pixels, the initial 0.003 margin
already satisfies that check. It capped the enormous calculated atlas size,
baked the broken layout, and only checked exported mesh structure and counts.
Neither that check nor quantization's precision check detects UVs already
collapsed by the packer.

Algorithm 3 retains foliage's published layout and falls back to the original
layout when the requested density needs an atlas above the size cap. The
additional pre-bake guard explicitly rejects newly collapsed triangles,
non-finite coordinates, and coordinates outside the single atlas tile; these
also trigger source-layout re-encoding and a recorded `packing_failure` reason.
Original models and physical opacity remain intact. Reduced padding is not used
as a production workaround: it would violate the verified gutter requirement.

`refinement/blender/test_uv_packing.py` provides a small real-Blender regression:
400 separate cards with a 0.05 fractional margin need four tiles for padding
alone. Blender returns success, but our guard rejects the result. The identical
cards with a 0.005 margin pass. Run with:

```sh
blender --background --threads 2 --python-exit-code 1 \
  --python refinement/blender/test_uv_packing.py
```

The numerical guard was also checked against the actual failed oak GLB: it
rejects 349,702 collapsed triangles in the first foliage primitive alone.
The full local trace is in `work/sherwood-refinement/uv-collapse-diagnostic/`.

A separate opaque-image bug discarded synthesized RGB by interpreting ownership
alpha as physical transparency during Blender sampling. Opaque source textures
now use Blender's `NONE` alpha mode; MASK/BLEND images retain physical alpha.
This was real, but did not explain the source-camera tree damage above.

## Wychford atlas memory audit (2026-10-02)

Wychford references 96 assets from several source maps. Its derivatives were
80 algorithm-2 and 16 algorithm-3 outputs, including 20 images at 4096². Several
small props, fences and bridges spent most of their atlas on unused space.
The east-village footbridge's summed triangle UV area was only 0.2% of its 4096²
image. Two prop derivatives had collapsed UVs and almost blank 4096² images.

Algorithm 5 adds `--max-atlas-expansion` (default 4): after calculating the
rebaked atlas size, retain and AVIF-encode the source textures if the atlas would
contain more than four times their total pixels. This avoids upsampling small
source sets into oversized atlases. Four times is already the nominal RGBA8/BC7
storage ratio; larger expansion loses even against uncompressed source images.
The source UVs and image dimensions are retained in this path. Large shared map
images still benefit from rebaking a small asset and do not trigger this guard.
Receipts record the setting/version; reports identify `expansion_limited`.
Existing packing/opacity safeguards remain in effect. No blanket resolution
reduction was applied.

Rebuilt and published the 19 Wychford assets whose old derivatives used more
than four times their source-image pixels, including fresh previews and hash
receipts. Original model hashes were unchanged. This rollout combines the new
expansion guard with packing fixes absent from the old derivatives.

| Measurement | Before | After |
| --- | ---: | ---: |
| Selected assets' image pixels | 228.7 million | 23.3 million |
| All 96 assets' image pixels | 581.9 million | 376.5 million |
| 4096² images | 20 | 11 |
| Selected runtime GLB bytes | 7.41 MB | 6.92 MB |
| Estimated live GPU texture payload (with mips) | 1.15 GB | 0.67 GB |
| Wychford median load | 14.96 s | 12.27 s |
| Median peak renderer RSS | 1.83 GiB | 1.67 GiB |
| Median peak GPU resident memory | 1.22 GiB | 0.93 GiB |
| Renderer RSS two seconds after load | 1.56 GiB | 1.32 GiB |
| GPU resident memory two seconds after load | 1.05 GiB | 0.77 GiB |

Image totals count glTF image entries, before browser deduplication, and exclude
mipmaps. Live texture payload sums exact BC7 mip buffers and estimates RGBA8
mips; those viewport snapshots are separate from the matched timing batch.
Browser measurements used three fresh hardware Chromium profiles per
phase on Radeon 780M, two encoding workers, and localhost delivery. Peaks are
sampled lower bounds (~100 ms intervals); no forced GC was used. Native GPU
counters and process RSS overlap on this integrated GPU and cannot be summed as
unique physical memory. These small samples do not predict remote-network loads.

Seven representative assets were compared against their published source from
eight views. Six had mean max-channel errors between 1.20 and 2.18 (0–255), with
worst-view p95 between 3 and 10. The repaired bucket had mean 5.65 and worst-view
p95 20; it replaces a broken blank atlas, and its 240² rebake was visually checked.
The exact validated GLB bytes match the published derivatives. Blender's unlit
comparison measures texture/geometry differences, not the editor's custom foliage
shader; the whole scene was also checked in hardware WebGL.

The 23 lossy-generator tests and 15 asset-index tests passed. All library
derivative/preview receipt checks passed after publication.
`wychford-atlas-measurements.json` records asset hashes, per-asset savings,
validation metrics and browser measurements. Local images, detailed audits,
publication records and rollback backups are in `work/wychford-atlas-audit/`.

Reproduce the read-only audit from `level-editor`:

```sh
python3 refinement/audit_lossy_atlases.py library/scenes/Wychford.rhlos-map.json \
  --output work/atlas-audit.json
```

The audit reports image dimensions, UV bounds, summed triangle area, collapsed
UV triangles and approximate raster coverage. Raster coverage is deliberately
omitted for out-of-range UVs because clipping tiled textures would be misleading.
For one-map browser runs, set `BENCH_MAPS=Wychford BENCH_POOLS=2` when running
`benchmarks/bc7f/worker-pool.mjs` from the repository root (with its full path).

### Smart UV and packing diagnosis

Read-only Blender 5.2.2 probes rebuilt layouts for the remaining 11 Wychford
assets with 4096² derivatives. These measure fresh algorithm-5 layouts, not
occupancy of the older published derivatives. All default layouts requested
more than 4096 pixels at the configured 95% surface-density coverage; the current
generator would therefore retain source layouts instead of publishing them.

Padding and fragmentation matter: Leicester's great keep had 5,266
edge-connected UV components, and removing gutters reduced its requested edge
from 17,684 to 9,533. Removing gutters is diagnostic only, since baking and mips
need padding. Several alternative packs also failed the collapsed-triangle guard.
Changing packing shape alone did not consistently solve the large layouts.

A stronger finding is source-material aspect handling. Smart UV explicitly
uses `correct_aspect=True` while the objects still reference nonsquare source
images. The destination atlas is square. A controlled probe substituted a square
image on every material during the entire production unwrap/scale/pack sequence,
keeping geometry, source-derived density targets, and all settings unchanged:

| Asset | Default required edge | Square-material required edge | Default / square p95 axis distortion |
| --- | ---: | ---: | ---: |
| Leicester watermill | 4,580 | 1,759 | 14.01 / 1.74 |
| Leicester church side tower | 5,486 | 1,791 | 1.71 / 1.37 |
| Derby east watchtower | 6,575 | 1,655 | 26.65 / 1.35 |

These are calculated density requirements, before rounding and other output
guards, not validated replacement texture sizes. The probe implicates the
material-image aspect context across UV operations; it does not isolate Smart UV
from scale normalization or packing. Correcting destination aspect handling is
the next candidate to bake and compare visually before publishing smaller assets.
No production settings or library assets were changed by this diagnosis.

`wychford-packing-diagnostic.json` records source hashes, settings, packing history,
and measurements. Reproduce from `level-editor` with a fresh output directory:

```sh
blender --background --threads 2 --python-exit-code 1 \
  --python refinement/blender/diagnose_atlas_packing.py -- \
  --output work/atlas-packing-new
```

Use `--assets leicester-watermill leicester-church-side-tower derby-east-watchtower`
to limit the probe. Island counts use mesh-edge connectivity with exact matching
UV endpoints; raster coverage is approximate. Ideal pixel estimates target the
entire surface and are not strict lower bounds for the 95% coverage criterion.

### Square destination aspect (algorithm 6)

UV projection, island scaling, and packing now run with square material images.
Only the selected objects' materials are touched, and their original images are
restored in a `finally` block before baking or source-layout fallback. Source
density targets are calculated before this substitution. Receipts advance to
algorithm 6; density, padding, AVIF quality, and packing safety limits are unchanged.
The diagnostic retains its source-aspect baseline so it can still compare the
two configurations.

Rebuilt, visually reviewed, and locally published three Wychford assets, including
previews and receipts, with backups in `work/wychford-square-atlas/publication/`:

| Asset | Old atlas | New atlas | Old / new mean render error | Old / new worst-view p95 |
| --- | ---: | ---: | ---: | ---: |
| Leicester watermill | 4096² | 1760² | 3.11 / 4.55 | 13 / 16.87 |
| Leicester church side tower | 4096² | 1792² | 4.78 / 4.16 | 22 / 18 |
| Derby east watchtower | 4096² | 1664² | 1.48 / 2.57 | 4 / 8 |

Errors are measured against the source GLBs over eight views using the same
camera settings and undenoised renders, on a 0–255 channel scale. The watermill
and watchtower trade some fine detail for smaller atlases; the church tower's
error improves. All three meet the density target on at least 95% of surface area.
This is not a claim of identical quality. Structural and packed-UV checks passed,
and the exact visually reviewed output bytes were published.

Combined texture pixels fall from 50,331,648 to 9,077,760 (82% less). Combined
GLB size falls from 2,697,568 to 1,655,408 bytes (39% less); the church tower alone
grows by 1,472 bytes. Source model hashes remain unchanged. The 24 lossy-generator
tests, 15 asset-index tests, and complete library derivative-receipt verification
passed. Detailed measurements are in `wychford-square-atlas-measurements.json`;
local render comparisons are in `work/wychford-square-atlas/`.

Hardware Chromium loaded the updated Wychford scene and passed the BC7 worker,
sharing, fallback, reupload, and cancellation checks. Compressed texture mip
buffers decreased by exactly 55,004,256 bytes; total estimated viewport texture
payload decreased from 666,981,963 to 611,977,707 bytes. Uncompressed textures were
unchanged. Browser records include single-run timing and native memory snapshots,
but these are not a repeated timing benchmark; concurrent editor development also
limits comparisons beyond the texture payload. The other assets were not rebuilt.

### Full library rollout and bounded packing fallback

Regenerated the remaining 1,005 eligible library assets and their previews;
the three earlier replacements were already current. The 185 entries refused by
the existing static checks (including non-rendered regions and transformed
assemblies) were left alone. Sources were snapshotted, and source hashes checked
again under the publication lock before replacing derivatives. Previous files
and the index are backed up in `work/library-square-atlas-v6/publication/`.

This pass exposed an overly strict fallback introduced after algorithm 2:
exceeding the desired atlas density at 4096 caused the generator to restore full
source textures. The older generator could successfully bake those assets by
clamping the atlas size. Density is now a best-effort target again: valid atlases
are capped at `--max-size`, with required and achieved density retained in reports.
Fifteen regenerated assets reach the cap. This does not bypass collapsed-UV checks.

Unsafe AABB packing now retries from a fresh projection with Blender's CONVEX
packer. This fixes Leicester's great keep: the automatic retry produced the
exact bytes of the separately reviewed 4096² convex bake, avoiding a 106.6-million
pixel source fallback. Repeated padding/chart-rescue failure is classified as an
unsafe atlas. Eleven small assets still retain source layouts after both packers
fail; an unsafe-packing fallback larger than the atlas pixel budget is refused.

Source-layout re-encoding also crops unused image borders, remapping UVs without
resampling. Bounds include all indexed primitives sharing each image. Tiled UVs
are left alone; axes touching an image edge remain intact to preserve wrapping.
Crops include padding and align interior boundaries to four pixels. This trims
one additional spline texture in the current library. Cropping a bounding box
does not remove unused holes inside it and is not a replacement for atlas packing.

| Measurement across the 1,005 regenerated assets | Before | After |
| --- | ---: | ---: |
| Texture pixels | 2,475,971,308 | 965,173,640 |
| Runtime GLB bytes | 474,731,856 | 442,956,856 |

Pixels count image entries before deduplication and exclude mipmaps; this is
61% fewer pixels, not a measurement of browser RSS. Individual assets can grow
when meeting the existing density settings; the totals include those increases.
Sources, geometry counts, and packing safety were checked throughout. Thirteen
sampled assets were compared with their sources from eight views, covering capped
buildings, large reductions, a cropped spline, and small source-layout fallback.
These comparisons are not exhaustive and do not establish identical quality.
Mean errors range from 1.62 to 7.89 on a 0–255 channel scale; worst-view p95 is at
most 27. The final GLB hashes match the sampled comparisons.

The 27 lossy-generator tests and 15 asset-index tests passed. Tests cover source
image restoration, convex retry, crop texel-coordinate preservation, shared-image
bounds and repeat edges. `library-square-atlas-measurements.json` records asset
hashes, sizes, packing decisions and comparison results. Full local reports and
staged artifacts are under `work/library-square-atlas-v6/`.

All 1,005 replacements were published; none were held back. All regenerated
receipts match the current source hashes and settings, and complete library
derivative verification passed. Hardware Chromium checks passed for York and
Wychford, including the BC7 worker/fallback/sharing/reupload/cancellation checks.
Estimated loaded texture payload changed as follows (exact BC7 mip buffers plus
estimated uncompressed mip storage):

| Map | Before full rollout | After |
| --- | ---: | ---: |
| Wychford | 611,977,707 bytes | 315,275,968 bytes |
| York | 215,314,117 bytes | 215,314,117 bytes |

Wychford saves another 48.5% of its loaded texture payload. York's loaded texture
payload is unchanged; library-wide savings cannot be applied uniformly to each
map. These figures are not total process memory. The browser records also include
single-run timings and native memory snapshots, which should not be treated as
repeated benchmark results; native GPU allocations overlap process RSS.
