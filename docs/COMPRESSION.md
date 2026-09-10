# Compression investigation — sprites and maps

Summary of a benchmark sweep looking at whether we can shrink the shipping datadir. Tools: `crates/robin_rs/examples/sprite_size_bench.rs` (codec sweep), `crates/robin_rs/examples/datadir_breakdown.rs` (where-does-the-shipping-blob-budget-actually-go), `cargo run --bin convert_datadir -- --map-format jxl-{lossless,q90}` (the actual production conversion). Data: `datadirs/fullgame_gog` and `datadirs/demo_leicester_ecoste`.

## Latest recorded outcome

[Earlier replay preparation](perf/replay-plan-earlier.md): a bounded batch of
exact mission files now starts before the normal mission-load boundary. Five
same-package 16 Mbit/s pairs all improved, with a median paired saving of
**172 ms** and identical bytes. Loopback was mixed (median paired 39 ms slower).
This comparison uses explicit captured HTTP gzip responses; its absolute times
must not be compared directly with the older forced-Brotli benchmark.

Transport qualification from the [follow-up investigation](perf/replay-wasm-transport-next.md):
the earlier 6,639,157-byte HTTP Brotli capture used a forced Brotli request. Local
Wrangler with Chrome's normal mixed encoding header returned 7,822,465-byte gzip
for both routes. The earlier replay benchmark explicitly served the captured
Brotli representation; its measured timings remain valid for that fixture, but
do not establish normal browser/CDN encoding negotiation. No new transport win
was verified.

This document is a chronological research log. Later implementation sections
supersede the early recommendations: VQ sprites now use `sprite_codec` with
cross-variant contexts, and web RLE sprites use quality-gated lossy JXL atlases.
The 2026-08-30 campaign close-out records **36,727,000 B** through the first
mission and **9.3 s** to in-game in headless Chrome. Earlier tables retain
their original formats and measurements for comparison.

[Bzip3 measurements](#bzip3-measurements-2026-09-07): smaller than zstd on
original sprite streams, but still larger than `sprite_codec`. On three exact
RLE patch groups it beats xz by 6.6% in total, with substantially slower decode.

[Decoder performance](#decoder-performance-2026-09-07): format-preserving
context and grid-loop improvements reduce measured complete native decode
CPU cost by about 18%, with matching decoded output. A follow-up cuts
four-worker RLE/JXL latency by a further 65% through image parallelism.

[Startup projection removal](#startup-verification-and-native-projection-encoding-2026-09-08):
matched headless Chrome startup falls from 9.433 s to 5.583 s; remaining
simulation fingerprints migrate from JSON to native bitcode.

[Startup follow-up](#independent-sprite-groups-early-terrain-and-mask-uploads-2026-09-08):
five matched optimized Chrome pairs improve bootstrap from **4.276 s to
3.755 s**. Independent sprite groups, balanced worker dispatch, early terrain,
raw mask atlases, and exported geometry metadata are implemented. Shipping
schema is now **v16**; mission parts grow 3.15% in this fixture.

[Replay startup follow-up](#replay-startup-and-boot-payload-reduction-2026-09-08):
the browser boot bundle falls from **7,968,036 B to 3,702,350 B** by removing
verified source-audio duplicates. Replay admission overlaps runtime loading;
playback skips unrelated menu audio and live Restart capture. Five matched pairs
improve replay first-present from **20.010 s to 17.137 s at 16 Mbit/s**, and
**4.059 s to 3.930 s** on unlimited loopback. The separate-opacity/sprite-deferral experiment has been removed: it added
9,966,216 bytes overall and substantially delayed playback progress.

## Initial findings (superseded by later implementation sections)

- **Character sprites (~78% of bank, ~67% of shipping blob)**: keep the existing shipping format, but trim demo shipping banks to the sprite IDs reachable from RHS profiles loaded by the demo mission. The current Leicester demo v4 q80 blob keeps 64 774 / 65 100 sprite slots and is **35 213 242 B**.
- **Patch / animation-overlay sprites (~22% of bank)**: also keep RLE/VQ + zstd. Counter-intuitively, JXL *loses* on the full patch bucket: small UI/icon patches dominate the bucket and compress phenomenally well under cross-sprite zstd, swamping JXL's per-image overhead. JXL only wins on the ~20 large hand-painted overlays — too small a slice to be worth a runtime format detour.
- **Background maps (`Data/Levels/*/*.map`)**: switch from bzip2-compressed RGB565 to JXL. Lossless JXL modular saves ~15%; **visually-lossless JXL `-q 90` saves ~60%** (2.5× smaller than shipping today). Wired up end-to-end behind `convert_datadir --map-format jxl-q90`, decoded at runtime via `jxl-rs` (the official libjxl Rust port).
- **Interface resource pictures**: JXL is wired behind `--interface-image-format`, but the deployed shipping blob keeps interface pictures raw. Lossy interface JXL has broken transparent/keyed art in practice; only terrain maps should use lossy JXL.

## Demo-only follow-up, 2026-04-30

All follow-up measurements in this section use only `demo_leicester_ecoste`.

### Remove duplicate legacy sprite-bank bytes from `raw`

The shipping converter was bundling legacy `.bks` / `.dic` files into
`ShippingDatadir::raw` while also embedding the parsed `ShippingSpriteBank`.
Runtime sprite loading already short-circuits to `ShippingSpriteBank` before
loose-file I/O, so the raw legacy bank is redundant in shipping output.

On `demo_leicester_ecoste`, `--map-format jxl-q90 --zstd-window-log 30`:

```
variant                                      datadir.bin
before (raw .bks/.dic bundled)               36,762,637 B
after  (raw .bks/.dic omitted)               36,395,640 B
saved                                           366,997 B
```

The isolated field looked much larger (`raw.robinhood.bks` was ~24.96 MiB under
per-field zstd), but whole-blob zstd deduplicated most of it against
`sprite_bank`. Still, removing the duplicate is a real unconditional win and
shrinks the serialized raw payload from ~203.84 MiB to ~130.11 MiB.

### Lower JXL map quality options

The decoder path accepts arbitrary JXL maps, so the converter now exposes
additional lossy terrain-map choices:

```
--map-format jxl-q90   36,395,640 B
--map-format jxl-q85   35,709,071 B   (-686,569 B vs q90)
--map-format jxl-q80   35,271,793 B   (-1,123,847 B vs q90)
```

These are explicit fidelity tradeoffs, unlike the `.bks`/`.dic` omission. Keep
`jxl-q90` as the visually-lossless recommendation; use q85/q80 only when the
download budget is tighter than the terrain-map quality budget.

### zstd parameter sweep

A one-off reserialization sweep recompressed the shipping demo blob with
selected zstd parameters. On the post-`.bks` q90 blob, the best tested setting
was `TargetLength(1536)`:

```
w30-tl1536        36,393,636 B
current-w30       36,394,749 B
```

That saves ~1.1 KiB and is not worth making the production compressor more
exotic. `windowLog=31` was slightly larger on this demo payload; no-LDM was
identical at `windowLog=30`.

### Interface images as JXL

The large interface resource images (`Interface/DEFAULT.RES` plus interface
`.pak` bundles such as `Loading.pak` / slideshow paks) can also move from
raw RGB565-in-zstd to per-picture RGB-only JXL. This does **not** touch the
sprite bank, including patch/overlay sprites.

On `demo_leicester_ecoste`, with `.bks/.dic` omitted and `windowLog=30`:

```
variant                                                        datadir.bin
maps q80, interface raw                                        35,271,793 B
maps q80, interface jxl-q80                                    32,678,657 B
maps q80, interface raw, mission sprite trim                   27,883,531 B
maps q80, interface jxl-q80 alpha, mission sprite trim         25,447,760 B
saved by interface jxl after sprite trim                        2,435,771 B
saved vs v2 q80                                                 7,229,849 B
```

The canonical browser converter path for the artifact named
`v8-web-opus-q80.rhdata.zst` is the checked-in wrapper:

```sh
scripts/build_web_shipping_datadir.sh \
  datadirs/demo_leicester_ecoste /tmp/robin-web-shipping
```

The wrapper invokes `convert_datadir --format shipping --map-format jxl-q80
--audio-format opus --zstd-window-log 30` explicitly. This matters because the
converter's native-oriented defaults retain raw maps and source audio.

Only map quality is lossy. Interface pictures are left in the raw RGB565
shipping representation so transparent/keyed UI art remains exact. The
Opus is web-only; native and Android artifacts retain the default
`--audio-format source`. Historical size figures below describe their named
schema and remain as measurement provenance.

### Demo mission sprite trim

The demo converter no longer embeds every sprite referenced by every character
RHS present in the demo datadir. It follows the converted mission instead:

- mission soldiers/civilians, mission-required PCs, rescue PCs, and the demo
  boot party profiles that actually have RHS files;
- proto/mission patches, ambient animations, and targets resolved through the
  same animation RHS fallback order as runtime loading: current ambiance, Day,
  then base `Animations/`;
- mission bonuses, scroll/clover sprites, the level-load accessory preload
  table, and the non-forest `Blip00` alternate profile.

That still pulls in more than the actor-only estimate because the Leicester map
has real patch/overlay sprites, targets, objects, and blipped NPC art. The q80
shipping run logs:

```
sprite bank: keeping 47539 / 65100 sprites (94 required RHS profiles, 47549 broad RHS refs)
datadir.bin: 25,447,760 B with alpha-preserving interface JXL
```

### Verification note

`jxl-rs` enabled AVX512 by default, which crashed this local Cranelift dev
toolchain while decoding q80/q85/q90 JXL (`llvm.x86.avx512.* is not yet
supported`). The dependency is now built with `default-features = false`.

## Data under test

- **Background maps** (`Data/Levels/*/<name>.map`): legacy 16-bit picture format — bzip2-compressed rectangular RGB565 pixels. Sizes 1.6–8.8 MiB, dimensions 1408×960 to 2304×3520.
- **Sprite bank** (`Data/robinhood.bks` + `.dic`): 404 855 sprites and 134 shared 4-pixel-tile dictionaries for the fullgame (~602 MiB on disk); 65 100 sprites / 31 dictionaries for the demo (~73 MiB). Each sprite is either RLE-encoded (per-scanline `[first, size, pixels…]` skipping transparent runs) or vector-quantised (per-scanline `[first, size, u16 dict_indices…]` where each index names a 4-pixel tile). Pixel format is RGB565, transparent key `0x07C0`, shadow key `0x001F`.
- **`.rhs` character files**: animation metadata only (profile → action rows → per-frame bank-id references + offsets/delays). 1.6 MiB each for Robin.

## What was tested

### Codecs (per single image — map or sprite sheet)

- PNG (`png` crate) and oxipng `-o 4`
- Lossless JXL (`cjxl -d 0 -e 7 --modular=1`)
- Visually-lossless JXL (`cjxl -q 90 -e 7`)
- Lossless AVIF (`avifenc --lossless`)
- QOI
- Raw RGB565 + zstd levels 22 and 3
- Raw RGBA8 + zstd-22
- Tight-bounded per-frame RGB565 + zstd-22 (fair analog to the RLE bank: crop each frame to its opaque bounds before concat)
- Animated JXL (APNG → cjxl)
- AV1 lossless (ffmpeg libaom, yuv444p, `lossless=1`)

### Bank/format tweaks (at whole-character, per-bucket, and whole-bank scale)

- zstd-22 on the existing RLE/VQ `packed_data` bytes, concatenated — the direct apples-to-apples "how close is the shipping blob to the floor".
- Reordering sprites before compression, two ways: playback-order first-occurrence, and frame-index-first across all 16 directions of each action.
- 8-bit-packed VQ indices when the dictionary has ≤256 entries.
- Transparency-bitmap split: emit a 1-bpp opacity bitmap + dense RGB565 of only the opaque pixels.
- Horizontal-mirror deduplication across the same character (canonical form = `min(sprite, hflip(sprite))`).
- Canvas-aligned XOR delta between consecutive frames of one animation row.
- Palette encoding with per-character unique RGB565 → 8-bit or 16-bit indices.
- **Whole-character JXL atlas**: pack every unique frame of one character into a tight 2D atlas, with both alpha-keyed RGBA and RGB-verbatim variants (transparent-key kept as opaque pure green).
- **Per-patch JXL** (every patch/anim sprite, not just the top-20 cherry-pick): each sprite individually JXL'd, then concatenated and zstd'd — fair simulation of "replace the bank's patch sprites with JXL files".

## Results

### Background maps (fullgame)

```
asset                                   w     h      orig   png-oxi    jxl-ll   jxl-q90   565+z22
Custom1/Nottingham.map               2304  3520  8.80 MiB 11.19 MiB  7.44 MiB  3.51 MiB  9.48 MiB
Day/Croisement01.map                 1408   960  1.60 MiB  2.00 MiB  1.37 MiB 693.5 KiB  1.76 MiB
Day/Croisement02.map                 1792  1152  2.47 MiB  3.07 MiB  2.12 MiB  1.04 MiB  2.67 MiB
Day/Croisement03.map                 1408   960  1.59 MiB  1.96 MiB  1.36 MiB 649.4 KiB  1.73 MiB
```

`jxl-ll` (lossless JXL modular) wins every row at ~0.85× the existing bzip2-RGB565 file. `jxl-q90` (VarDCT, visually lossless) lands at ~0.4× — 2.5× smaller than shipping. AVIF, QOI, oxipng, and zstd-on-raw-RGB565 are all strictly worse than `jxl-ll`.

### Sprite animations (10 random rows, fullgame, ≥8 frames each)

```
asset                                                      w     h      orig  z22-orig   565+z22   bound+z  z22-delt
Soldier A00/Soldat A:row1965(act231, 12f)                484    56  12.3 KiB   5.4 KiB  10.3 KiB  10.6 KiB  20.1 KiB
WillScarlet/Will Ecarlate:row2117(act255, 11f)           416    61   9.8 KiB   4.8 KiB   9.0 KiB   9.0 KiB  17.1 KiB
WillScarlet/Will Ecarlate:row856(act42, 10f)             344    49   7.7 KiB   4.5 KiB   8.5 KiB   8.7 KiB  15.4 KiB
Soldier A04/Soldat A:row1027(act104, 8f)                 356    58   9.3 KiB   4.2 KiB   7.8 KiB   7.9 KiB  13.0 KiB
Sherif/Sherif:row125(act6, 22f)                          592    57  16.2 KiB   7.4 KiB  12.8 KiB  13.1 KiB  23.0 KiB
Friar Tuck/Frere Tuck:row124(act6, 22f)                  656    50  15.8 KiB   7.4 KiB  12.0 KiB  12.4 KiB  23.0 KiB
Guisbourne/Guisbourne:row16(act1, 9f)                    320    56   8.6 KiB   1.4 KiB   3.0 KiB   2.8 KiB   3.0 KiB
RobinTown/Robin des bois:row1581(act85, 10f)             316    60   9.0 KiB   4.7 KiB   8.3 KiB   8.3 KiB  15.0 KiB
Scatlock/Scatlock:row147(act50, 8f)                      304    53   7.8 KiB   3.4 KiB   6.6 KiB   6.7 KiB  10.8 KiB
Soldier A00/Soldat A:row787(act72, 10f)                  476    65  12.8 KiB   5.7 KiB  11.6 KiB  11.6 KiB  19.3 KiB
```

- `z22-orig`: zstd-22 of concatenated RLE/VQ `packed_data` for this row's frames. **Wins every row**, 0.33–0.47× of raw RLE/VQ, because zstd within one row also dedupes the repeated frame IDs that appear in the animation cycle.
- `565+z22`: zstd-22 of a rectangular decoded-RGB565 sprite sheet. Distant second.
- `bound+z`: zstd-22 of tight-bounded RGB565 frames concatenated. Same ballpark as `565+z22`; transparency-stripping is free because zstd already matches runs of `0x07C0` at ~0 bytes.
- `z22-delt`: canvas-aligned XOR delta between consecutive frames in the animation, then zstd-22. 2–3× *worse* than `z22-orig` — the transparent padding per-frame on a shared canvas costs more than the delta saves.

For context:

- PNG/png-oxi, lossless JXL, AVIF, QOI, rgba+zstd, anim-JXL, AV1 lossless: **all worse than `z22-orig`**, most of them 1.5–4× worse. Image codecs lose on hand-drawn pixel art with hard alpha edges and lots of transparent border.
- `jxl-q90` is usually *bigger* than lossless JXL on sprites — VarDCT has a fixed per-tile header cost that dominates on 300×60-pixel sprite sheets.

### Whole-character (all profiles, all rows, all frames — 5 characters)

```
character          unique  orig-rle  z22-orig  z22-play  z22-frm1  z22-u8vq  z22-tspl  mir%  best/o
RobinTown            7584  7.47 MiB  2.95 MiB  2.95 MiB  2.96 MiB  2.95 MiB  3.78 MiB  0.0%   0.39×
LittleJohn           5713  9.73 MiB  2.92 MiB  2.92 MiB  2.94 MiB  2.92 MiB  3.64 MiB  0.0%   0.30×
Friar Tuck           5505  4.39 MiB  1.95 MiB  1.95 MiB  1.96 MiB  1.95 MiB  2.41 MiB  0.0%   0.44×
Soldier A00          5856  5.43 MiB  2.31 MiB  2.31 MiB  2.32 MiB  2.31 MiB  2.96 MiB  0.0%   0.42×
Sherif               5488  5.04 MiB  1.94 MiB  1.95 MiB  1.95 MiB  1.94 MiB  2.49 MiB  0.0%   0.38×
```

Baseline (`z22-orig`) puts each character at 0.30–0.44× of the raw RLE/VQ bytes. Everything else:

- `z22-play` (playback-order first-occurrence): ≤0.3% difference.
- `z22-frm1` (frame-index-first across 16 directions): ≤0.3% difference.
- `z22-u8vq` (pack VQ indices as u8 when dict ≤256 entries): 0% difference — zstd already compresses away the zero high bytes.
- `z22-tspl` (transparency bitmap + dense opaque pixels): 20–30% **worse**. Separating the bitmap breaks the 2-D periodicity zstd was exploiting on the rectangular blob.
- Horizontal-mirror dedup: **0% exact-mirror hits** across 30 k+ unique sprites in these five characters. The art is hand-drawn with directional lighting, not pixel-bilateral.

#### Whole-character JXL atlas (per-character JXL fails too)

Packing every unique frame of one character into a single tight 2D atlas and JXL-encoding it. Both alpha-keyed RGBA and RGB-verbatim (transparent key kept as opaque pure green) variants tested:

```
character      unique  orig-rle  z22-orig   rgba-LL    rgba-Q90    rgb-LL     rgb-Q90    best/o
RobinTown        7584  7.47 MiB  2.95 MiB  10.58 MiB  11.93 MiB  11.10 MiB   14.13 MiB   0.39×
LittleJohn       5713  9.73 MiB  2.92 MiB  10.41 MiB  12.17 MiB  10.49 MiB   15.20 MiB   0.30×
Friar Tuck       5505  4.39 MiB  1.95 MiB   5.13 MiB   6.54 MiB   5.26 MiB    8.30 MiB   0.44×
Soldier A00      5856  5.43 MiB  2.31 MiB   8.14 MiB   8.05 MiB   8.28 MiB   10.24 MiB   0.42×
Sherif           5488  5.04 MiB  1.94 MiB   6.64 MiB   7.57 MiB   6.98 MiB    9.49 MiB   0.38×
```

JXL is **2.5–4.5× worse** than the shipping format on character atlases regardless of alpha representation. Two effects compound:

1. **Content mismatch.** Character sprites are hand-drawn pixel art with hard 1-pixel alpha edges, flat-shaded regions with abrupt color jumps, no continuous-tone content. JXL's entropy models assume natural-image statistics; they pay extra for every hard edge instead of compressing it.
2. **VarDCT eats edges, not gradients.** `q90` is *worse* than lossless on characters because the VarDCT block-DCT modes pay storage cost to represent the ringing they introduce around hard alpha transitions. q90 only wins on photographic content (the maps + the largest hand-painted overlays).

The RGB-verbatim variant (keeping `0x07C0` as opaque pure green pixels, no alpha channel) is consistently *worse* than RGBA-keyed: explicit alpha lets JXL's modular predictor skip transparent regions; RGB-verbatim forces it to encode them as part of the color stream.

### Patch / animation-overlay sprites (the other 22% of the bank)

Cherry-picked top-20 largest patch sprites (each 200–354 KiB packed; mostly 200×300 hand-painted building overlays):

```
top-20 in one zstd22 stream (closest analog to shipping today)
  packed+z22  = 2.53 MiB   (baseline)
  jxl-ll+z22  = 2.20 MiB   (0.87×, -13%)
  jxl-q90+z22 = 1.08 MiB   (0.43×, -57%)
```

JXL wins on the top-20 alone — these are big enough that the per-image overhead is dwarfed by the actual pixel data, and the photographic-ish content (gradients, shadows, wood texture) is exactly what JXL VarDCT is tuned for.

But on the **full** patch bucket (all 1337 patch sprites in the demo, 17 794 in the fullgame), JXL flips around and *loses*:

```
## Full demo patch bucket (1337 sprites, all individually JXL'd)
format                            raw sum zstd22 (in-stream) vs packed+z22
packed RLE/VQ                    7.51 MiB     1.84 MiB        1.00×  (baseline)
JXL lossless                     3.75 MiB     3.71 MiB        2.02×  (worse!)
JXL q90                          2.69 MiB     2.59 MiB        1.41×  (worse)
```

The patch bucket is dominated by hundreds of small sprites (UI buttons, font glyphs, icon variants, small effects) where:

1. Cross-sprite zstd LZ matching captures massive redundancy (7.51 MiB → 1.84 MiB, ratio 0.245×).
2. Per-image JXL overhead (signature box + bitstream header + entropy-coding tables, ~150–300 bytes minimum) is a real tax on a 500-byte sprite.

So replacing the whole bucket with JXL files pays the per-image tax 1337 times *and* loses cross-sprite zstd matching. Both effects compound.

Conceivably you could ship the top-20 in JXL and keep the rest as RLE/VQ inside the existing zstd stream, but the marginal demo win is ~240 KiB (out of 34.9 MiB) and it requires a per-sprite format flag in the runtime decode path. Not worth the complexity.

### Whole-bank reorder (demo datadir — 73 MiB raw)

```
ordering                              raw zstd22 long=31      ratio
bank (shipping today)           72.77 MiB    25.16 MiB      0.35×
reordered (action/frame/dir)    72.77 MiB    25.32 MiB      0.35×
reorder vs bank                              +168.6 KiB     +0.65%
```

With the actual shipping compressor settings (`windowLog=31`, `EnableLongDistanceMatching(true)`, level 22), reordering ~500 MiB of bank data by `(character, action, frame-index-in-action, direction)` lands 0.65% *larger* — measurement noise at best, and not a win.

Side observation: the reordered blob compressed 2.3× faster (49.7 s vs 113.3 s). The reorder is genuinely putting similar sprites closer together, so the LZ encoder finds shorter-distance matches more cheaply. But total output size at level 22 with long mode is the same because long-range matches are encoded nearly as cheaply as short-range ones.

### Demo `datadir.bin` component breakdown

Where the 34.9 MiB demo shipping blob actually goes (per-field bitcode → zstd-22):

```
field                     entries  bitcode raw       zstd22   % blob
sprite_bank                     1    63.33 MiB    23.43 MiB   67.5%
res_files                       4    15.90 MiB     3.73 MiB   10.7%
raw (.map + .min)               2     6.64 MiB     6.64 MiB   19.0%
pak_files                       1     4.50 MiB    438.0 KiB    1.3%
levels                          1     1.23 MiB    371.6 KiB    1.1%
rhs_files                      13     2.12 MiB    299.1 KiB    0.9%
scripts                         1     57.8 KiB      5.3 KiB    0.0%
profiles                        1      9.3 KiB      3.0 KiB    0.0%
keysets                         2        743 B        334 B    0.0%
red_files                       1        108 B        110 B    0.0%
```

Two notable observations:

- **`raw`'s zstd column equals its bitcode column** (6.64 MiB each). The `.map` files are bzip2-compressed inside, so zstd can't squeeze any more out. That's exactly why JXL conversion is so impactful: we're replacing already-maxed-out compression with a format that genuinely fits the content. After `--map-format jxl-q90`, `raw` drops from 6.64 → 2.81 MiB (saves ~4 MiB on a 34.9 MiB blob = 11%).
- **The sprite bank's zstd ratio is 0.37×** (63.3 MiB bitcode → 23.4 MiB zstd). Holds at bank scale; matches the per-character estimates above.

After the converter `--map-format jxl-q90` flag is wired up, the demo blob drops:

```
flag                                    datadir.bin    saved   ratio
--map-format raw (default)               34.90 MiB       –     1.00×
--map-format jxl-lossless                33.89 MiB    1.01 MiB 0.97×
--map-format jxl-q90                     31.06 MiB    3.84 MiB 0.89×
--zstd-window-log 30 (wasm-compatible)   +0.01% (noise)
```

For the fullgame the absolute savings scale roughly with the `.map` count and dimensions (40+ MiB of `.map` files vs the demo's 6.6 MiB), so the same flag plausibly saves 20+ MiB on the fullgame shipping blob.

## Shell sanity check

Direct zstd-22 on the raw `.bks` + `.dic` files (full-game, first 100 MiB of .bks + full 9.3 MiB .dic):

```
first 100 MiB of .bks      → 24.02 MiB (0.24×)    72 s  (zstd -22 --long=27)
full 9.25 MiB of .dic      →  4.65 MiB (0.50×)     6 s
```

Extrapolating the `.bks` ratio to 565 MiB: ~141 MiB total for the full bank zstd-22'd from the raw on-disk format. Consistent with the 0.35× we see at the demo-bank scale.

## Per-idea post-mortem

- **Reorder sprites** — no win at any tested scale. Level-22 zstd with a 2 GB window doesn't care about order.
- **u8-pack VQ indices** — no win. zstd flattens the zero high bytes for free.
- **Transparency bitmap split** — worse. Breaks the horizontal periodicity the rectangular blob has.
- **Horizontal mirror dedup** — no exact mirrors exist in the data.
- **Frame-to-frame XOR delta on shared canvas** — much worse. Full-canvas per-frame bytes dominate even when the XOR is mostly zeros.
- **Palette** — within 1–2% of raw RGB565+zstd, because >256 unique colours per character forces u16 indices, which zstd compresses identically to the raw colours.
- **Animated JXL / AV1 lossless** — both 1.5–3× worse than `z22-orig`. Wrong tool for hand-edge pixel art.
- **AVIF lossless / QOI** — both 2–4× worse than `z22-orig`. Unusable here.
- **Lossless JXL on sprites** — 1.3–1.8× worse than `z22-orig`. JXL needs continuous-tone content to shine.
- **Visually-lossless JXL (q90) on sprites** — often bigger than lossless JXL. VarDCT per-tile header cost dominates at these sizes.
- **Per-character JXL atlas** (whole character packed into one big JXL, both alpha-keyed RGBA and RGB-verbatim variants) — 2.5–4.5× worse than `z22-orig`. Atlas scale isn't enough to beat the content mismatch.
- **Per-patch JXL (full bucket)** — 1.4–2.0× worse than `z22-orig` on the bucket as a whole, despite winning on the cherry-picked top-20. Small-sprite cross-zstd matching dominates.

## Recommendations

1. **Ship the current sprite format, but trim mission-specific demo banks.** `ShippingSpriteBank` → bitcode → zstd-22 `windowLog=31` + long-range matching is still the right per-sprite representation. The useful win is omitting unreachable sprite payloads from the demo shipping bank.
2. **Convert maps to JXL.** Wired up: `convert_datadir --format shipping --map-format jxl-q90` transcodes every `.map` file via `cjxl`, the runtime decodes them via `jxl-rs` (the official libjxl Rust port). The converter feeds cjxl an RGB-only PNG (maps are fully opaque) and the decoder asks for `JxlColorType::Rgb`, so JXL reports zero extra channels and the pixel-format negotiation is trivial.
3. **Default to `--zstd-window-log 30` for wasm shipping.** The 31-bit long-range window saves <0.02% over 30 on this data and 32-bit zstd builds (wasm32) refuse to decode windowLog=31 streams.
4. **Don't add per-sprite/per-patch JXL.** The investigation made the case clearly: the bank's RLE/VQ + zstd pipeline is the right tool for pixel-art sprites. If we want substantial further sprite gains we'd need to go lossy (k-means palette quantisation, perceptually-weighted), and that's a format and tooling change that's out of scope here.

## Mission-selective shipping layout

Shipping format v8 applies the trimming recommendation without breaking the
compression properties measured above. The converter now emits a file tree:

```
Data/datadir.bin
Data/missions/<mission>-w<window>-<content-hash>.rhmission.zst
Data/rhs/<rhs>-w<window>-<content-hash>.rhmission.zst
Data/terrain/<content-hash>.rhmission.zst
Data/audio/assets/<content-hash>.opus
```

`datadir.bin` is the boot manifest: profiles, shared UI/text resources, level
descriptors, the sprite-bank dictionary/index shape, and a mission dependency
graph. Each mission file contains its parsed level and script, terrain/minimap,
and loading resources. Each RHS file contains that character/accessory's
parsed RHS metadata and only its reachable sprite-bank slots. There is
intentionally no raw RHS compatibility copy: runtime sprite lookup consumes
the parsed form directly.

RHS payloads are intentionally **not** split into one file per sprite. The
measurements in this document show that hundreds of small related sprites gain
substantially from cross-sprite zstd matching, and a file per sprite would also
pay format and HTTP overhead for every frame. One zstd stream per RHS keeps
those within-character matches while allowing heroes, accessories, and common
effects to be shared by several mission dependency lists. Browser requests for
a mission's files run concurrently. Decoded parts are move-merged into only the
active mission and then released; ordinary HTTP caching avoids retransferring
content-addressed files when a later mission reuses them.

The Opus recipe stores only logical-path, encoded-size, and authoritative
source-duration metadata in `datadir.bin`. Audio is not a mission dependency:
the browser fetches a content-addressed `.opus` file at first playback and
passes its JavaScript `ArrayBuffer` directly to `decodeAudioData`. Neither the
encoded stream nor decoded PCM is copied into wasm memory. The converter's
default `--audio-format source` remains the native/Android layout: it embeds
source audio in the relevant shipping payloads instead of requiring Web Audio.

A raw-map Leicester conversion (`--map-format raw --zstd-window-log 30`) gave
this preliminary layout measurement:

```
boot manifest                 5,955,105 B
mission core                  7,914,712 B
52 shared RHS payloads       27,112,169 B
all files                    40,981,986 B
```

That single-mission demo is about 6 MiB larger than the former monolith because
separate zstd frames cannot match across the boot/core/RHS boundaries. This is
an intentional latency and reuse tradeoff, not a compression-size win for a
one-mission package. Full-game and replay use are the target: startup fetches a
small manifest, a mission fetches only its dependency closure, and later
missions reuse already cached RHS files. Production map artifacts should still
use `--map-format jxl-q80` (or the selected quality) and window log 30 for wasm.

## Wasm transfer and resident-memory follow-up (2026-08-28)

The split format changes what should be measured. There are now three different
budgets, and improving one does not necessarily improve the others:

1. bytes transferred before the menu;
2. additional bytes transferred at the mission boundary; and
3. the decoded wasm heap and GPU/audio allocations retained after loading.

The development wasm seen in a local Vite session was **88,035,056 B**. That is
not the shipping size: it includes development code and debug information. The
same source built with the `wasm-release` profile, passed through `wasm-bindgen`,
then through `wasm-opt -Oz --strip-debug --strip-dwarf` and `wasm-strip`, is
**13,114,717 B**, or **4,923,318 B** with gzip level 9. The optimized module is
10.88 MiB code and 1.55 MiB initialized data; the rest is wasm metadata. The
publish workflow must run `wasm-bindgen` *before* Binaryen: optimizing the raw
Rust wasm first can remove wasm-bindgen adapter metadata and makes the pinned
wasm-bindgen reject the module.

The data path has larger opportunities than another compiler flag:

- The current boot manifest is **9,446,491 B compressed -> 73,238,365 B
  bitcode**. Its largest decoded fields are the raw bundle (about 37.2 MiB),
  parsed resource files (about 28.6 MiB), and sprite dictionaries (about
  3.9 MiB). In particular, `Interface/DEFAULT.RES` exists both as a roughly
  21.6 MiB raw archive and as parsed `ResourceManager` data. Zstd can match the
  duplicate bytes on the wire, but wasm retains both representations.
- The earlier first-full-game-mission measurement (`H01_Lin_VL`, about 32 MB)
  predates authoritative audio dependencies and therefore is not a valid
  network-total measurement. It covered the mission/RHS shape but omitted the
  sounds that the synchronous runtime can play.
- The publish workflow eagerly fetches **527** demo audio files before boot,
  one request after another. They total **9,162,515 B** (8,029,994 B if each is
  gzip-9 encoded). This is independent of the shipping datadir and is currently
  paid even when a mission never plays most of those files.
- Every RHS payload contains a `Vec<Option<ShippingSprite>>` with one slot for
  every global bank id. The full-game vector has about **404,855 slots**, even
  when one RHS owns only a few sprites. An option is approximately 20 bytes on
  wasm32 before its `packed_data`, so each decoded RHS starts with roughly
  **7.7 MiB of sparse index storage**. A mission with 55 RHS dependencies can
  therefore transiently retain more than 400 MiB just in mostly-`None` vectors.
- `loaded_files` retains every decoded RHS part, while `install_mission_parts`
  clones its profiles, raw bytes, and sprites into a merged mission. Activating
  the mission clones `payload.raw` once more for the VFS, and an `SbFile` read
  currently clones the selected raw file again. This makes resident memory much
  larger than either the compressed download or the raw bitcode byte count.
- Each independently compressed part currently advertises the requested
  `windowLog=30` (a 1 GiB zstd window), including RHS files only a few MiB in
  size. The decoder does not necessarily commit a full GiB for every frame, but
  the frame's requirement is needlessly high and generic zstd tools refuse it
  unless explicitly allowed. Per-file adaptive windows should cap the window at
  the smallest power of two that covers that payload; the manifest alone needs
  a larger value.

### Recommended order of work

1. **Make RHS sprite storage sparse.** Encode sorted `(u32, ShippingSprite)`
   pairs, or parallel `ids`/`sprites` vectors, in each RHS part. Allocate the
   dense runtime bank only once while installing the selected mission. This is
   primarily a several-hundred-MiB heap and decode-time win; compressed size may
   improve modestly because bitcode no longer emits 400k enum tags per file.
2. **Stop retaining two mission representations.** Cache compressed bytes or an
   `Arc`-backed compact decoded part, move data into the active mission, and
   discard parts that are not needed for a future mission. Make VFS blobs
   `Arc<[u8]>` (with a serialization DTO if bitcode should remain plain-`Vec`)
   so mounting and `SbFile` reads do not copy whole RHS/map files.
3. **Remove the redundant RHS representation.** Runtime sprite scripting still
   opens the raw RHS through `SbFile`; the parsed `rhs_files: RhsData` copy is
   used to build the shipping payload but not to execute the mission. Either
   omit `RhsData` after conversion, or migrate runtime lookup to it and omit the
   raw RHS. Do not keep both.
4. **Remove boot-time raw/parsed duplication.** Audit `ShippingDatadir::raw`
   against `res_files`, profiles, fonts, and other parsed fields, then retain
   exactly the representation each wasm loader consumes. `DEFAULT.RES` is the
   first target. This attacks the 73.2 MiB boot heap even if the 9.45 MiB wire
   size changes little.
5. **Make dependencies depend on runtime state.** A mission manifest can know
   mission-authored actors, but not the player's current gang, inventory, or a
   replay's initial state. Store an RHS-name-to-content-file index in the boot
   manifest and add those runtime names at the async mission boundary. Avoid
   unconditional loading of every bonus, relic, and accessory merely because a
   save could contain it.
6. **Make audio mission-selective.** Boot only the menu music/UI sounds; fetch
   the selected mission's music, voices, and required effects at the same async
   boundary as its RHS files. The current preload loop should also fetch in
   parallel or load a small number of content-addressed packs instead of making
   527 serial requests. Demo audio is overwhelmingly WAV; lossless repacking or
   Vorbis/Opus conversion should be benchmarked, but lazy selection is the
   unconditional first win.
7. **Use adaptive zstd windows and benchmark a shared dictionary.** Independent
   RHS frames lost the cross-file matches of the old monolith. Zstd explicitly
   recommends trained dictionaries for collections of small related payloads.
   Put one content-addressed RHS dictionary in the boot manifest, reuse a
   prepared decoder dictionary, and measure total mission closure size and
   decode peak before adopting it. A smaller advertised window is valuable even
   if the dictionary does not win.
8. **Trim wasm features by target, then measure again.** Kira's default feature
   set includes FLAC, MP3, Ogg/Vorbis, PCM, and WAV decoders. The demo payload is
   490 WAV files plus non-audio metadata, although full-game data also contains
   Ogg. A wasm-specific `kira` feature set should include only the formats that
   the web publisher actually emits. Keep native features separate. Repeat this
   process for archive/editor paths that wasm never invokes; do not infer savings
   from a Cargo dependency list without comparing post-`wasm-opt` artifacts.

A `twiggy` pass over the 20,007,360-byte pre-bindgen release module explains
part of the compiler-side difference. Debug function names alone are 4,038,689
bytes and wasm-bindgen's adapter metadata is 734,321 bytes; both disappear from
the served artifact. The largest named executable bodies include native-bitcode
decoders for `ActorCivilian` (290,064 bytes) and `ActorSoldier` (289,696 bytes),
plus their encoders. Legacy-save adoption/read paths and JSON `PlayerCommand`
decoding also appear among the largest individual bodies. Follow-up code-size
experiments should therefore be controlled builds of:

- wasm-specific Kira features (WAV plus Ogg/Vorbis only if the published data
  uses it), compared after `wasm-bindgen` and `wasm-opt`;
- browser builds without legacy binary-save import and obsolete JSON replay
  decode, if product requirements confirm those imports are not exposed;
- separate WebGPU and WebGL fallback artifacts. The current universal module
  contains both wgpu paths, and the tested Firefox/Radeon machine actually
  selected GL, so removing WebGL from the only artifact is not viable.

The large bitcode bodies are not dead compatibility code: compact replay and
snapshot decoding needs them. Reducing those requires a narrower wire DTO or a
different snapshot boundary, not merely hiding derives behind cfg attributes.

## Schema-v5 implementation follow-up (2026-08-28)

The memory recommendations above are now reflected in schema v5:

- Per-RHS sprite storage is a sorted sparse list of `(u32, ShippingSprite)`
  entries plus the global bank length. The dense runtime index is allocated
  once for the active mission, rather than once per decoded RHS chunk. Packed
  sprite buffers are `Arc<Vec<u16>>`, so the decoded shipping payload and
  `FrameHolder` share them instead of retaining a second pixel-data copy.
- RHS chunks ship parsed metadata only. A normalized parsed-RHS registry feeds
  `SpriteScriptor`; loose native datadirs retain the legacy `SbFile` parser.
- Split files are decoded and move-merged directly into one active mission.
  Decoded part caches and previously visited merged missions are not retained.
- Mission raw files become cheap-clone shared asset buffers. `SbFile` cursors
  share those buffers, and the VFS has one replaceable active-mission slot, so
  opening files or visiting another mission no longer deep-clones or stacks
  raw mounts.
- Boot raw copies of parsed `DEFAULT.RES`, `Level.res`, Exclamations
  `actors.res`, and `profile.cpf` are omitted. Unused `Text/actors.res` and
  launcher-only `slideshow_in.pak` are omitted entirely. Parsed shipping
  resource managers explicitly disable legacy archive recovery, so a future
  accidental recovery attempt returns a clear error instead of depending on a
  raw archive that is no longer shipped.
- Character RHS dependencies are indexed by CPF profile in the boot manifest.
  The mission boundary unions authored dependencies with the selected team and
  every currently eligible gang reinforcement; action-capability mappings add
  only the projectile and pickup masters those characters can create. A load
  from a decoded save uses an explicit conservative object-master closure until
  exact saved entity types are threaded to this boundary.
- Browser audio is no longer one 527-file boot preload. Menu audio remains at
  boot; mission refs add a shared FX chunk, shared exclamation metadata,
  content-addressed per-actor localized voice chunks for possible participants,
  and the mission profile's exact green/yellow/red music. The boot manifest
  records authored speaker IDs and CPF-profile-to-speaker mappings; runtime
  publishes the precise mission/team/reinforcement speaker closure and keys the
  process sound cache by that closure. Missing selected metadata or samples are
  conversion/runtime errors rather than silent omissions. Music packaging
  preserves the source datadir's WAV or Ogg representation, matching the audio
  backend's existing fallback. The JS boot preloader fetches its small manifest
  with bounded concurrency.
- Wasm Kira keeps PCM/WAV and Ogg/Vorbis but omits unused MP3/FLAC decoders;
  native builds retain the complete default decoder set. In the measured build
  the final schema-v5 artifact is 13,065,723 B and 4,888,828 B with gzip-9,
  compared with the earlier 13,199,749 B / 4,965,145 B build. Concurrent
  source edits make that size delta
  directional rather than a controlled feature-only A/B; the feature graph and
  release build were verified directly.
- Dependency fetches are bounded to eight concurrent files and each decoded
  part is move-merged immediately. This prevents mission startup from retaining
  all compressed responses and all decoded part shells alongside the final
  merged payload. JS boot preloads use the same fetch/install/release pattern.
- `--resume` filenames include the requested zstd window and existing chunks
  are decoded and compared with the exact native-bitcode payload before reuse.
  Compression uses at most four workers and writes each completed chunk from
  its worker, avoiding a result vector containing the entire compressed RHS
  corpus.

The final raw-map full-game validation measured `H01_Lin_VL` as follows. This
is deliberately a conservative source-fidelity build; production q80 JXL
reduces the map-heavy mission core but does not change the audio totals.

```
boot manifest                 9,269,836 B
mission core                 18,363,666 B
55 parsed RHS chunks         27,093,269 B
shared effects               38,793,824 B
13 voice chunks              36,409,729 B
mission music                 1,821,174 B
exclamation metadata              1,473 B
mission boundary total      122,483,135 B (72 files)
boot + first mission        131,752,971 B
```

The difference from the historical ~30 MB figure is overwhelmingly audio:
75.2 MB of effects and voices are now included authoritatively rather than
being omitted from the accounting. Vorbis/Opus conversion remains the largest
available wire-size follow-up; it requires a controlled quality/determinism
benchmark and duration-decoder support before replacing the source WAV files.

An exact zstd window benchmark over the previous 223-file full-game RHS corpus
found adaptive windows effectively wire-size neutral: 196,338,366 B with every
frame advertising `windowLog=30`, versus 196,348,002 B when each frame pledges
its source size and uses `ceil(log2(size))` (+0.0049%). The H01 closure was
slightly smaller (32,035,889 B to 32,030,733 B). The maximum RHS decoder window
falls from 1 GiB to 16.1 MiB (9.70 MiB within H01), so schema-v5 compression
now pledges input length and caps each frame adaptively.

A shared trained zstd dictionary was measured and rejected. A 112,640 B COVER
dictionary increased the full RHS corpus by 3.62% including the dictionary and
increased H01 by 3.37%; a 16 KiB fastCover dictionary was worse. Even an oracle
that used the large dictionary only for the 90 individually improving files
saved 21,871 B gross, less than the dictionary itself. No dictionary support or
format complexity should be added unless a materially different payload layout
is benchmarked.

### Sprite format candidates

The earlier measurements remain decisive: the original RLE/VQ bytes plus zstd
beat per-sprite PNG/QOI/AVIF/JXL, whole-character JXL and lossless WebP atlases,
palettes, exact mirror deduplication, and canvas-aligned XOR deltas. The next
experiments should
therefore use whole-RHS or whole-character samples and include decoder/code-size
and GPU-memory costs:

- **Lossless WebP atlas:** now measured and rejected. RobinTown's 7,584 unique
  frames are 2.95 MiB as current RLE/VQ + zstd, versus 3.84 MiB as either an
  exact keyed-RGB or alpha-cleared RGBA WebP atlas (**1.30x larger**). This is
  before charging for atlas coordinates, a Rust decoder or asynchronous browser
  image plumbing, and the much larger decoded atlas. The benchmark uses
  libwebp's lossless mode, method 6, and `exact=true` through ImageMagick.
- **Near-lossless WebP / quantized RLE-VQ:** only if small color changes are
  acceptable. Compare frame-edge halos and the green transparency key, not only
  aggregate SSIM. A palette or endpoint quantizer applied *inside* the existing
  RLE/VQ representation is more promising than replacing its spatial model.
- **Basis Universal ETC1S/UASTC in KTX2:** useful mainly for reducing resident
  GPU texture memory and upload cost. It can transcode to BC/ETC/ASTC depending
  on the adapter, but WebGL exposes those formats through optional extensions
  and fallback devices need RGBA. It also wants atlas-oriented rendering and a
  transcoder in the wasm/JS payload. Benchmark it only after the sparse-bank
  work, and count the fallback plus transcoder. It is not expected to beat the
  current representation for network transfer of small pixel-art frames.
- **GPU atlases without a new transport codec:** potentially useful after load.
  Keep RLE/VQ+zstd on the wire, decode an action/character on demand, pack it
  into a texture atlas, then release its CPU pixels. This separates the proven
  transport format from a renderer optimization and avoids paying an RGBA atlas
  for sprites never drawn.

Primary references for the candidates: the
[WebP lossless bitstream specification](https://chromium.googlesource.com/webm/libwebp/+/refs/heads/main/doc/webp-lossless-bitstream-spec.txt),
[Basis Universal transcoder documentation](https://github.com/BinomialLLC/basis_universal/wiki/How-to-Use-and-Configure-the-Transcoder),
[Khronos WebGL S3TC extension](https://registry.khronos.org/webgl/extensions/WEBGL_compressed_texture_s3tc/),
[WebGPU feature guarantees](https://gpuweb.github.io/gpuweb/#adapter-capability-guarantees),
and the [zstd dictionary API](https://facebook.github.io/zstd/zstd_manual.html#Chapter5).

## Schema-v6 web audio (superseded, 2026-08-28)

This section records the intermediate eager-audio design and its measurements.
Schema v8 below is authoritative for the current browser layout: do not use the
v6 boot/mission totals to estimate current transfers.

Browser shipping now uses `convert_datadir --audio-format opus`. FFmpeg's
libopus encoder runs offline with 20 ms VBR frames and complexity 10: localized
exclamations and dialogue use 24 kbit/s `voip`, ordinary effects use 48 kbit/s
`audio`, and music uses 64 kbit/s `audio`. Native and Android conversion keeps
the default `source` representation; this is intentionally a web-only codec
change.

FFmpeg randomizes Ogg stream serials, so the converter parses its output and
remuxes the Opus packets with a fixed serial and canonical `OpusTags`. This is
required for reproducible content hashes and useful `--resume` behavior. Menu
audio is part of the shipping boot manifest rather than the wasm executable;
mission effects, actor voices, dialogue, and music remain independently loaded
dependencies. Dialogue WAVE-table references are resolved per `.red` mission
descriptor, fixing the earlier omission of later-mission
`Data/Text/Dialogues/*.ogg` files. H01 has no descriptor dialogue and is
unchanged by that particular correction.

Wasm no longer includes Kira, CPAL, or a Rust audio decoder. It calls Web
Audio's `decodeAudioData` at the asynchronous boot and mission boundaries and
keeps decoded PCM exclusively in browser-owned `AudioBuffer`s. Encoded Opus
stays once in the mounted VFS bundle; the engine sound cache retains an empty
loaded sentinel plus encoded size and duration instead of cloning the bytes.
Decode concurrency is bounded to eight and appears as its own loading-screen
component. Legacy `.wav`/`.ogg` names resolve the corresponding `.opus` key.

Each boot/mission payload records the exact source duration before transcoding.
Gameplay timing therefore does not depend on Opus pre-skip, resampling, end
trimming, or browser rounding. This changes the top-level shipping schema to
v6 (`RHDDNAT6`) and mission chunks to v3 (`RHMISN03`); older generated data
must be rebuilt. The two Ogg/Theora cinematics still contain their original
Vorbis tracks because changing them is a separate video-remux pipeline task.

The completed raw-map full-game conversion measures `H01_Lin_VL` as follows.
This is directly comparable to the schema-v5 raw-map numbers above; production
JXL changes the map-heavy mission core but not these audio totals.

```
boot manifest                 9,653,475 B
mission core                 18,363,808 B
55 parsed RHS chunks         27,093,080 B
shared effects                6,644,383 B
13 voice chunks               2,928,975 B
mission music                 1,573,054 B
exclamation metadata              1,454 B
mission boundary total       56,604,754 B (72 files)
boot + first mission         66,258,229 B
```

The mission boundary is 65,878,381 B smaller than schema v5 (-53.8%). The
parsed mission/RHS data is effectively unchanged; nearly all of the reduction
is the authoritative audio changing from 77,026,200 B to 11,147,866 B. The
boot manifest grows by 383,639 B because it now owns the transcoded menu audio
that the publish workflow previously shipped as a separate eager preload.

With the same shell accounting as the schema-v5 browser measurement, the
optimized raw-map cold load through the first mission is **79,926,218 B**:
12,746,082 B wasm, 164,302 B wasm-bindgen JS, 486,683 B core overlay, 3,067 B
preload manifest, 267,855 B shell, and the 66,258,229 B boot/mission data above.
That is 66,401,671 B smaller than the previous 146,327,889 B total (-45.4%).
HTTP content encoding can further reduce the wasm/JS/shell portion; the
already-zstd-compressed data and Opus streams should not be counted on for a
similar secondary reduction.

## Schema-v8 lazy web audio and exact dependencies (2026-08-28)

Schema v8 (`RHDDNAT8`) replaces eager boot/mission Opus payloads with a catalog
in `datadir.bin`. Each catalog entry maps a normalized legacy path to a
content-addressed `Data/audio/assets/<sha256>.opus`, its encoded size, and the
source-authoritative duration. The browser fetches an asset only at first
playback and passes its JavaScript `ArrayBuffer` directly to
`decodeAudioData`; encoded audio and decoded PCM never enter wasm memory.
In-flight requests are deduplicated by content URL. Native and Android
`--audio-format source` output remains embedded and does not require Web Audio.

Mission references now select the exact ambiance/day map and minimap, loading
PAK, physical character RHS set, and RobinHood/RobinTown variant. Terrain is a
shared content-addressed dependency instead of being embedded in each mission
core. The production browser recipe is q80 JXL, 24 kbit/s voice, 48 kbit/s
effects, 64 kbit/s music, and zstd window log 30.

The full-game `H01_Lin_VL` artifact and a fresh-profile Chrome run measured:

```
wasm gzip + bindgen JS gzip        4,633,216 B
boot datadir                       9,352,150 B
required overlay assets             201,506 B
boot game payload                 14,186,872 B

59 blocking mission files         26,142,522 B
audio played through startup       1,322,900 B (2 unique requests)
boot + mission + played audio     41,652,294 B
```

The single-file shell is 14,522 B raw / 5,550 B gzip. Adding its gzip body and
the 58-byte build pointer plus 1,731-byte preload manifest gives a production
body total of **41,659,633 B** through first-mission startup, excluding HTTP
headers. Mission loading reached 59/59, installed `H01_Lin_VL`, initialized all
portrait/action/fighting caches, and began replay recording without a panic or
runtime exception. Audio is timing-driven; later dialogue, effects, or voices
add only the standalone files actually played.

## Reproducing

```
# build
cargo build --release --example sprite_size_bench --example datadir_breakdown

# winners only, small demo datadir
cargo run --release --example sprite_size_bench -- \
    --data-dir datadirs/demo_leicester_ecoste \
    --anim-samples 8 --max-maps 4

# whole-character + per-patch JXL bucket bench
cargo run --release --example sprite_size_bench -- \
    --data-dir datadirs/fullgame_gog \
    --skip-maps --skip-sprites \
    --whole-character RobinTown --whole-character LittleJohn \
    --sprite-breakdown --whole-bank

# full sweep including the losers (slow)
cargo run --release --example sprite_size_bench -- \
    --data-dir datadirs/fullgame_gog \
    --anim-samples 10 --max-maps 4 \
    --all-codecs --av1

# convert + inspect a split shipping datadir with JXL maps
cargo run --release --bin convert_datadir -- \
    --input datadirs/demo_leicester_ecoste --output /tmp/ship-q90 \
    --format shipping --map-format jxl-q90 --zstd-window-log 30
cargo run --release --example datadir_breakdown -- /tmp/ship-q90/Data/datadir.bin
```

## Sprite research: VQ structure, cross-variant coding, context modeling (2026-08-28)

A research pass on shrinking the RHS sprite corpus further for web delivery,
prompted by the SOG v2 gaussian-splat format
([playcanvas/splat-transform#38](https://github.com/playcanvas/splat-transform/issues/38):
k-means codebooks + label images + byte-plane splitting) and Meta's
[OpenZL](https://engineering.fb.com/2025/10/06/developer-tools/openzl-open-source-format-aware-compression-framework/)
format-aware framework (field extraction, tokenization, transpose, delta).
Tool: `crates/robin_rs/examples/sprite_compression_probe.rs`. Data:
`datadirs/fullgame_linux`. Compressors: zstd 1.5.7, xz 5.8.1, bzip2, cjxl
0.12.0, libwebp 1.5.0 (magick), ffmpeg 7.1.5 libaom/FFV1. Wasm compatibility
and decode time were deliberately out of scope for this pass.

**Headline: the character corpus (161.2 MB under today's zstd-analog) measures
at 73.8 MB (2.19x) with a context-model coder plus cross-variant coding —
without touching a pixel. All numbers below are lossless.**

### What a character chunk actually is

Character sprites are 100% vector-quantized — no RLE at all (RLE lives only in
patches/accessories/UI). Each character has exactly one `FrameDictionary` with
4096/4096 entries used; tiles are 4x1 pixels, so a sprite is a `(w/4) x h`
grid of 12-bit tile indices stored in u16 words.

```
character    unique  avg dims  opaque  colors  packed      = index words
RobinTown      7584   37 x 56   43.6%    2596   7.83 MB      3,917,240
Knight01       4352   80 x 97   47.8%    1809  16.45 MB      8,222,780
Guard A00      5072   53 x 69   29.8%    1652   8.63 MB      4,312,900
```

A character is 2 MB+ compressed simply because every action x frame x 16
directions is pre-rendered: 4-8k unique frames each. At ~1 bit/pixel the
current format is respectable — but zstd turns out to sit almost exactly at
the *order-0* entropy of the tile-index stream, i.e. LZ extracts nothing from
the 2-D grid structure.

### Variant families: not recolors, but tile-predictable

48 of 117 characters form palette-variant families (Archer00-05,
Crossbowman00-05, Guard A/B 00-05, Knight01-03, Officer02-05, Officier B00-04,
Soldier A/B 00-05). Structure findings:

- Variant RHS metadata is byte-identical apart from the frame-id tables, and
  frames pair positionally 1:1 with **zero** dimension mismatches. The split
  chunk payloads are even byte-identical in *size* (all three knights:
  16,637,473 B).
- They are **not** palette swaps: ~70% of opaque pixels differ (variants were
  re-rendered from re-textured 3D models; lighting/dither diverge per pixel).
  A best-fit global color LUT leaves 17-34% of pixels wrong, single colors
  fanning out to hundreds of targets. Correspondingly, zstd-22 over the
  concatenated knight family finds ~no cross-variant matches (49.9 MB -> 12.91
  MB vs 12.98 MB compressed separately).
- But at *tile-symbol* level the mapping is nearly functional — variant B's
  tile id is almost determined by base A's tile id at the same position:

```
conditional entropy of variant given base   bits/tile   bytes   (standalone zstd)
Knight02 | Knight01 tile                        0.958   984 KB   (4.40 MB)
Knight02 | Knight01 tile + above                0.546   562 KB
Guard A01 | Guard A00 tile                      0.757   408 KB   (2.39 MB)
Guard A01 | Guard A00 tile + above              0.306   165 KB
```

The `| A-tile` rows use only 4096 contexts over 4-8M samples, so they are
robust, not overfit. An honest adaptive simulation (PPMC escapes, online
learning, chain (A-tile,above) -> A-tile -> above -> order-0) confirms:
Archer01 codes at 512,825 B against Archer00 vs 2,338,144 B standalone zstd19
(4.56x).

### Context-modeling headroom (standalone characters)

Conditional entropies of the tile-index grid, and a realistic adaptive PPM
simulation (single pass, all learning cost included, no mixing/exclusion):

```
RobinTown (3.92M tiles)     bits/tile     bytes
order-0                         6.139   3.01 MB   <- zstd-22 achieves 3.09 MB
| left                          4.745   2.32 MB
| above                         3.610   1.77 MB   (4k contexts, robust)
| left+above                    1.806   0.88 MB   (493k contexts, partly overfit)
adaptive PPM sim                4.350   2.13 MB   (-31% vs zstd)

Knight01: order-0 5.90 MB, |l+a 1.35 MB, PPM sim 3.06 MB (-31%)
Guard A00: PPM sim 1.69 MB (-31%)
```

The naive PPM already beats zstd-22 by ~31%; proper context mixing/SSE should
land -40..50%. The 2-D structure zstd cannot see is the entire opportunity.

### Transform matrix (RobinTown, one zstd-22 --long=30 frame unless noted)

OpenZL-style format-aware splits, hand-rolled:

```
baseline (w,h,dict,len,packed AoS)      3,089,401 B   (raw 7,910,320)
baseline xz -9e                         2,849,832     -7.8%
baseline bzip2 -9                       3,051,517     -1.2%
SoA field split                         3,087,904     -0.05%
vq_idx lo/hi byte planes                3,609,534     +17%  WORSE
vq up-delta (numeric, per column)       4,062,663     +31%  WORSE
freq-ranked dict permutation + planes   2,928,451     -5.2%
freq-ranked + xz -9e                    2,811,644     -9.0%  best "no new codec"
freq-ranked then delta                  3,691,696     +20%  WORSE
```

Lessons: tile ids are nominal symbols — numeric deltas and byte planes destroy
the exact-match structure LZ uses. A frequency-ranked dictionary permutation
(free at conversion; dictionary ships reordered) is the only transform that
helps zstd/xz, and xz is consistently ~8% ahead of zstd on this data. The
dictionary itself is noise (32 KB raw -> 23 KB); headers are trivial.

### Direction/frame-interleaved layouts and video codecs

Tested the "merge 16 directions as 4x4 blocks + exploit frame-to-frame
similarity" idea end to end: per-action sheets (16 directions across, frames
down, aligned via script offsets on a common canvas) for image codecs, and a
constant-size rawvideo stream (each video frame = 4x4 grid of the 16
directions, actions concatenated, 909 frames of 512x528 for RobinTown) for
video codecs:

```
RobinTown, all lossless                 bytes      vs 3.09 MB baseline
sheets JXL 0.12 -d0 e7 RGB          22,273,671     7.2x worse
sheets JXL RGBA-keyed               21,332,359     6.9x worse
sheets WebP m6 exact                 8,965,012     2.9x worse
aligned raw565 stream zstd-22        4,529,862     1.5x worse  (491 MB raw)
aligned raw565 stream xz -9e         4,593,752     1.5x worse
aligned raw565 stream bzip2          11,052,832    3.6x worse
video FFV1                          51,486,953     17x worse
video AV1 lossless (libaom,
  enable-palette + enable-intrabc)  45,891,485     15x worse
```

Conclusively negative: even with screen-content tools and inter prediction
across the direction grid and time, pixel-domain codecs cannot exploit the
similarity, because adjacent directions/frames diverge in nearly every opaque
pixel (same root cause as the recolor finding). The similarity that actually
exists is at tile-symbol level, where the CM results above capture it far more
cheaply. This closes the layout/atlas/video line of inquiry with data.

### Corpus projection (all 117 Characters/*.rhs)

`--corpus` codes every character: standalone PPM for family bases and
non-family characters, cross-variant PPM against the family base for the 39
variants (9 families detected by name):

```
                                packed        zstd19        cm/cm2
39 family variants          (89,803,350)  89,803,350 -> 24,091,234   3.73x
78 standalone VQ characters              71,375,290 -> 49,670,100   1.44x
RLE-only accessories/relics                  62,902 (kept at zstd)
TOTAL                      464,511,438  161,241,542 -> 73,761,334   2.19x
```

This projects the character RHS corpus at **46% of today's size** with a
first-generation coder, before context mixing, before touching the ~106
animation/patch RHS chunks (RLE-domain; the same context-modeling approach
applies to their pixel streams but is unmeasured), and fully lossless.

### Recommendations

1. **Free win now:** frequency-rank dictionary permutation at conversion time
   (-5% zstd, -9% with xz). No decoder change beyond using the shipped
   reordered dictionary.
2. **Cheap win:** switch RHS chunk entropy stage from zstd to LZMA/xz (-8%).
   Pure-Rust decode exists (`lzma-rs`); decode-speed budget deferred by scope.
3. **The real win:** a small rANS/arithmetic coder over tile indices with
   context (above, left) — measured -31% naive, -40..50% expected with
   mixing — plus cross-variant coding for the 39 family variants (3.73x on
   that half of the corpus). Family variants add a chunk dependency edge
   (variant chunk requires base chunk); content-addressed fetching and
   caching already support multi-chunk closures.
4. **Do not pursue:** image/video codecs on any sprite layout (JXL/WebP/AV1/
   FFV1 all lose 3-17x), byte planes, numeric index deltas, per-pixel color
   LUTs for variants, bzip2.
5. **OpenZL**: its transform vocabulary is exactly what was hand-tested here;
   the measured winners (tokenization already inherent, rank permutation) are
   simple enough that pulling in the native framework is not warranted for this
   one fixed format. Revisit if many more structured formats need the same
   treatment.

### Reproducing

```
cargo build --release --example sprite_compression_probe
target/release/examples/sprite_compression_probe --data-dir datadirs/fullgame_linux \
    --stats RobinTown --recolor Knight01:Knight02 --entropy RobinTown --cm RobinTown \
    --entropy2 'Guard A00:Guard A01' --cm2 Archer00:Archer01 --corpus
# stream/atlas file emission for external compressors:
target/release/examples/sprite_compression_probe --data-dir datadirs/fullgame_linux \
    --streams RobinTown --atlas RobinTown --out /tmp/sprite_streams
```

The shell drivers for the external-compressor sweeps (zstd/xz/bz2 combos,
cjxl/webp sheets, FFV1/AV1 video) are `scripts/sprite_compress_streams.sh` and
`scripts/sprite_compress_atlas.sh`; they only need the emitted stream/atlas
files and standard CLIs (`CJXL=<path>` to point at a static cjxl 0.12 binary).

## Implementation: sprite_codec + dictionary ranking (2026-08-28, same session)

Follow-up to the research section above: both wins are now implemented.

### Small win, shipped: dictionary rank permutation in the converter

`convert_datadir --rank-dictionaries` (default **on**; `=false` to disable)
counts how often every dictionary entry is referenced across the whole bank,
reorders each dictionary so the most used tile is index 0, and rewrites every
VQ sprite's packed indices through the same map. A consistent permutation is
invisible to the decoder — no runtime change at all.

A/B on `demo_leicester_ecoste` (`--map-format raw --zstd-window-log 30`):

```
                       no rank         ranked
Data/ total         51,169,682     50,397,716   -1.5%
Data/rhs bucket     26,703,578     25,943,258   -2.85%
```

Verified end to end with `sprite_compression_probe --verify-shipping`: all 52
chunks, 65,058 sprites (64,414 VQ), 146,584,025 pixels decode identically to
the source bank in both variants.

Two pre-existing demo-conversion bugs surfaced and were fixed along the way
(current main could not convert `demo_leicester_ecoste` at all): the boot
manifest's all-profiles character index `bail!`ed on CPF profiles whose RHS
(MerryMan gang) or exclamation samples (`X_PC_MA_*.wav`) are absent from the
demo datadir. Mission-authored requirements stay strict; index-only profiles
now warn and are omitted from the manifest.

### Big win, implemented as a library: `robin_assets::sprite_codec`

Adaptive context-model codec for VQ tile-index grids. Entropy stage is an
LZMA-style range coder — deliberately *not* rANS: rANS emits symbols LIFO,
which fights adaptive models (the decoder must replay updates in encode
order); a FIFO range coder pairs with adaptation naturally. Model: PPM escape
chain with PPMC escapes, full exclusion, per-context count halving.

```
standalone: (above, left) -> above -> left -> order-0 -> uniform
vs base:    (base, above) -> base -> above -> order-0 -> uniform
```

`--code` / `--code2` in the probe run the real codec against the bank and
verify the roundtrip bit-exactly. Real measured sizes (fullgame_linux):

```
                          zstd reference  real codec    bits/tile
RobinTown standalone      3,089,401 z22   2,072,062     4.23   (-33%)
Knight01 standalone       4,504,233 z19   2,984,055     2.90   (-34%)
Guard A00 standalone      2,389,774 z19   1,639,664     3.04   (-31%)
Knight02 vs Knight01      4,472,735 z19     977,814     0.95   (4.6x)
Guard A01 vs Guard A00    2,341,539 z19     466,297     0.86   (5.0x)
Archer01 vs Archer00      2,338,144 z19     490,056     1.31   (4.8x)
```

Model experiments, all measured on real data (kept ✓ / rejected ✗):

```
✓ above before left in the fallback chain      -3% vs left-first
✓ PPM exclusion (stamp-set, O(1))              -3%; bit-identical output to
                                               naive exclusion, 7x faster
✗ faster adaptation (count increment 4)        +4..9% — streams are stationary
✗ order-3 context (diag / base+above+left)     ±1% wash, more memory/time
✗ PPMD-style escape (distinct/2)               +1.3..1.9% standalone,
                                               -0.3..0.5% variants: net loss
✗ LZMA-style match layer (adaptive "equals     +2.6..11.6% — the (primary,
  above/base?" bit with run context, then       second) contexts already code
  PPM with the predictor excluded)              the identity case with more
                                                specificity than any flat
                                                match-bit context; the CM
                                                subsumes copy-above/copy-base
```

Note on layering: the codec's range-coded output is effectively
incompressible, so an outer zstd/xz pass is a no-op — all composition has to
happen inside the model (transforms feeding contexts), not behind it.

Speeds are research-grade and untuned (RobinTown: enc ~4s, dec ~9s; decode
optimization deferred by scope — the escape path rescans large order-1
contexts linearly).

### Real-codec corpus result (replaces the simulation estimate)

`--corpus` now encodes every character with the real codec (39 variants
against their family base):

```
                              packed        zstd19       sprite_codec
39 family variants                       89,803,350  ->  23,053,518   3.90x
78 standalone characters                 71,375,290  ->  48,130,326   1.48x
TOTAL                    464,511,438    161,241,542  ->  71,183,844   2.27x
```

### Shipping integration design (shipped as schema v9 — see next section)

- Chunk payload: per-RHS `ShippingSpriteBank.sprites` keeps `(bank_id, w, h,
  dictionary_index)` rows, but VQ `packed_data` moves into one
  `sprite_codec::encode_grids` blob per chunk (grids in `bank_id` order).
  RLE sprites keep raw `packed_data` (they are the tiny minority in RHS
  chunks and live mostly in patch/animation chunks).
- Cross-variant chunks: family membership is detected at conversion (name
  stem + verified positional pairing, as in the probe); a variant chunk
  records `base_rhs: String` plus per-sprite base bank ids, and its blob is
  encoded with `base` slices. The mission dependency closure gains a
  variant->base edge so the base chunk downloads and decodes first; the
  content-addressed fetch/cache layer already handles multi-chunk closures.
- The boot manifest keeps the (now rank-permuted) dictionaries; alphabet for
  each chunk's codec = its dictionary's `num_entries()`.
- Decode order inside a chunk is deterministic (bank-id order), so the
  decoder needs no per-sprite framing — dims come from the sprite rows.
- Expected effect at H01 scale: the measured 27.1 MB RHS closure shrinks to
  roughly 12-13 MB; fullgame character corpus 161 MB -> ~71 MB.
- Decode speed must be optimized before shipping (currently ~0.5M tiles/s;
  worst single chunk Knight01 ~15s): candidate fixes are cum-frequency
  skip structures for big contexts, capping order-1 context sizes, and
  move-to-front symbol lists. Deferred by scope this session.

### New probe modes (reproduction)

```
target/release/examples/sprite_compression_probe --data-dir datadirs/fullgame_linux \
    --code RobinTown --code2 Knight01:Knight02 --corpus
# verify a converted shipping tree pixel-for-pixel against its source bank:
target/release/examples/sprite_compression_probe \
    --data-dir datadirs/demo_leicester_ecoste --verify-shipping /tmp/ship/Data
```

## Codec model research round 2: SEE, mixing, perf (2026-08-29)

Continued experiments on `sprite_codec` (all real coder, bit-exact roundtrips):

```
✓ SEE (secondary escape estimation): adaptive escape mass bucketed by
  (chain level, log2 distinct, log2 sum, top-symbol skew quartile) replaces
  PPMC's fixed "escape = distinct" mass.
      RobinTown  2,072,062 -> 1,994,346  (-3.8%; -35.4% vs zstd-22)
      Knight01   2,984,055 -> 2,892,434  (-3.1%)
      Guard A01    466,297 ->   458,952  (-1.6%)
      Knight02     977,814 ->   968,626  (-0.9%)
  Bucketing matters: level+distinct alone REGRESSED variants (+3.3%);
  adding the skew quartile fixed standalone; adding log2(sum) (context
  maturity) made it a win everywhere.
  Corpus with SEE (--corpus, cm2 for the 39 family variants):
      TOTAL  464,511,438 packed  161,241,542 zstd19  ->  68,957,409
      (2.34x vs zstd-19; was 2.27x with PPMC escapes)

✗ Context mixing (PAQ-lite prototype, --mix in the probe): 12-bit indices
  binary-decomposed MSB-first; per-bit logistic mix of hashed order-2 /
  order-1(above) / order-1(left) / order-0 count-based predictors with
  agreement-bucketed adaptive weights. Exact cost accounting.
      RobinTown  2,017,059  (+1.1% vs PPM+SEE)
      Knight01   3,081,887  (+6.5%)
  (A first shift-counter version was +7..14%.) The PPM's exact-keyed
  order-2 contexts and symbol-level exclusion beat hashed bitwise models;
  closing the gap would need exact keys + SSE + more models, plus a
  fixed-point mixer for cross-platform determinism. Not pursued.

Decode-speed work (measured under background load; re-verify when quiet):
frequency-bubbled symbol lists + early-terminating fast paths + single-scan
exclusion + foldhash: Knight01 decode 13.4s -> ~6.5s, bytes unchanged.
A dense flat-count/Fenwick context representation regressed (hot contexts
are skew-dominated; the bubbled head answers in one cache line) and is kept
behind a disabled PROMOTE_AT. Exclusion remains the main decode cost
(~2x over no-exclusion for ~3% ratio); revisit only when decode time
becomes a shipping constraint — chunk-level parallel decode at install is
the cheaper lever.

### RDO tile assignment: closed (2026-08-29, subagent)

Tested whether re-pointing grid tiles at identical/near-identical dictionary
entries reduces entropy (`sprite_probe_rdo.rs`). The premise is false for
this data: the original VQ quantizer produced clean dictionaries — RobinTown
0 / Knight01 1 / Guard A00 0 duplicate entries (lossless canonicalization:
exactly 0 bytes), and <3% of tiles have any neighbor within max-channel
delta 2 (transparent/shadow keys exact-match only). Greedy RDO with the real
codec: eps=1 -0.003..0.031%, eps=2 -0.089..0.252% (11 KB across three
characters), visually indistinguishable in side-by-side renders but noise at
corpus scale. Not productionized; k-means dictionary re-quantization is
capped by the same histogram at ~3% of entries and was not pursued.
## Shipping integration: schema v9 (2026-08-29)

The design above is wired into the shipping datadir format (v8 shipped audio
splitting in the meantime, so this landed as **v9**: `RHDDNAT9`, mission
chunks `RHMISN04`; either side mismatching fails loudly, bitcode is not
self-describing).

Format changes (`robin_assets::shipping_datadir`):

- `ShippingSpriteBank` gains `vq_chunks: Vec<SpriteVqChunk>`. Each converted
  RHS chunk stores its well-formed VQ sprites' index grids in one
  `sprite_codec::encode_grids` blob (bank-id ascending order); those sprite
  rows keep `(bank_id, w, h, dictionary_index)` but ship empty `packed_data`.
  RLE sprites — and the rare VQ sprite whose packed length disagrees with its
  `(w/4) x h` grid — keep raw packed words. `SpriteVqChunk` records the
  encode order (`sprite_ids`), the codec `alphabet` (max `num_entries()` of
  the dictionaries involved), per-sprite base bank ids for cross-variant
  coding, and the source/base RHS rels for diagnostics.
- Conversion (`convert_datadir --format shipping`) detects variant families
  among `Characters/*.rhs` (trailing-two-digit stem, >1 member, base =
  lexicographically first — the probe's corpus rule), pairs base/variant
  sprites positionally over the full-profile script frame-id order, and codes
  each variant chunk against its base's rank-permuted grids. Pairs whose
  dims/lengths mismatch code standalone; a chunk that pairs poorly (>10%
  unbased) falls back to standalone entirely. The base grids a variant needs
  are added to the base chunk — synthesized as a sprite-only chunk when no
  mission requires the base RHS itself — and every dependency list that names
  a variant chunk (mission files, `character_rhs_files`,
  `saved_world_rhs_files`) also names its base chunk.
- Runtime: `ShippingSpriteBank::materialize_vq_chunks` decodes the blobs back
  into per-sprite packed data at `install_mission` time, after all mission
  parts merged (wasm fetches complete out of order, so materialization
  iterates to a fixpoint instead of assuming an install order). A variant
  chunk whose base sprites never materialize is a hard error naming the
  missing base RHS. Downstream (`FrameHolder::load_from_shipping`, renderer,
  savegames) is unchanged.

Measured on `demo_leicester_ecoste` (`--map-format raw --zstd-window-log
30`), against the schema-v8 ranked numbers above:

```
                        v8 (ranked)        v9        delta
Data/ total             50,397,716    43,070,372    -14.5%
Data/rhs bucket         25,943,258    18,617,203    -28.2%
```

`--verify-shipping` on the converted tree: 52 chunks (31 VQ blobs,
16,796,145 blob bytes), 65,058 sprites (64,414 VQ), 146,584,025 pixels — all
identical to the source bank. The demo has no complete variant families
(`Archer01` without `Archer00`, …), so this is pure standalone context
modeling; the 3.9x family-variant multiplier applies at fullgame scale, where
the cross-variant path is exercised (unit tests cover the merge/materialize
order and missing-base error paths). Decode of the whole demo corpus took
~110 s single-threaded at this baseline — the decode-speed optimization
listed in the design section remains the open item before wasm shipping.

## Parallel research results (2026-08-29, subagents)

Four parallel investigations; full data in each probe example.

### Family base topology (`sprite_probe_experiments.rs --topology`) — SHIPPED

Full pairwise real-codec matrix over all 9 families: the lexicographically
first member is the best star base in only 1 of 9. Best-base stars total
37,286,493 B vs the naive 38,861,886 B (-4.05% of the family corpus,
~1.6 MB fullgame). "01"/"02" members are the natural hubs; "04" members are
uniformly poor bases (smallest standalone entropy, worst predictors).
Chains beat the best star only for Officier B (-0.85%) and couple the
dependency closure, so star stays. The converter now selects the base per
family via a sampled conditional-entropy proxy (H(base|above) +
sum H(member|base)) instead of name order.

### Sprite coding order (`--order`) — closed

Animation-script first-occurrence order is +0.05..0.14% WORSE than bank-id
order on all three test characters. The model's information is the 2-D
neighborhood, not stream position (mirrors the zstd reorder non-result).
Bank-id order stays (and needs no permutation metadata).

### Mirrored-direction prediction (`--mirror`) — closed

Direction d vs 16-d: only 4.4% (RobinTown) / 12.9% (Knight01) of opposite
pairs even share dimensions (independent cropping), and on that favorable
subsample the mirror context is ~1 bit/tile WEAKER than the plain above
context (3.41 vs 2.44 b/t Knight01; 3.78 vs 2.73 RobinTown) despite 33-46%
of tiles mirroring exactly: directional lighting breaks bilateral symmetry
(same root cause as the recolor/video negatives).

### RLE bucket context modeling (`sprite_probe_rle_dict.rs --rle`) — closed

The RLE bucket is 10,134 sprites / 66.8 MB raw, dominated by the 116
Data/Animations RHS. Pixel-domain PPM (left/above contexts): 16.65 MB total
vs zstd-19 17.69 MB (-5.9%) but xz -9e 15.56 MB (-12.1% vs zstd) BEATS the
CM by 7%: animation frames carry real LZ matches an order-2 neighborhood
model can't see, and the literals are high-entropy dither (4.1 bits/px).
Actionable: an xz/LZMA entropy stage for animation/patch chunks (~2.1 MB)
instead of a bespoke pixel CM. Also surfaced: the animation RHS files carry
8,023 VQ sprites (65 MB packed) — schema v9's generic chunk path already
blob-codes those.

### Family-shared dictionaries (`--dict`) — closed

Family dictionaries are essentially disjoint: 0.7-3.8% exact tile overlap
with the base, 75-85% of the rest >= 3 channel-steps away. Unified-id
cross-variant coding is uniformly +0.07..0.31% worse (near-pure permutation,
which the PPM is invariant to, plus a bigger alphabet); shared-dictionary
storage saves only ~6 KB raw per family. No format change.

### Schema v9 + SEE, integrated demo numbers

After merging the v9 wiring with the SEE codec (`demo_leicester_ecoste`,
raw maps, windowLog 30): Data/ 51,169,682 (v8 no-rank) -> 50,397,716 (v8
ranked) -> 43,070,372 (v9, PPMC codec) -> **42,422,327 B (v9 + SEE)**;
VQ blob bytes 16,796,145 -> 16,148,418 (-3.9%); corpus decode 110 s -> 55 s
with the fast paths. verify-shipping: all 65,058 sprites / 146,584,025
pixels identical to the source bank. The demo carries no complete variant
families; fullgame conversion exercises the cross-variant path.

## Sibling-context coding and the cluster negative (2026-08-29)

✗ Tile-similarity cluster contexts (a (cluster(primary), cluster(second))
level between the exact pair and the order-1 fallbacks, clusters derived
from dictionary colors): RobinTown +1.4%, Knight01 +2.2%, Guard A01 vs base
+25%. Same failure mode as order-3: a mid-strength level inserted into the
escape chain delays stronger fallbacks at real escape cost. Reverted.

✓ Two-predecessor ("sibling") coding: a family member with two already-
decoded siblings codes through (b1,b2) -> (b1,above) -> b1 -> above ->
order-0 (`encode_grids_multi`). Ships zero extra bytes — the decoder holds
both predecessors via dependency edges. Conditional-entropy pricing
(--entropy3) promised ~2x; the real chain (--code3) delivers:

```
                         one base        two bases
Guard A02 (vs A00+A01)     515,795 ->     401,680   -22%
Archer02  (vs 00+01)      ~515,000 ->     385,605   -25%
Knight03  (vs 01+02)     1,156,196 ->   1,148,686   -0.6%  (Knight02 adds
                                                    little over Knight01)
```

~30 of the 39 variants are third-or-later family members; wiring a star-2
topology into the converter (each later member coded against the two best
hubs) is the follow-up, worth an estimated ~3-4 MB on the fullgame corpus.
This supersedes the "synthetic centroid base" idea: a computed base would
have to be shipped (~a member's own coded size), canceling its gains, while
sibling contexts are free.

## Fullgame schema-v9 validation (2026-08-29)

First fullgame conversion with the complete pipeline (SEE codec, family
cross-variant coding with proxy-selected bases, rank permutation):

```
Data/rhs bucket        193,7xx,xxx (v8-era zstd chunks) -> 97,829,439 B  (1.98x)
VQ blob bytes          78,244,997 across 133 blobs (characters AND the
                       animation RHS files' VQ half, which v9 covers
                       generically)
verify-shipping        223 chunks, 402,303 sprites, 1,101,554,622 pixels —
                       all identical to the source bank (decode 232 s
                       single-threaded)
```

Proxy base selection picked the measured-best star hub in 7 of 9 families
(Archer01, Crossbowman02, Guard B01, Knight01, Officier B01, Soldier A01,
Soldier B01; Guard A05 / Officer05 diverge from the pairwise matrix's
A01/O03 but sit near-best in it). Open follow-ups: star-2 wiring
(two-predecessor coding, measured -22..25% on third-and-later members),
parallel chunk decode at install, xz stage for the RLE animation bucket.

## Temporal and cross-direction reference contexts (2026-08-29)

Measured whether previously decoded frames can serve as extra context,
aligned through the script offsets that already ship (--entropy-temporal,
--entropy-crossdir, --code-aux):

- Previous frame in the same animation row, offset-aligned: 43-68% of tiles
  match the aligned predecessor exactly; H(x|prev) 2.4-4.0 bits/tile is
  comparable to |above and largely independent of it (H(x|prev,above)
  0.86-1.42 on covered tiles). 38% of frame pairs skip because the x offset
  delta is not a multiple of the 4-pixel tile width.
- Adjacent camera direction (22.5 deg), same frame: 30-40% exact match; never
  beats |above alone. Useful only as a fallback where no temporal
  predecessor exists.
- Real codec (`encode_grids_auxref`: chain (aux, above) -> (above, left) ->
  above -> left -> order-0; aux = temporal predecessor, else adjacent
  direction, ref_id < cur_id for causality; roundtrip verified):
      Knight01    2,892,434 -> 2,793,636   -3.4%
      Guard A00   1,639,664 -> 1,555,313   -5.1%
      WillScarlet             1,631,129    (-3.4% vs its SEE standalone)
      RobinTown   1,994,346 -> 1,989,761   -0.2%
  Ordering the aux level after (above,left) measured worse (Knight +2.3%).
  The gap to the entropy table is the usual escape-chain and overfit tax.
  Zero shipped bytes; converter/schema wiring pending (fold into the next
  chunk version bump alongside star-2). Recovering the x-misaligned 38% via
  shifted-pixel-hash contexts is the known follow-up.

## Decode-speed round 2 (2026-08-29)

- Cached the range-coder division between decode_target and commit
  (bitstream identical): Knight01 decode ~6.5 -> 5.8 s.
- Capped exclusion (escaped contexts with >256 distinct symbols no longer
  feed the exclusion set; bitstream change, rides the v10 schema): decode
  -10..15% for +0.4..0.6% standalone size; variant/sibling/aux streams
  unchanged within 0.5%. Measured curve at caps 64/128/256 in the
  EXCL_SOURCE_CAP doc comment. Knight01 5.8 -> 5.5 s, RobinTown ~4.6 ->
  3.9 s (timings under background load; relative deltas from same-session
  A/B runs).
- Still open: parallel chunk decode at install (after the star-2 branch
  merges; chunks are independent), and exclusion-adjusted Fenwick coding if
  single-thread decode ever needs another 2x.
## Shipping integration: schema v10 — star-2 family topology (2026-08-29)

The two-predecessor coding above is wired into the shipping format as
**v10** (`RHDDNA10` — the tag is capped at 8 bytes, the u32 version beside
it says 10 — mission chunks `RHMISN05`; mismatches fail loudly as before).

Format (`robin_assets::shipping_datadir`): `SpriteVqChunk` gains
`base2_rhs: String` (empty = none) and `base2_ids: Vec<Option<u32>>`
(aligned with `sprite_ids`; must be empty/all-`None` when `base2_rhs` is
empty, and a `Some` requires the matching `base_ids` entry). Blobs are
`encode_grids_multi` output; single-base and standalone chunks stay
byte-identical to v9. Materialization resolves base2 grids exactly like
base grids inside the same fixpoint (order-independent); a chunk whose
declared base2 sprites are absent from the payload is a hard error naming
the missing base2 RHS.

Converter: per family, hub1 stays the proxy-selected base; hub2 = argmin
over candidates c != hub1 of sum over members m not in {hub1, c} of the
sampled H(m | c tile) pair proxy — the best *second* predictor for the
rest (logged as "selected family second base"). hub2's own chunk codes
against hub1 only. Every other member pairs positionally against BOTH hubs
and follows the probe's `code3` ladder per sprite: two aligned bases ->
one (either hub can serve as the single base) -> standalone. Hub grids a
variant references are unioned into the respective hub chunk, and the
dependency map is now multi-edge: a variant chunk pulls in hub1 always and
hub2 whenever referenced, in mission files, `character_rhs_files`, and
`saved_world_rhs_files`. Two-member families (none in fullgame) keep the
plain star-1 path; the demo (no complete families) converts byte-identically
to v9 modulo the new empty fields.

Measured, fullgame (`fullgame_linux`, raw maps, windowLog 30), against the
v9 numbers above:

```
                          v9              v10 (star-2)      delta
Data/rhs bucket           97,829,439      96,658,119        -1,171,320 B (-1.20%)
VQ blob bytes             78,244,997      77,024,677        -1,220,320 B (-1.56%)
```

All 9 families picked a hub2 (Archer01+02, Crossbowman02+03, Guard A05+01,
Guard B01+05, Knight01+02, Officer05+02, Officier B01+02, Soldier A01+02,
Soldier B01+02); 30 chunks code star-2. Knight03's chunk lands at
1,148,686 B — exactly the probe's `--code3` measurement. The corpus win is
~1.2 MB, well under the ~3-4 MB projected from the code3 samples: those
were measured against *lexicographic* single bases (Guard A02 vs A00 =
515,795 B), while v9 production already coded third-and-later members
against their proxy-selected best hub, so much of the projected gap was
already banked by hub selection. The remaining marginal value of the
second predecessor is real but smaller (Guard A02: 399,170 B here).

verify-shipping (both trees): demo 52 chunks / 65,058 sprites /
146,584,025 pixels, fullgame 223 chunks (133 blobs) / 402,303 sprites /
1,101,554,622 pixels — all identical to the source banks. The probe's
verifier additionally gained a dependency-closure check: for every mission
/ character-profile / saved-world list it asserts the listed chunks
provide every base/base2 sprite id the listed variant chunks reference
(the merged verify alone cannot catch a missing hub edge).

## Shipped self-refs, usage-weighted hubs, and the v10 browser measurement (2026-08-29)

Production wiring landed for the remaining v10 pieces:

- **Self-referential aux contexts ship at zero bytes.** Standalone chunks
  (hub or family-less) derive temporal/adjacent-direction tile predictions
  from the `RhsData` script metadata already in the payload
  (`derive_chunk_self_refs`): pass 1 links each animation frame to its
  temporal predecessor, pass 2 falls back to the same frame in an adjacent
  direction; refs must be tile-aligned (`dx % 4 == 0`) and causal
  (`ref_id < cur_id`). The decoder resolves them against its own earlier
  output, so the bitstream carries only a `self_refs` flag per chunk.
- **Family hubs are picked by mission usage.** Among members whose
  standalone (or pair) cost is within 5% of the best
  (`FAMILY_HUB_PROXY_TOLERANCE = 1.05`), the converter now picks the one
  required by the most mission builds, so first-load closures pull hubs
  the mission needs anyway instead of an unused proxy (Guard A05 → A01,
  Archer01 → Archer02, …).
- `--prune-unreferenced` (probe): `convert_datadir --resume` leaves the
  previous run's content-addressed chunks behind when content changes;
  this deletes everything no manifest list references (42 orphans,
  32.4 MB, after the hub reselection).

Fullgame web recipe (q80 JXL, Opus, windowLog 30), rhs bucket:

```
v10 star-2 (proxy hubs, no aux)    96,658,119 B
v10 + self-refs + usage hubs       94,677,379 B   (-1,980,740 B, -2.0%)
v8-era zstd chunks                193.7 MB        (2.05x overall)
```

verify-shipping: 223 chunks (133 VQ blobs, 75,044,019 blob bytes),
402,303 sprites, 1,101,554,622 pixels — identical to the source bank;
dependency-closure check green.

**H01_Lin_VL browser measurement** (fresh-profile headless Chrome,
localhost, same accounting as the schema-v8 run):

```
                                   v8 (2026-08-28)    v10 (2026-08-29)
wasm gzip + bindgen JS gzip         4,633,216 B        5,921,549 B
shell + preload manifest + overlay    208,845 B          272,776 B
boot datadir                        9,352,150 B        9,324,395 B
blocking mission files             26,142,522 B (59)  24,857,987 B (73)
audio played through startup        1,322,900 B        1,322,900 B
total through first-mission        41,659,633 B       41,699,607 B
```

The 1.28 MB blocking-set win is offset by 1.29 MB of wasm growth — a
deliberate trade, not a regression: commit 03ffb67b3 switched wasm-release
to no-LTO with robin_assets at opt-level 3 + simd128 for decode speed
(4.92 → 5.73 MB gzip on its own; the rest is the fob branch's UI/webfont
work). The mission pulls 73
files instead of 59 because family variants now ride with their hub
chunks; usage-weighted hubs removed the pure dependency tax (A05) but the
mission uses most family members anyway, so H01's closure moved only
-1.3 MB. The global rhs halving pays off on later missions and full
predownload, not mission 1.

Load time, measured for the first time (12-core desktop, software GL,
localhost so transfer is ~free): navigation → in-game (replay recording
starts) = **71.6 s**, of which 61.7 s is the single-threaded wasm VQ
context-model decode of the 68 blocking rhs chunks (fetch 5.5 s, boot
1.5 s, session setup 4.5 s). v8 was never timed, but its chunk decode was
plain zstd (the same path that inflates the 9.3 MB boot datadir in ~1 s),
so v8's equivalent figure is ~10-15 s. **Wasm decode speed is now the
gating cost of the codec**, worth roughly its own workstream: wasm
threads, further codec hot-path work, decode-cost-aware chunk format
choice (keep first-mission closures on plain zstd), or lazy/streamed
chunk install behind the loading screen.

## Decode-speed campaign: 5x, and schema v11 (2026-08-29)

Same-day follow-up to the measurement above. Native single-thread H01
blocking-set materialize (the wasm proxy; `--decode-bench`, quiet box,
interleaved mins):

```
v10 codec, v10 tree (start)                        36.0 s
+ no scratch materialization on the excl path      30.1 s
+ partition_point/rotate bubbling                  28.6 s
+ &mut chains, no re-lookup at bump                ~27 s
+ dense count mirrors (excl subtracts, O(|excl|))  18.3 s
+ schema v11: exclusion OFF (EXCL_SOURCE_CAP 0)    13.8 s   (+1.85% bytes)
+ known-outcome learning (bump_at / push_new),
  hot first-level exits, no EDGE aux entries       11.3 s
+ pre-sized context maps                           ~10.9 s
```

Everything except the v11 cap change is bitstream-exact; all steps
verified against the source bank (402,303 sprites, closure-check green;
fullgame parallel materialize 43.5 -> 7.2 s). The key structural facts:
the exclusion-path scratch copy was >half the profile; and every decoder
chain level's outcome is provable (a hit knows its list index from the
find; ANY miss proves absence, because the exclusion set can never
contain the coded symbol), so learning never rescans a symbol list.

The v11 fresh conversion also shrank the boot manifest 9,324,395 ->
7,893,113 B (41.4 -> 16.9 MB raw): the v10 tree's resume chain predated
`--interface-image-format jxl-q80`, so its manifest still embedded raw
interface images.

**H01 browser measurement, v11** (same setup as the v10 run):

```
                                   v10 (morning)      v11 (evening)
wasm gzip + bindgen JS gzip         5,921,549 B        4,862,157 B
boot datadir                        9,324,395 B        7,893,113 B
blocking mission files             24,857,987 B (73)  25,272,333 B (73)
total through first-mission        41,699,607 B       39,623,279 B
navigation -> in-game                    71.6 s             23.5 s
  of which wasm VQ decode                61.7 s             12.3 s
```

Smaller than the v8 baseline (41.66 MB) AND 3x faster to in-game than
the morning's v10. Wasm decode now runs at roughly native single-thread
speed (12.3 vs ~11 s; simd128 + O3 robin_assets, no LTO).

Remaining levers, in rough value order: wasm threads for the
materialize (12.3 -> ~2-4 s; needs an atomics build plus COOP/COEP
headers on the Cloudflare static origin); overlap chunk decode with the fetch
phase (~4 s hidden); a schema-v12 binary escape coder (the SEE
`esc_freq` 64-bit division plus the escape-side `decode_target`
division are ~2 divisions per visited level, an estimated 15-20% of
decode); SIMD/block-skip symbol scans (`find_by_target` ~10%); and the
~6 s session-setup phase, which is engine work, not codec.

## Schema v12: binary escape coding (2026-08-29)

The escape lever from the list above, shipped as RHDDNA12/RHMISN07.
Hit-vs-escape at each PPM chain level is now one LZMA-style adaptive
binary decision — an 11-bit probability per SEE bucket (same bucketing:
level, log2 distinct, log2 sum, top-skew quartile), init 1024, shift
update `p ± delta >> 5` — coded via `encode_bit`/`decode_bit`
(`bound = (range >> 11) * p`, multiply-only). On a hit the symbol is
then coded in the context's plain frequency interval over `sum` (ONE
division), skipped entirely when the context has a single candidate
(`freq == sum`); on an escape nothing further is coded at that level.
This removes both per-level escape divisions (v11: SEE `esc_freq`'s
64-bit mul+div plus the enlarged-total `decode_target` div on every
visited level).

Adaptation shift tuned on coded bytes (Knight01 + RobinTown +
Guard A00->A02 total): shift 3 = 5,449,385 B, 4 = 5,419,538 B,
5 = 5,420,021 B, 6 = 5,445,021 B. 4 and 5 within 0.01%; 5 kept (LZMA
default).

Size and speed vs v11 (same probes; decode times interleaved
old/new binaries on a loaded box, minimums):

```
                       v11             v12            size delta
Knight01               2,882,599 B     2,887,813 B    +0.18%
RobinTown              2,001,621 B     2,002,372 B    +0.04%
Guard A00->A02           522,768 B       529,836 B    +1.35%
Knight01 decode (min)  1.9 s           1.7 s          -10%
```

Fullgame web tree (fresh convert, jxl-q80 maps/interface, opus,
window-log 30): verify-shipping green — 223 chunks (133 VQ blobs,
76,656,179 blob bytes), 402,303 sprites, 1,101,554,622 pixels, all
identical to source bank, dependency closure covered. H01_Lin_VL
blocking set: 68 files, 22,859,249 B (v11: 22,809,615 B, +0.22%).
Single-thread materialize, 6 interleaved rounds v11-binary-on-v11-tree
vs v12-binary-on-v12-tree (box load 12-40 throughout, so minimums are
the honest statistic): v11 min 12.56 s, v12 min 11.52 s, pairwise
median ratio 0.90 — **~8-10% decode saved for +0.22% blocking bytes**.
Short of the 15-20% hoped for from division counting alone: the escape
bit adds a data-dependent branch per level, and the surviving hit-path
`decode_target` division was always the more predictable of the two.

## Negative result: block-skip symbol scan (2026-08-29)

Tried the other decode lever from the v11 ledger: `Ctx::find_by_target`
(the frequency-ordered interval walk, ~10% of decode) rewritten to scan
the first 8 (symbol, count) pairs element-wise, then skip whole
8-pair blocks by a branchless reduction-tree count sum
(`target >= cum + block_sum`), resolving element-wise only inside the
target's block. Identical (index, symbol, cum, count) results; all
roundtrip tests green (default and ROBIN_EXCL_CAP=256).

It measures SLOWER, and `perf stat` on the H01 single-thread
decode-bench (two samples each, v12 tree) shows exactly why:

```
                      plain walk           block-skip
instructions          66.7 / 67.0 G        54.4 / 54.4 G   (-19%)
branches              13.1 G                7.5 G          (-42%)
branch-miss rate      1.28%                2.40%
cycles                50.9 / 55.5 G        56.7 / 59.2 G   (+8-11%)
IPC                   1.20-1.32            0.92-0.96
wall (interleaved
 mins, loaded box)    12.48 s              12.83 s
```

Fewer instructions, more cycles: the plain walk's per-element exit
branch is almost always correctly predicted "keep scanning", so the
core speculates deep ahead and pipelines all the loads — the loop runs
memory-parallel. The block variant makes each skip decision depend on
a just-computed 8-count reduction (a serial chain the branch must wait
for) and roughly doubles the mispredict rate, wiping out the
instruction savings. Same physics as the 2026-08-29 dense-promotion
regression (6.4 -> 11.8 s): these contexts are so skewed that the
bubbled list answers from its cache-resident head, and any cleverness
that adds latency to the head path loses. Change dropped; the plain
walk stays.
## Lossy JXL head-to-head on large sprites: negative (2026-08-29)

Earlier rounds only closed the door on lossless image codecs and on
JXL over atlas/verbatim dumps of whole banks. Remaining open question:
the VQ pixels are already lossy (the original game's vector
quantisation), so for sprites big enough that the per-image header tax
stops dominating (>= 20x20 px), does *lossy* JXL over the decoded
RGB565 pixels beat the context-model codec? Answer: no — it loses by
3.8-5.1x at visibly degraded quality, and by 6.2x at the lossless
setting that parity actually requires.

Harness: `crates/robin_assets/examples/jxl_sprite_probe.rs` (research
example; new `png` dev-dep for the cjxl input files; needs an external
`cjxl` — v0.12.0 here — via `--cjxl` or PATH). Per RHS file it
collects the script-referenced VQ sprites, selects those >= 20x20 px,
and compares:

- codec comparator: the selected sprites re-encoded via `sprite_codec`
  as a standalone chunk with derived self-refs (`blob_sel`, the exact
  byte comparator; the whole-character `blob_all` reproduces the
  shipping chunk and agrees with the tile-prorated share within 0.2%);
- JXL side: each sprite decoded to RGB565 (Day dictionary), exported
  as RGBA PNG, `cjxl -e 7` at `-q 90`, `-q 80`, and lossless `-d 0`.
  Both key colors (transparent 0x07C0, shadow 0x001F) are excluded
  from the image (alpha 0, RGB free for the encoder), and a 2-bit
  per-pixel class map (transparent/shadow/opaque), zstd'd per
  character, is counted toward the JXL totals — keys and shadows must
  be exact, and requantised RGB has no guarantee of avoiding the key
  values, so the mask is not optional. Lossy output is decoded back
  with `jxl-rs` (the runtime's decoder), requantised to RGB565, and
  scored over opaque pixels only.

fullgame_linux, 17,390 selected sprites = 20.6M VQ tiles (>= 20x20
captures 98.5-100% of all VQ tiles per file — "large sprites" is
effectively the whole character/animation banks):

```
character                     n_sel |  codec-sel |    jxl-q90    jxl-q80     jxl-d0     mask+z
Characters/Knight01.rhs        4352 | 2752.1 KiB |  12.77 MiB 9420.4 KiB  17.05 MiB 1057.6 KiB
Characters/RobinTown.rhs       7579 | 1965.9 KiB | 9211.2 KiB 6733.4 KiB  10.72 MiB 1052.7 KiB
Characters/Guard A00.rhs       5072 | 1516.3 KiB | 7075.3 KiB 5309.7 KiB 8198.1 KiB  821.1 KiB
Animations/Day/chariot01.rhs    267 |  575.2 KiB | 2257.3 KiB 1593.8 KiB 2589.2 KiB  152.5 KiB
Animations/Day/sherwood.rhs     120 |  176.0 KiB |  708.4 KiB  457.9 KiB 1139.2 KiB     4256 B
TOTAL                         17390 | 6985.5 KiB |  31.57 MiB  22.96 MiB  39.42 MiB 3088.1 KiB
ratio vs codec-sel                  |            |      5.07x      3.81x      6.22x  (incl mask)
```

Every axis is a loss:

- **Size at lossy settings.** q80+mask is 3.81x the codec; q90+mask
  5.07x. Even the class masks ALONE cost 44% of the codec's entire
  budget for the same sprites. Per sprite: codec 411 B avg vs 1.4 KiB
  (q80) / 1.9 KiB (q90) / 2.4 KiB (d0) plus 182 B mask.
- **Quality at those settings is already bad.** Opaque-pixel PSNR at
  q90: 25.1 dB Knight01, 29.6-30.7 dB the other characters, 33-34 dB
  the two animations (worst single sprites 22.7 dB); q80 is 2-3 dB
  worse. Visually the VQ dither patterns smear into gradients (see
  the `worst_q90/` side-by-side dumps). Only 7-26% of opaque pixels
  (per file; 6.8-10.3% on characters) survive q90 with their exact
  RGB565 value — parity tests compare composited RGB565 framebuffers,
  so lossy JXL is a parity break by construction, and the setting
  that is not (`-d 0`) is 6.22x the codec.
- **Atlas packing doesn't save it.** Packing the biggest animation
  (Knight act6, 320 frames) into one grid atlas recovers ~15% of the
  per-image header tax (938.8 -> 800.4 KiB q90) but the codec does
  the same frames in 225.6 KiB — still 3.5x. Same shape on all five
  files.
- **Decode is slower, not faster.** jxl-rs single-thread decode of the
  17,390 q90 images: 16.4 s (0.4-1.2 ms/img for character sprites) vs
  4.6 s for the codec to decode the same content from the blobs — 3.5x
  slower where it hurts (wasm is single-threaded today), before adding
  RGB565 requantisation and mask application. Encode side: cjxl -e 7
  took 171 s wall on 12 threads for the sweep.
- Zero key collisions were measured (0 collisions on 24.7M opaque px,
  at both qualities), so the mask scheme works — but it never gets
  cheap.

Why it loses: the content is 4096-entry dictionary indices arranged in
grids — the codec models exactly that symbol stream with context, ~2.8
bits/tile. JXL sees the rasterised OUTPUT of that quantiser: VarDCT
spends bits re-approximating dither texture the dictionary already
paid for once, and modular/lossless has to reproduce it exactly.
Pixel-domain image codecs are the wrong model for this data at every
quality point; this closes the last JXL-for-sprites variant. (JXL
remains the right tool where it shipped: maps and interface images,
which are true continuous-tone rasters.)

Repro:

```
cargo run --release --example jxl_sprite_probe -- \
    --data-dir datadirs/fullgame_linux --out tmp/jxl_sprite_probe
# report: tmp/jxl_sprite_probe/report.txt; worst-case side-by-side
# PNGs under tmp/jxl_sprite_probe/<char>/worst_q{80,90}/
```

### Follow-up: the RLE/patch bucket — lossy JXL WINS here (2026-08-29)

Same probe, `--rle` mode, on the content class where the economics
differ: the RLE bucket has no VQ codec side (its best entropy stage so
far is xz -9e, "RLE bucket context modeling" above), and the content is
map-like art, which is what took the terrain maps down 60%. The mode
reproduces the ledger bucket exactly — 10,134 RLE sprites, raw corpus
blob 63.73 MiB (= 66.8 MB), zstd-19 16.87 MiB (= 17.69 MB), xz -9e
14.84 MiB (= 15.56 MB); the 9,516 VQ sprites in the same 150 RHS files
are v9's business and excluded. Same mask methodology, extended to four
classes: transparent/outside-run, shadow, opaque, plus "in-run literal
with the transparent-key value" — with the map, RLE run extents AND all
key literals reconstruct exactly, so lossy error is confined to opaque
RGB (0 trailing-word sprites in the bucket; some patches carry LARGE
key-literal interiors, visible magenta in the dumps).

Selected >= 20x20 px: 8,277 sprites (8,155 from the 116 animation RHS,
122 accessory) = 98.8% of bucket bytes; 62.2M canvas px, 41% opaque.

```
selected comparators |  zstd-19 16.59 MiB   xz -9e 14.62 MiB
jxl per-sprite + 865.8 KiB mask (cjxl e7):
  q90 16.18 MiB (1.16x xz) | q80 11.18 MiB (0.82x) | q70 8.93 MiB (0.67x) | d0 19.32 MiB (1.38x)
atlas per (rhs,profile,action), 383 of 569 groups >= 4 frames (8,088 frames):
  atlas       q90 11.70 MiB | q80 7985 KiB | q70 6205 KiB | d0 13.60 MiB
  per-sprite  q90 13.90 MiB | q80 9921 KiB | q70 8001 KiB | d0 15.07 MiB
  animated    q80 10.12 MiB | d0 15.19 MiB  (APNG -> cjxl frame sequence)
```

- **Per-sprite lossy already beats the best entropy coder**: q80+mask
  is 0.82x of xz -9e, q70+mask 0.67x — where the VQ characters showed
  3.8-5.1x LOSSES. No dictionary quantiser ever touched these pixels,
  so JXL is not re-buying anyone else's bits.
- **Atlases add ~20%**: unlike the VQ case, per-animation grid atlases
  recover real money (-19.5% at q80, -22.5% at q70 vs the same frames
  per-sprite) — frames are large and similar, and cjxl's patch/context
  machinery sees the repetition. Animated JXL is a bust: cjxl codes
  APNG frames essentially independently (10.12 MiB q80, WORSE than the
  8.0 MiB atlas), so atlas > animation.
- **Best measured config**: atlas-q70 (6,205 KiB) + per-sprite q70 for
  the 189 ungrouped frames (1,146 KiB) + masks (866 KiB) = **8.02 MiB
  = 0.55x of xz** (0.48x of zstd-19). Fullgame: ~6.6 MiB under the xz
  plan, ~8.6 MiB under the shipping-zstd status quo, for the cost of
  going lossy. The same config at q80 is 10.14 MiB = 0.69x xz.
- **Lossless JXL still loses** (d0+mask 1.38x xz): exact dither
  reproduction is the same bad deal it was for characters. The win is
  ONLY available by accepting lossy opaque RGB.
- **Quality**: opaque-px PSNR q90 33.3 / q80 30.7 / q70 29.2 dB.
  On the >= 200x200 subset (186 big map patches, 22.5% of bucket
  bytes — where JXL is at its best, q70+mask 0.31x xz) the WORST
  patch at q70 is 24.6 dB and visually near-clean (building roof
  texture slightly softened; invisible composited on a q80 map).
  The global worst cases are small dithered effect/pickup sprites
  (ids ~121xx, 18.4-19.8 dB at q70/q80) — visible softening, but
  these contribute almost no bytes. 565-exact opaque pixels: 17-26%,
  so this is NOT framebuffer-parity-safe (see below). Key collisions:
  0 across 25.5M opaque px at all three qualities.
- **Decode cost is the real price**: jxl-rs 8.6-9.4 s single-thread
  for all 8,277 images (1.0-1.1 ms/img, ~6.7 Mpx/s) vs 0.10 s zstd /
  0.55 s xz inflate for the same content — 15-90x slower. The bucket
  is spread across 116 mission-scoped files, so the per-mission
  increment is a fraction of that, but a wasm boot that materializes
  many missions' patches would feel it. Encode is cheap (81 s
  per-sprite + 75 s atlases on 12 threads).

Actionable: for WEB delivery, a jxl-q70/q80 atlas path for animation/
patch chunks supersedes the earlier "xz entropy stage" recommendation
(~4x the savings: ~8.6 MiB vs ~2.1 MB). Two caveats before wiring it:
(1) parity — replay/screenshot traces compare composited RGB565
framebuffers, and ambient patches appear in them; lossy patches must
be web-only or parity re-baselined; native/parity datadirs keep the
lossless path. (2) decode-time budget on wasm (above): ship atlases
per animation group so decode stays lazy per mission.

### Follow-up: loading-art `.pak` pictures (2026-08-29)

`--pak` mode. Premise correction first: with `--interface-image-format
jxl-q80` (the v11 shipping flag) BOTH fullgame paks already take the
converter's keyed-RGBA JXL path (`is_interface_path` matches
`Interface/Loading.pak` and `2047/Data/Interface/Slideshow_in.pak`);
`transcode_pak_drop_bzip` is only the raw-format fallback. So this
measurement quantifies the shipped choice and the headroom below it
(cjxl e9, keyed RGBA, exactly like the converter):

```
                       raw      zstd-max     d0     q90     q80*    q70    PSNR q80/q70
Loading.pak (3x1024x768)   4608 KiB   437.0 KiB  527 KiB  267 KiB  145 KiB  102 KiB   36.0 / 35.0 dB
Slideshow_in.pak (3x640x480) 1800 KiB  88.3 KiB   71 KiB   51 KiB   35 KiB   29 KiB   36.9 / 35.2 dB
                                                                   *q80 = shipped setting
```

The shipped q80 is 2-3x under the best lossless alternative and
visually transparent — this is photographic/painted art (the Robin portrait
loading screen), exactly VarDCT's home turf; even q70's 33-35 dB reads
clean at full size. Dropping to q70 would save another ~28% but only
~50 KiB absolute — not worth a schema knob. A handful of key-collision
pixels exist in the slideshow (2-4 px per quality); harmless because
the runtime keys off the shipped alpha channel, not RGB. Verdict: keep
q80; nothing to wire.

### Visual-tolerance map across the asset classes

- **Tolerant**: loading/slideshow art (photographic; q80 transparent,
  q70 fine) and large RLE map patches / ambient animation frames
  (organic textures composited onto maps that are already jxl-q80;
  worst q70 case visually near-clean). These two classes are exactly
  "map-like" — the same content family where JXL took terrain 60%.
- **Marginal**: small dithered RLE effect/pickup sprites (worst cases
  18-20 dB, visible softening at 1x) — they ride along with the patch
  bucket but cost almost nothing; if one ever looks bad in-game, a
  per-sprite lossless escape (d0 or raw RLE) is cheap.
- **Risky / closed**: VQ character sprites (the head-to-head above —
  lossy breaks 565-exactness AND loses 4-5x on size), and anything a
  parity trace screenshots. Palette-keyed transparency itself is a
  solved non-issue in every mode via the 2-bit class maps (masks) or
  the shipped alpha channel (paks); the risk was never the keys, it
  is the dither.

RLE/pak repro:

```
cargo run --release --example jxl_sprite_probe -- \
    --data-dir datadirs/fullgame_linux --out tmp/jxl_sprite_probe --rle
cargo run --release --example jxl_sprite_probe -- \
    --data-dir datadirs/fullgame_linux --out tmp/jxl_sprite_probe --pak
# reports: tmp/jxl_sprite_probe/report_{rle,pak}.txt; dumps under
# tmp/jxl_sprite_probe/rle/worst_q{70,80}/ and .../pak_*/worst/
# big-patch subset: add --min-dim 200 (use a separate --out)
```

## Final integrated browser measurement: 71.6 s -> 16.1 s (2026-08-29)

Everything from today combined into one build and one conversion (the
canonical `scripts/build_web_shipping_datadir.sh` recipe): schema v12
chunks, all JXL at q70 (maps, minimaps, interface, loading art), music
at 48 kbit/s from the lossless remaster drop, merged `.map`+`.min`
terrain payloads, wasm threads (wasm-bindgen-rayon, 4 workers, talc
allocator) with the fully-streamed mission install (all part requests
issued at once, zstd+bitcode decode on arrival, dependency-ready VQ
chunks dispatched to workers immediately), and the parallelized session
setup. Verify green (402,303 sprites bit-identical, closure check ok).

Fresh-profile headless Chrome, loopback COOP/COEP server, same
methodology as the v10/v11 runs, `H01_Lin_VL`:

```
                                   v10 (morning)   v11 (afternoon)   final
wasm gzip + bindgen JS gzip         5,921,549 B     4,862,157 B     4,899,142 B
boot datadir                        9,324,395 B     7,893,113 B     7,235,504 B
blocking mission files             24,857,987 B    25,272,333 B    24,699,538 B (72 files)
total through first-mission        41,699,607 B    39,623,279 B    38,432,730 B
navigation -> in-game                    71.6 s          23.5 s          16.1 s
```

Timeline of the final run: wasm instantiated +0.4 s, datadir loaded
+0.5 s, worker pool ready (4 threads) +0.6 s, all 72 mission files
fetched +4.0 s (parallel; the old loader issued them one at a time),
mission activated +9.6 s (fetch, streamed decode, and SwiftShader
engine bring-up all overlap in that window), in-game (replay recording)
+16.1 s. Session bootstrap is now 6.6 s of the total and is dominated
by the JXL background-map decode (3.1 s) and frontend/menu resource
assembly (2.6 s) — the next optimization targets if anyone wants them;
the sprite codec no longer appears in the top spans at all.

Headless caveat: SwiftShader (software GL) inflates engine bring-up;
on a real GPU the total should land noticeably under 16 s. The shell
also gained a boot progress bar (streamed byte progress through engine
download/compile, assets, datadir, boot). Production cross-origin isolation
is supplied by reviewed response headers on the Cloudflare static origin.

## Shipping integration: schema v15 — runtime locale overlays (2026-08-29)

Datadir schema v15 (`RHDDNA15`) adds the canonical per-locale resources and
raw byte maps used by runtime language switching. The serialized form keeps
owned `Vec<u8>` values; loading converts those bytes to the VFS's shared
`AssetBytes` representation only at the atomic locale-mount boundary. This
keeps the on-disk manifest portable while avoiding repeated runtime copies.

Mission payloads are unchanged from the preceding RLE-atlas format at
`RHMISN08`/v8. Because bitcode is not
self-describing and the top-level datadir shape changed, older datadir
manifests are rejected with a regeneration error instead of being decoded as
the new shape or assigned an invented locale identity.

## Session bootstrap: overlapped terrain + interface decodes (2026-08-29)

Follow-up to the final integrated browser measurement above, whose closing
note named the two remaining session-setup hot spots: the JXL background-map
decode (`level+bank setup: background map dims`, 3.1 s) and frontend/menu
resource assembly (`mission bootstrap: frontend assembly`, 2.6 s, of which
`process frontend: in-game menu resources` 1.6 s and `mission sprite setup:
portrait cache` 0.7 s).

Two things changed since that run, and both matter for reading the numbers:
main enabled jxl-rs SIMD and `opt-level = 3` for the load-path decoders
(which alone cut the map decode several-fold), and this work overlapped the
remaining decodes with the rest of setup. Everything below is measured on top
of the SIMD build, so it is *additional* to that win, not a restatement of it.

### What changed

- **Terrain (background `.map` + minimap `.min`)** — `PendingTerrainDecode`
  (`crates/robin_rs/src/level_loading_host.rs`) starts the decode as soon as
  the mission header names the map: a dedicated thread natively, a rayon
  worker job on `wasm-threads` builds (joined by awaiting a oneshot, so the
  browser main thread never `atomics.wait`s), and the old inline pre-engine
  decode as the single-threaded-wasm fallback. `Engine::new` still runs on
  header-probed dimensions; interactive missions now carry the pending decode
  through spellforge/audio/descriptor setup and join it in frontend assembly,
  immediately before the GPU upload. True-headless joins right after engine
  construction as before. The minimap decode rides in the same job instead of
  running on the main thread.
- **Interface (`DEFAULT.RES`)** — after pre-engine metadata extraction,
  `MissionProcessResources::start_interface_decode` hands the archive to a
  worker that eagerly decodes every encoded (JXL) picture once
  (`ResourceManager::decode_all_encoded_pictures`) and duplicates the decoded
  manager for the in-game menus. Frontend assembly awaits the pair, so
  `load_mission_sprites` (portrait cache) and `IngameMenuResources` find every
  picture already decoded instead of decoding hundreds of interface images
  serially on the loading path.
- **jxl-rs parallelism** — `Picture::load_jxl_rgb565_parallel` installs a
  rayon-backed `JxlParallelRunner`, plus a row-parallel RGB565 collapse. Used
  only off the browser main thread.

### Browser: baseline vs. after, same machine, same load

Headless Chrome, loopback COOP/COEP server, `H01_Lin_VL`, RHDDNA13 tree,
4 worker threads. Baseline is main + this branch's instrumentation commit
(same SIMD jxl, same spans), after is the full branch; the two runs were taken
back to back at load ~6 so the comparison is not confounded (an earlier
after-run taken at load ~14 read roughly twice these numbers throughout).

```
span                                        baseline     after
level+bank setup: background map dims          429 ms      < 50 ms (probe only)
level+bank setup: total                        681 ms       395 ms
frontend assembly: terrain decode join           —          438 ms
frontend assembly: map upload                  131 ms        97 ms
mission sprite setup: portrait cache           194 ms        89 ms (whole phase)
process frontend: in-game menu resources       272 ms        58 ms
mission bootstrap: frontend assembly           708 ms       721 ms
mission bootstrap: total                     1,804 ms     1,418 ms
navigation -> in-game                           10.8 s       10.0 s
```

Session bootstrap drops 1.80 s -> 1.42 s (-21%). The interface pre-decode is
the clear winner: menu resources 272 -> 58 ms and the portrait-cache phase
194 -> 89 ms, i.e. ~380 ms of JXL decoding moved off the loading path
entirely.

### Native (`--headless --mission H01_Lin_VL`, fullgame_linux, dev build)

```
span                                        before(*)     after
level+bank setup: background map join        1,742 ms      0 ms
level+bank setup: total                      3,890 ms  1,301 ms
```

(*) the before column is this branch's own pre-SIMD run; the SIMD merge and
the worker overlap both contribute to the after number. What the after run
shows unambiguously is that the join itself is free: the decode thread
finishes inside the 912 ms `Engine::new`, so true-headless pays nothing for
the map even though it cannot defer to frontend assembly.

### What did not pay off, and what is still open

- **jxl-rs has no ROI/tile decode.** jxl 0.6 exposes parallelism only through
  a caller-supplied `JxlParallelRunner` (an index-addressed task set per
  decode group); there is no crop/region API, and `scan_frames_only` /
  `start_new_frame` are animation-frame seeking, not spatial. So "decode the
  visible region first" is not available without patching jxl-rs.
- **Parallel sections inside the browser worker look counterproductive.**
  With SIMD on, the whole map decode is ~430 ms serial, yet the deferred join
  still waited 438 ms — the job spread across the same 4-worker pool did not
  finish inside the ~700 ms of overlap it was given. Running the job serially
  on one worker (the pool exists to keep it off the main thread, not to split
  it) should close that gap; the change is a one-line flag at the wasm
  dispatch site in `PendingTerrainDecode::start_with_files`. It is *not* in this branch:
  the machine's btrfs metadata filled up (8.20/8.72 GiB with zero unallocated
  space, so every build failed with ENOSPC despite ~50 GiB free), and shipping
  an unmeasured perf change would have broken the code/measurement pairing.
- **Remaining serial cost.** After this work the largest session-setup span is
  the terrain join (438 ms); frontend assembly's own total barely moved
  because the join simply replaced the decode that used to sit earlier in the
  timeline. Fixing the point above is the next ~400 ms.

State safety: only host-side pixel/GPU preparation moved. The order of every
engine-affecting step (bank install, scripts, `Engine::new` inputs, spellforge
startup, audio, campaign clock, peasant-name generation) is unchanged, so
replay determinism is preserved; an existing 790-frame replay plays back
cleanly on the new binary.
## Lazy character-chunk streaming: activate before the decode tail (2026-08-29)

The browser install no longer waits for every VQ sprite chunk before
activating the mission. Chunks are partitioned at install time
(`SpriteDeferral` in `crates/robin_rs/src/shipping_mission.rs`):

- **Critical (blocks activation)**: everything referenced by entities
  present at mission start — the mission team, the level's authored
  soldiers/civilians/PCs-to-rescue, all mission-core chunks (targets,
  patches, animations, objects, Blip00), plus any family-hub chunk a
  critical chunk names as a coding base (`base_rhs`/`base2_rhs`
  promotion runs to a fixpoint, so hub-before-variant ordering is
  preserved).
- **Deferred (streams after activation)**: reinforcement-only gang
  characters — uninstanced non-VIP gang profiles whose RHS is not also
  needed by the team or the level. Unknown/ambiguous chunks default to
  critical, so a misjudgment can only delay activation, never a
  start-visible sprite.

After `install_mission`, a `spawn_local` driver streams the deferred
chunks on the same rayon worker pool (strict dependency dispatch on a
sparse `Arc`-shared row clone) and publishes each decoded grid through
the mission-owned `robin_assets::late_sprites::SpriteStreaming` context:
every not-yet-decoded VQ
row in the live `FrameHolder` holds a shared `OnceLock` cell
(`PackedSprite::late_grid`), so grids appear to rendering and mouse
hit-testing in place, with no frame-holder republish. A draw that races
a pending grid degrades safely: `ensure_sprite_cached`/
`ensure_outline_cached` skip (and crucially do not cache) the sprite
with a `skipped draw: sprite pixels still streaming` debug line, the
`uncompress_frame*` family paints transparent instead of panicking, and
`is_pixel_opaque` reports transparent. Simulation orientation commands
also consume opacity, so simulation-required grids must remain resident
before activation; pending visual rows cannot supply authoritative
simulation opacity. Post-activation decode failures warn and leave those
sprites skipped. Each installed mission and its frame-holder clones own
their own cells, progress and skipped-draw diagnostics. The background
publisher holds a weak handle and rejects publication after retirement.
A successful replacement retires only the replaced mission's stream;
a failed replacement leaves it running. Independent asset installations
can stream overlapping sprite IDs without resetting each other.
Serial (non-cross-origin-isolated) and
native installs keep the old fully-blocking behavior. Mission restarts
reuse the registry cells, so a finished tail survives re-entry.

The loading bar is now work-weighted instead of files-counted: fetch
progress by bytes received (bodies read through `ReadableStream`
readers with `Content-Length` learned from the parallel headers; totals
reconciled to actual body sizes), decode progress by critical VQ blob
bytes materialized (weight 6.0 per byte vs. one network byte), combined
into one monotonic fraction (`InstallWorkModel`, reported as n/100 work
units through the existing `MissionLoadProgress` interface, with a
150 ms ticker so the bar moves during long bodies). After activation,
the deferred tail reports honest blob-byte progress as a small
"Streaming sprites N% (done/total)" HUD line (`hud_text.rs`,
`FrameHolder::sprite_streaming_status`) until it completes or fails.

No shipping formats changed: same magics and no converter changes. The
mission-owned context in `shipping_datadir.rs` is runtime-only and excluded
from serialization; the partition and driver reuse the existing
`VqDecodeScheduler` entry points.

### What actually defers, measured

The deferrable set is exactly the payload `required_dependencies` pulls
in *beyond* the mission itself: the reinforcement candidate pool. In the
fullgame CPF only three of the ten character profiles are non-VIP —
`MerryManA/B/C` (`datadirs/fullgame_linux/Data/Configuration/profile.json`)
— and reinforcement selection can only instantiate uninstanced non-VIP
gang members, so those three (plus the object/projectile RHS their
actions enable) are the entire candidate set. Everything else is
critical.

Consequently the size of the win scales with how many Merry Men are
recruited-but-not-taken on the mission, and it is exactly zero for a
context-free launch: `Campaign::reset` seeds only Robin into the gang,
so a `?mission=`/`--mission` launch (what the headless-Chrome harness
drives) has an empty reinforcement pool, fetches no reinforcement RHS,
and therefore has nothing to defer. Browser measurements on that path
are a no-regression check, not a speed-up demo:

```
                       pkg-base (main)          pkg-after (this change)
H01_Lin_VL       26.2 / 12.6 / 17.7 s      11.7 / 17.7 / 15.9 s
                        median 17.7 s             median 15.9 s
Tac01_FoA_MP                   17.9 s             9.0 / 9.3 s
Sherwood                        6.1 s                    4.8 s
```

(headless Chrome, SwiftShader, loopback COOP/COEP server, `ship_web_v13`
datadir; time from `wasm_boot` to "activated shipping mission", which on
these launches equals time to "Recording replay"). H01 runs were
interleaved base/after to share load conditions. The run-to-run spread
(11.7-26.2 s for the same binary) dwarfs any difference between the
columns: this machine runs many concurrent agent builds, and the
measurement is dominated by CPU/IO contention rather than by the
install. That is the expected result — with an empty reinforcement pool
the partition is a no-op and both binaries execute the same schedule —
so these runs are a no-regression check only. No deferred-tail line, no
`skipped draw` line, and no missing-sprite artifacts appeared on any
run.

The partition rules themselves are unit-tested in `shipping_mission.rs`
(`SpriteDeferral` compiles on every target; only the streaming driver is
browser-only): only uninstanced non-VIP gang members outside the mission
team are deferrable, coding bases named by critical chunks are promoted
transitively (including `base2` star-2 hubs), level-start characters are
pulled back into the critical set along with their hub chains, and
Sherwood/saved-world launches defer nothing at all.

## Shipped: lossy-JXL RLE sprite atlases, web only (schema v14, 2026-08-30)

Productionizes the 2026-08-29 "the RLE/patch bucket — lossy JXL WINS here"
follow-up as `convert_datadir --rle-sprite-format {exact,jxl-q70,jxl-q80}`,
wired into `scripts/build_web_shipping_datadir.sh`. Default is `exact`:
native shipping stays byte-preserving, because these sprites composite into
RGB565 framebuffers that parity traces screenshot.

Two things about the shipped format differ from the research prototype, and
both made it smaller and simpler.

### The class map is the alpha channel, not a sidecar

The probe carried a zstd'd 2-bit class map beside each image to keep run
extents and the keyed pixels exact. Shipping instead puts the class in the
JXL's own alpha channel — 0 transparent, 128 shadow, 255 opaque — coded
losslessly with `cjxl --alpha_distance=0` while the color channels stay
lossy VarDCT. One RGBA image per atlas, no sidecar, no second entropy stage.

Alpha is a class MARKER, never a blend factor. Nothing composites the stored
RGB at partial opacity: materialization turns the decoded image straight into
the RGB565 canvas with raw `SHADOW_KEY` in the shadow pixels, so the
ambience-dependent shadow substitution still happens per draw exactly where it
always did (`decompress_rle_arno_law`'s mapping), and hit-testing still keys
off `SHADOW_KEY` / `TRANSPARENT_COLOR_16`.

Exactness is enforced twice, and loudly: the converter decodes every encode it
produces and fails the conversion if any pixel changed class, and
`--verify-shipping` re-checks per sprite against the source bank. A stray
alpha value that is not one of the three markers is a hard decode error
rather than a nearest-match guess. Requantized visible pixels are also
key-dodged (low green bit flipped) so a lossy color can never land on a key
value and silently become transparent.

### Invisible pixels are edge-extended, not black-filled

The key colors must never be coded literally — the transparent key `0x07C0`
is bright green and the shadow key `0x001F` blue, so VarDCT ringing would
bleed them into neighbouring visible pixels. But the obvious fix, writing
`(0,0,0,0)` (what `Picture::to_rgba8888` does for the interface path), still
drags edge pixels dark. Every invisible pixel now takes the color of its
nearest opaque neighbour (4-connected BFS, radius 8, alpha untouched), which
gives the DCT a smooth continuation across the sprite boundary and across
atlas cell gutters. The same smear was applied to the interface/pak keyed
JXL path, which had the black-fill bleed too.

### No repacking: the decoded atlas IS the sprite

The prototype rebuilt packed RLE run/literal words at load time. That is pure
busywork: nothing downstream draws from runs — all four `frame_holder`
consumers (the ArnoLaw blit, both shadow-extraction blits, and the
`is_pixel_opaque` hit test) decompress to a full RGB565 canvas anyway. So one
atlas decodes into one shared canvas and each sprite keeps a window into it
(`SpriteRaster { atlas, stride, x, y }`); the consumers gained a raster branch
that is a strided copy with the identical per-pixel mapping.

Dropping the run format also dropped a class: the prototype's fourth
"in-run literal carrying the transparent key" class exists only to reproduce
run bytes. Its canvas value is `TRANSPARENT_COLOR_16`, which is precisely what
all four consumers already produce for it, so three classes suffice.

### Demo numbers (demo_leicester_ecoste, full web recipe)

`rhs/` bucket, same binary and schema, only the flag differs:

```
--rle-sprite-format exact      17,966,211 B
--rle-sprite-format jxl-q70    17,305,957 B   (-660,254 B, -3.7% of the bucket)
  prototype mask design        17,367,733 B   (alpha is 61,776 B smaller)
```

The bucket total is dominated by VQ character chunks (16.1 MB of blobs); the
RLE part itself goes 1.30 MB (zstd'd exact words) -> 637,057 B of JXL, i.e.
**0.49x**, for 288 lossy sprites. Verify is green: 64,770 sprites bit-identical
to the source bank, 288 RLE sprites lossy with structure and keys bit-exact,
28.1 dB opaque-pixel PSNR, worst chunk 24.2 dB, dependency closure covered.

### Quality gating

q70's worst cases are small dithered pickup/effect sprites, exactly as the
research predicted — the first demo verify failed on `RELIC_Ampulla` at
21.8 dB. The converter now scores every encode and keeps exact words below a
24 dB per-sprite floor: a member under the floor is ejected from its atlas and
the group re-packed (it retries individually first), which also guarantees the
per-chunk floor `--verify-shipping` enforces. On the demo that demotes 105
sprites; 250 more are skipped for being under 20x20 px, and 1 loses on size.

### Resident memory: the honest cost

A canvas is 2 B/px whether or not it is mostly transparent, while packed RLE
words cost nothing for transparent runs. Demo, all chunks materialized:

```
decoded atlases (288 sprites, 29 atlases)   11,162,802 B
  of which sprite pixels                     9,571,256 B
  of which atlas gutter waste                1,591,546 B  (14%)
packed RLE words they replace                 4,720,222 B
```

So the shipped bytes shrink ~2x while resident bytes grow ~2.4x. That is the
trade to watch on wasm; it is bounded per mission (chunks are mission-scoped),
and the gutter share is small because animation groups pack same-size frames.
Uniform-grid cells are why any gutter exists at all — a shelf packer would
recover most of that 14% if it ever matters.

### Fullgame numbers (fullgame_linux, full web recipe)

Same binary and schema on both sides; only `--rle-sprite-format` differs.

```
                                exact            jxl-q70          delta
rhs/ bucket                96,287,605 B     87,951,100 B    -8,336,505 B  (-8.7%)
whole Data/               156,931,728 B    148,594,453 B    -8,337,275 B
H01_Lin_VL blocking set    24,699,643 B     24,192,060 B      -507,583 B  (-2.1%)
```

The bucket saving lands where the research said it would (~8 MiB); the
per-mission blocking saving is much smaller because one mission touches only a
few animation chunks. 5,682 sprites ship lossy across the corpus: 6,144,079 B
of JXL replacing 59,398,464 B of raw RLE words (zstd'd to ~14.5 MB in the
exact tree), in 904 images across 60 chunks.

Verify green on the whole tree: 402,303 sprites, 396,621 bit-identical to the
source bank, 5,682 lossy with structure and keys bit-exact, 29.8 dB
opaque-pixel PSNR, 17.9% of visible pixels still exactly 565-equal, worst
chunk 24.1 dB, all 50 dependency lists closure-covered. The quality floor
demotes 1,546 sprites to exact words and 1,755 more are under 20x20 px.

Native H01 install decode (`--decode-bench`, all 68 blocking files):

```
                        exact tree      jxl-q70 tree
VQ blobs (45)              2.50 s          2.80 s
RLE-JXL chunks (3)            —            0.32 s
resident RLE rasters          —          3,862,642 B (88 sprites, 21 atlases,
                                          13% of it gutter)
```

### Browser install (headless Chrome, threaded wasm, `H01_Lin_VL`)

`node scripts/wasm_mission_install_chrome.mjs <tree>`, the two trees run
interleaved on a noisy box (load 4.5-8.8), so minimums are the honest
statistic:

```
                    runs (s)                  min
exact       9.0, 11.9, 8.3                    8.3 s
jxl-q70     11.2, 9.1, 8.7, 8.5               8.5 s
```

Install time is unchanged within noise: the extra JXL decode (0.32 s native
for H01's 3 chunks; on the pool it overlaps the fetches) is roughly offset by
the 507,583 fewer bytes to download. Both trees report the same 72 blocking
files and reach `activated shipping mission` cleanly.

### Verdict and what is still open

Ship it for web: -8.3 MB off the rhs bucket for no measurable install-time
cost, at 29.8 dB on visible pixels with keys and structure bit-exact. Native
keeps `exact` and stays parity-safe.

Open items, in the order they would pay:

- **Resident memory** is the real trade (+2.4x on the sprites it touches;
  3.86 MB for H01). If wasm heap ever becomes the constraint, the levers are a
  shelf packer (recovers the ~13% gutter share) and dropping the raster to
  RGB565-with-holes only for sprites that are actually drawn.
- **The 24 dB floor is conservative.** It demotes 1,546 fullgame sprites back
  to exact words; several are the `Z_*` ambient character animations, which are
  large. A per-sprite quality-vs-size search (encode at q80 when q70 misses the
  floor, instead of falling all the way back) would recover part of that.
- **Interface/pak art now gets the same edge extension** but was not measured
  separately here; the earlier `--pak` numbers predate the smear.

## Campaign close-out: 71.6 s -> 9.3 s, 41.7 MB -> 36.7 MB (2026-08-30)

Everything from the two-day campaign, integrated and measured on one
build and one conversion (canonical `scripts/build_web_shipping_datadir.sh`
recipe): schema v14 chunks (RHDDNA14/RHMISN08), all JXL at q70 including
minimaps and the RLE patch bucket, 48 kbit/s music from the lossless
remasters, logical audio bundles, merged terrain payloads, wasm threads
with a fully streamed install, deferred character-chunk streaming,
overlapped session setup, jxl-rs SIMD, and GPU atlas sprite rendering.

Fresh-profile headless Chrome, loopback COOP/COEP server, `H01_Lin_VL`:

```
                              v8 base    v10 (day 1)  v11        final
navigation -> in-game         (untimed)  71.6 s       23.5 s     9.3 s
  wasm VQ decode              —          61.7 s       12.3 s     (in the 8.1 s install)
  mission bootstrap           —          6.7 s        6.7 s      1.29 s
wasm gzip                     4,633,216  5,921,549    4,862,157  5,081,695
boot datadir                  9,352,150  9,324,395    7,893,113  7,161,383
blocking mission files       26,142,522 24,857,987   25,272,333 24,192,060
total through first mission  41,659,633 41,699,607   39,623,279 36,727,000
```

The wasm carries threads, atomics, the rayon worker snippets and SIMD
jxl/zstd at opt-level 3, and is still smaller than the v10 build.
Startup audio no longer appears in the blocking set: sounds ship as 51
logical bundles plus 16 standalone tracks (was 2,046 files), and the
catalog prefetches in the background from first playback.

Fullgame buckets: rhs 87,951,100 (193.7 MB in the v8 era, 2.2x),
audio 26,771,451, terrain 17,222,400, missions 9,484,742.

Verification: 223 chunks (133 VQ blobs, 60 RLE-JXL chunks / 904 JXL
images), 402,303 sprites, 1,101,554,622 pixels — 396,621 bit-identical
to the source bank; the 5,682 lossy RLE sprites verify structurally with
keys and class bit-exact at 29.8 dB opaque PSNR, worst chunk 24.1 dB
against a 24 dB conversion-time floor that demotes failures back to
exact words. Sprite rendering is byte-identical: eight full-map captures
across three missions and four frames, 0 differing pixels.

Remaining bootstrap spans (browser): terrain decode join 426 ms,
frontend assembly 695 ms, level load 447 ms. Headless SwiftShader
inflates engine bring-up; a real GPU should land under 9 s.

## Bzip3 measurements (2026-09-07)

**Measured: useful for exact RLE compression, not a replacement for the VQ
context model.** [Bzip3](https://github.com/iczelia/bzip3) combines run-length
encoding, Lempel-Ziv prediction, a Burrows-Wheeler transform, and context-mixing
entropy coding. Its upstream text benchmarks prompted this test; all numbers
below are from our own sprite data, not extrapolated from those benchmarks.

### Setup and verification

- Source: `datadirs/fullgame_linux`; repository `d3c213aba`.
- Bzip3 **1.5.3**, upstream commit
  [`3c60c830d14f51a905fea92c6b9ffe51d7fd3742`](https://github.com/iczelia/bzip3/commit/3c60c830d14f51a905fea92c6b9ffe51d7fd3742),
  CMake Release / GCC 14.2, static libbz3, default portable CPU settings.
- Comparators: zstd 1.5.7 `--ultra -22 --long=30 -T1`, xz 5.8.1 `-9e -T1`,
  bzip2 `-9`, and freshly built release `sprite_compression_probe` codec modes.
  Bzip3 uses `-j 1` and `-b 1,4,16,32` (MiB).
- Linux x86-64, Ryzen 5 3600. Jobs ran sequentially after the build finished.
  Encode times are one run; decode times are minima of three warm-file runs,
  including process startup and writing decoded bytes to a local file.
- **63 compressed outputs, 189 successful byte-for-byte decode comparisons**:
  three original VQ streams, three ranked/plane-split VQ streams, and three
  exact RLE groups, each through seven compressor configurations. All six
  `sprite_codec` encode/decode checks also passed.

The original baseline is the probe's bank-id-ordered sequence of
`(u16 width, u16 height, u16 dictionary, u32 word_count, packed u16 words)`.
Ranked/plane-split inputs concatenate `hdr_w.u16`, `hdr_h.u16`, `hdr_d.u16`,
`hdr_len.u32`, `vq_idx_rank.lo`, and `vq_idx_rank.hi`, matching the earlier
transform matrix. Dictionary bytes are excluded from both layouts. Each
RLE group uses the original baseline layout, filtered to nonzero-dimension
sprites with dictionary index `0xFFFF`; no decoded-canvas padding is added.
The selected Day patch groups contain 232 (`leipatch`), 85 (`Linpatch`), and
97 (`notpatch`) RLE sprites respectively.

### VQ character streams: bzip3 beats zstd, but the custom codec still wins

Compressed bytes, one independent file per character:

```
input          raw bytes   zstd-22     xz -9e     bzip2-9    bz3 b1     bz3 b4     bz3 b16
RobinTown      7,910,320  3,089,401  2,849,832  3,051,517  2,713,213  2,646,596  2,608,235
Knight01      16,489,080  4,408,805  3,970,344  4,832,639  4,456,310  4,246,256  4,010,431
Guard A00      8,676,520  2,389,450  2,197,232  2,401,592  2,134,397  2,073,806  2,028,490
TOTAL         33,075,920  9,887,656  9,017,408 10,285,748  9,303,920  8,966,658  8,647,156
```

At `-b 16`, bzip3 saves **12.5% vs zstd-22** and **4.1% vs xz** over these
three files. It beats xz on RobinTown and Guard A00, but loses by 1.0% on
Knight01. Bzip2's earlier negative result does not apply to bzip3.

Fresh measurements of the existing VQ codec, same characters and tile grids:

```
input         sprite_codec standalone   sprite_codec with aux refs   bzip3 b16 baseline
RobinTown                  2,002,372                    2,012,616              2,608,235
Knight01                   2,887,813                    2,819,336              4,010,431
Guard A00                  1,546,398                    1,553,822              2,028,490
TOTAL                      6,436,583                    6,385,774              8,647,156
```

The codec columns are range-coded index blobs, excluding sprite-row metadata
and dictionaries; bzip3 includes the small per-sprite headers described above.
Even before cross-family prediction, bzip3 is **35.4% larger** than the aux-ref
blobs. Standalone codec decode takes 0.6 / 1.0 / 0.5 s respectively; bzip3 b16
baseline decode takes 0.84 / 1.73 / 0.85 s. These are different API boundaries
(in-memory codec vs CLI file decode), so treat the timing as context rather
than an install benchmark. There is no size case for replacing `sprite_codec`.

### Frequency ranking plus byte planes does not help bzip3

Same raw byte counts as the original streams:

```
input           zstd-22     xz -9e     bzip2-9    bz3 b1     bz3 b4     bz3 b16
RobinTown      2,928,451  2,811,644  3,203,271  2,838,352  2,823,379  2,846,564
Knight01       4,363,527  4,091,536  5,171,807  4,765,478  4,698,236  4,635,462
Guard A00      2,322,615  2,241,324  2,570,088  2,296,654  2,274,618  2,297,714
TOTAL          9,614,593  9,144,504 10,945,166  9,900,484  9,796,233  9,779,740
```

This combined transform increases bzip3 b16 size by **13.1%** over its original
layout total. That does not isolate ranking from byte-plane splitting; it
rejects this particular combination. RobinTown's zstd and xz numbers reproduce
the historical transform matrix exactly.

### Exact RLE patches: a modest win over xz

These are three Day patch RHS groups, not the complete RLE corpus or the
quality-gated exact remainder of a web conversion:

```
input          raw bytes   zstd-22    xz -9e    bzip2-9    bz3 b1    bz3 b4    bz3 b16
leipatch       4,627,900  1,262,103  1,140,284  1,246,812  1,086,057  1,102,391  1,092,602
Linpatch       2,059,772    763,225    696,996    720,858    635,927    637,743    637,743
notpatch       1,190,246    600,543    550,116    556,832    506,724    504,266    504,266
TOTAL          7,877,918  2,625,871  2,387,396  2,524,502  2,228,708  2,244,400  2,234,611
```

A fixed `-b 1` saves **158,688 B (6.6%) vs xz**, or **15.1% vs zstd**.
It wins on each group individually (4.8–8.8% below xz). Bigger blocks are
not consistently better: b1 is smallest for leipatch and Linpatch; b4 is
smallest for notpatch. Bzip3 is therefore worth considering alongside xz for
**exact** RLE chunks. This does not supersede the shipped lossy JXL atlas
result, which changes pixel fidelity and was not rerun here.

Summed per-file wall times for these three RLE groups:

```
                    zstd-22    xz -9e    bzip2-9    bzip3 b1    bzip3 b16
encode                 2.67      2.44       1.25        0.70         0.73 s
decode                 0.020     0.079      0.227       0.566        0.595 s
```

Bzip3 b1 encodes ~3.5x faster than xz, but decodes **7.1x slower** (and ~29x
slower than zstd). Offline encoding benefits; mission loading pays the cost.

### Decoder memory and block-size limit

All nine inputs produced identical **sizes** at b16 and b32: every input is
smaller than 16 MiB, so increasing the block ceiling buys nothing here.
Memory still rises. RobinTown, original layout:

```
codec           compressed bytes   encode s   decode s   decoder peak RSS (MiB)
zstd-22                3,089,401      3.25      0.016                11.7
xz -9e                 2,849,832      3.17      0.083                10.1
bzip2 -9               3,051,517      0.59      0.247                 4.8
bzip3 -b 1             2,713,213      0.95      0.702                 9.2
bzip3 -b 4             2,646,596      0.95      0.721                27.4
bzip3 -b 16            2,608,235      0.95      0.840                85.8
bzip3 -b 32            2,608,235      0.96      0.841               149.3
```

Peak RSS was measured in a separate verified decode pass using a small native
`fork`/`exec`/`wait4` launcher, so Python's resident input/output buffers do not
set a misleading inherited RSS floor. Knight01 b16 peaks at 99.2 MiB and b32
at 166.3 MiB. These are process RSS measurements, not browser heap forecasts;
concurrent independent decoders require a separate measurement.

The upstream [manual](https://github.com/iczelia/bzip3/blob/3c60c830d14f51a905fea92c6b9ffe51d7fd3742/bzip3.1.in)
allows 1–511 MiB blocks (default 16) and estimates encode/decode memory at
roughly six times the block size. Our small-chunk results argue for explicitly
bounded blocks, not blindly maximizing `-b`.

### Reproduction

Build the probe separately from running it, and build the pinned bzip3 CLI:

```sh
cargo build -p robin_rs --release --example sprite_compression_probe
probe_dir=$(mktemp -d /tmp/robin-bzip3.XXXXXX)
git clone https://github.com/iczelia/bzip3 "$probe_dir/bzip3"
git -C "$probe_dir/bzip3" checkout 3c60c830d14f51a905fea92c6b9ffe51d7fd3742
cmake -S "$probe_dir/bzip3" -B "$probe_dir/bzip3/build" \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF
cmake --build "$probe_dir/bzip3/build" -j 2

target/release/examples/sprite_compression_probe \
  --data-dir datadirs/fullgame_linux --out "$probe_dir/streams" \
  --streams RobinTown --streams Knight01 --streams 'Guard A00' \
  --code RobinTown --code Knight01 --code 'Guard A00' \
  --code-aux RobinTown --code-aux Knight01 --code-aux 'Guard A00'

for character in RobinTown Knight01 'Guard A00'; do
  input="$probe_dir/streams/$character/baseline.bin"
  for block in 1 4 16 32; do
    output="$probe_dir/streams/$character/b${block}.bz3"
    time "$probe_dir/bzip3/build/bzip3" -e -b "$block" -j 1 -c "$input" > "$output"
    for repeat in 1 2 3; do
      time "$probe_dir/bzip3/build/bzip3" -d -j 1 -c "$output" > "$probe_dir/decoded.bin"
      cmp "$input" "$probe_dir/decoded.bin" || exit 1
    done
    wc -c "$output"
  done
done
```

For the RLE exports, the probe only accepts names under `Data/Characters`.
Create a scratch datadir whose `Data` entries symlink to the source, except
for a real `Characters` directory containing symlinks to the selected
`Data/Animations/Day/{leipatch,Linpatch,notpatch}.rhs` files under temporary
character names. Run `--streams` on those names, then select the RLE records
as described above. This avoids modifying game data or the probe.

Local run artifacts are retained in `/tmp/robin-bzip3-bench-20260907/`:
`bench.py` (layout preparation and complete CLI sweep), `inputs.json` (sizes
and SHA-256 hashes), `results.jsonl`, `measure.c`, and `memory.jsonl`.
These scratch files are not repository dependencies.

**Decision:** retain `sprite_codec` for VQ and the current shipping formats.
Bzip3 is a measured candidate for exact RLE chunks where download size matters
more than decode latency. TODO: before integrating it, benchmark the complete
per-RHS exact RLE corpus and the web quality-gate remainder, count actual
mission-closure savings, and measure current wasm decoder size, heap, and
install time. Upstream's [Emscripten notes](https://github.com/iczelia/bzip3/blob/3c60c830d14f51a905fea92c6b9ffe51d7fd3742/PORTING.md)
contain historical v1.1.7 code-size figures, not measurements of this build.


## Decoder performance, 2026-09-07

Profiled the shipping mission decode as a whole: boot manifest, zstd/bitcode
parts and merge, VQ materialization, RLE/JXL atlases, interface pictures, and
terrain. VQ context decoding dominated; the sampled hot paths were context
lookup/update, symbol selection, and allocation. This change keeps the existing
formats and decoded output:

- Calculate SEE dominance quartiles with exact threshold comparisons instead
  of a variable 64-bit division at every visited context.
- Walk VQ grids by row and column, avoiding repeated width divisions and
  rebuilding the shifted sprite reference for each tile.
- Store the first four context symbols inline with `SmallVec`, avoiding tiny
  allocations while preserving symbol order, adaptation, and heap spill behavior.

The new `asset_decode_bench` example measures each phase separately, with inputs
pre-read and fresh decoder state per iteration. Hashing and destruction are
outside its phase timings. It explicitly enumerates shipping picture collections
without relying on the omitted legacy archive index, and forces decode errors
to fail the run. It does not measure network, GPU upload, audio playback, or
simulation; these are asset decoder measurements, not time-to-play estimates.

### First pass: native VQ improvements

Ryzen 5 3600, Linux x86-64, Cargo release, baseline `c84d66d79` versus these
changes. Converted Leicester demo once with the baseline converter, using
`--format shipping --map-format jxl-q80 --interface-image-format jxl-q80
--rle-sprite-format jxl-q80 --audio-format opus --zstd-window-log 30`.
Both executables read the exact same `Dem_Lei_MP` closure: 22,920,071 compressed
bytes, 28 VQ chunks, five RLE/JXL chunks, 1,171 interface pictures, and two
terrain images. Both builds used the same benchmark harness and dependency features.

Three interleaved runs per build and worker count; values below are medians.
The shared host was busy, so wall times are noisy. Process CPU and cycles,
measured by `perf stat`, also include hashing, destruction, and setup.

| Workers | Measurement | Before | After | Reduction |
|---|---|---:|---:|---:|
| 1 | Complete decode wall time | 16.065 s | 13.264 s | 17.4% |
| 1 | VQ wall time | 11.922 s | 10.114 s | 15.2% |
| 1 | Process CPU time | 13.814 s | 11.285 s | 18.3% |
| 1 | Process cycles | 54.780 billion | 44.750 billion | 18.3% |
| 4 | Complete decode wall time | 6.655 s | 5.719 s | 14.1% |
| 4 | VQ wall time | 4.489 s | 3.592 s | 20.0% |
| 4 | Process CPU time | 14.046 s | 11.496 s | 18.2% |
| 4 | Process cycles | 55.671 billion | 45.544 billion | 18.2% |

All twelve runs produced the same SHA-256 over decoded VQ grids, visible RLE
sprite rasters, interface pixels, and terrain pixels:
`a8a7ab70f933ec54c4b76759a81ac69c087c641399801864b375b59ea55519d4`.
In this first comparison JXL and zstd are unchanged; variations in their
phase timings are not claimed as improvements. The image scheduling follow-up
below is measured separately.

### First pass: WebAssembly comparison

The existing `wasm_decode_bench` was repaired to borrow mission payload fields
explicitly, then built for both versions with `wasm-release`, Rust
`1.100.0-nightly (e7769602a 2026-08-24)`, and matching wasm-bindgen 0.2.127 glue.
Node v26.7.0, single thread, three interleaved runs per version on the same
converted tree: median VQ materialization **11.510 s → 9.001 s (21.8% lower)**.
Individual before/after timings were 13.865/9.001, 11.510/8.102, and
10.629/10.623 seconds; host contention was substantial. All six results agreed
on 47,179 VQ sprites and grid FNV `589daa9f51ba962e`.
This wasm benchmark covers part decoding and VQ only; the comprehensive
picture/raster hash and remaining decode phases above were checked natively.
No browser startup or worker-pool speedup is inferred from the Node result.
Build/run instructions are in `crates/robin_assets/examples/wasm_decode_bench.rs`.

### Follow-up: RLE/JXL parallelism

The five RLE/JXL chunks contain 73 atlases. Decode both independent atlases
and each image's independent JXL sections on the existing rayon pool. This
lets workers share the remaining work inside a large chunk or atlas, rather
than waiting for its serial image decoder. The section runner is shared with
terrain decoding. On wasm, the new path uses blocking parallel joins only
when called on an initialized pool worker; main-thread and non-threaded
calls remain serial. Atlas ordering, shared raster windows, keyed pixels,
and error propagation are preserved.

After builds and smoke runs finished, repeated the same three-way comparison
three times at each worker count. `First pass` is the VQ-only optimization
above; `Final` adds atlas and JXL-section scheduling. Values are medians:

| Workers | Measurement | Original | First pass | Final |
|---|---|---:|---:|---:|
| 1 | RLE/JXL wall time | 0.570 s | 0.790 s | 0.542 s |
| 1 | Complete decode wall time | 10.345 s | 8.533 s | 7.430 s |
| 1 | Process CPU time | 10.302 s | 8.395 s | 7.527 s |
| 1 | Peak process RSS | 279.9 MiB | 298.2 MiB | 298.1 MiB |
| 4 | RLE/JXL wall time | 0.867 s | 0.677 s | 0.236 s |
| 4 | Complete decode wall time | 5.581 s | 4.206 s | 3.221 s |
| 4 | Process CPU time | 13.501 s | 10.027 s | 9.884 s |
| 4 | Peak process RSS | 581.2 MiB | 492.0 MiB | 487.9 MiB |

The strongest incremental result is **65.1% lower four-worker RLE/JXL
latency**: every final sample was 229–245 ms, versus 533–1,052 ms for the
first pass. The additional scheduling does not materially raise measured
peak RSS. The inline context storage does trade about 18 MiB more RSS in
this one-worker run for lower CPU cost; four-worker peak RSS was lower.
RSS comes from a fresh `wait`/`getrusage` wrapper around each process tree,
not a cumulative maximum across runs. These are native process measurements,
not browser heap estimates.

The host still had variable external load, despite no concurrent builds of
this worktree. VQ and interface timing also moved between these executables,
although this follow-up does not change their work; do not attribute the
entire observed total-time difference to image parallelism or claim a serial
JXL speedup. All 18 final comparison runs matched the full output SHA-256
above. Raw results are `final-comparison.jsonl` in the scratch directory.

An additional range-decoder head-symbol shortcut was tested and discarded:
it regressed the four-worker native and single-threaded wasm comparisons.
TODO: measure threaded browser latency/heap and larger full-game closures;
continue profiling interface decode once VQ and RLE/JXL costs are reduced.

Validation: 148 active `robin_assets` tests pass (eight fixture tests remain
ignored), including exact SEE boundary equivalence, codec round trips,
parallel atlas placement order, and malformed-atlas errors. Both serial and
threaded wasm compile checks pass. The release `robin` build succeeds, and
timed headless runs loaded and ran the original and converted Leicester demo
missions; each was stopped by its timeout. Demo profile/missing-asset warnings
remain outside this decoder change.

### Reproduction

Build before running, and preserve a baseline executable before applying the
runtime changes. Use the same converted data for both executables; do not
re-encode between comparisons.

```sh
cargo build -p robin_assets --release --example asset_decode_bench
RAYON_NUM_THREADS=1 target/release/examples/asset_decode_bench \
  /path/to/converted/Data Dem_Lei_MP 3
RAYON_NUM_THREADS=4 target/release/examples/asset_decode_bench \
  /path/to/converted/Data Dem_Lei_MP 3
perf stat -e task-clock,cycles,instructions -- env RAYON_NUM_THREADS=1 \
  target/release/examples/asset_decode_bench /path/to/converted/Data Dem_Lei_MP 1
cargo test -p robin_assets --release
```

Interleave the two executables to reduce drift from host load. Compare the
`output_sha256` values as well as timings. Local raw comparisons, profiles,
and the fixed converted tree are in `/tmp/robin-decoder-perf/`; scratch files
are not repository dependencies.

### Further profiling and JXL 0.7.1

Upgraded `jxl`, `jxl_macros`, `jxl_simd`, and `jxl_transforms` from 0.6.0 to
0.7.1. The rayon runner now reports its worker count, and all pixel-format
configuration calls propagate the new fallible API's errors. Both threaded
and single-threaded wasm paths are checked.

The upstream upgrade changes decoded RGB values slightly. Compared all 1,213
encoded JXL images in the fixed demo closure with both library versions:
1,132 interface images, six PAK images, 73 RLE atlases, and two terrain images.
Dimensions and alpha bytes were identical; the largest RGB-channel difference
was **1/255**. This can cross RGB565 quantization boundaries, so the complete
asset hash intentionally changes to
`3e80fe96ac7f5f20a422a8e1d60aeffeb4f28388944acb611da129b80a34b6c4`.
Performance candidates using 0.7.1 must match this new hash, not the 0.6.0 hash.

A fresh `perf record -F 499` profile after the four-symbol search experiment
still attributed 19.15% of process samples to `Ctx::find_by_target`, 15.68%
to `decode_level`, and 9.51% to `decode_sym_aux`. Disassembly showed the block
sum still used serial scalar loads and additions. The experiments therefore
focus on reducing this scan's work while preserving every arithmetic-coding
interval, symbol ordering, and adaptation step.

The retained search checks the hottest symbol first, skips 16 intervals at a
time with a portable SIMD sum, then scans only the containing block. A
`repr(C)` symbol/count pair and `bytemuck`'s derived POD validation permit
safe contiguous vector loads; the state layout is not a file format. The
repository already pins nightly Rust; `portable_simd` uses that toolchain.
On x86-64-v3 the loop compiles to two AVX2 loads, shifts, and a vector sum.
The final scalar block search stays out of line to prevent LLVM from replacing
the vector reduction with shared scalar partial sums. The first-symbol and
short-list paths remain cheap, and the interval test checks every target for
flat and skewed contexts across block boundaries and up to 4,096 symbols.

Same fixed demo, three interleaved runs at each worker count, no concurrent
worktree builds; medians below. All three variants use JXL 0.7.1. These runs
were on a less contended host than the earlier sections, so compare columns
within this table, not absolute times between experiment sessions.

| Workers | Measurement | Previous search | Four-symbol search | SIMD search |
|---|---|---:|---:|---:|
| 1 | VQ wall time | 3.856 s | 3.533 s | 3.268 s |
| 1 | Complete decode wall time | 5.439 s | 5.106 s | 4.858 s |
| 1 | Process CPU time | 5.536 s | 5.203 s | 4.956 s |
| 1 | Process instructions | 49.863 billion | 42.660 billion | 36.726 billion |
| 4 | VQ wall time | 1.412 s | 1.296 s | 1.217 s |
| 4 | Complete decode wall time | 2.018 s | 1.911 s | 1.835 s |
| 4 | Process CPU time | 6.228 s | 5.969 s | 5.677 s |
| 4 | Process instructions | 49.957 billion | 42.746 billion | 36.807 billion |

Relative to the previous search, this is **15.2% / 13.8% lower VQ latency**
and **10.7% / 9.1% lower complete decode latency** at one/four workers.
Process instructions fall by about 26% at both counts. Relative to the
four-symbol experiment, SIMD reduces VQ latency a further 7.5% / 6.1%.
All 18 outputs match the 0.7.1 hash above. Image and zstd algorithms are
unchanged by the search optimization; their timing variation is not a claimed
speedup. The raw experiment is the 18 `jxl071/grouped/packed` rows immediately
before the `packed/split` comparison in `third-comparison.jsonl`.

Discarded experiments: wider scalar sums, SIMD constructed from individual
count loads, an adjacent-count shortcut during context updates, and splitting
and inlining the context decode's exclusion branch. The latter did not produce
a reliable incremental CPU-time improvement. TODO: profile the remaining
escape/range-decoder cost and larger full-game closures before changing more
of the model representation.

Validation for the retained SIMD implementation: 149 active `robin_assets`
tests pass (eight remain ignored), including codec round trips, interval
boundaries, JXL lossless fixtures, and malformed atlas handling. The native
release game build passes; original-data and converted-data headless smoke
runs reach replay recording and are stopped by timeout. Existing demo profile
warnings persist. Two wasm-only mission-loader borrows now name
`merged.payload.levels` explicitly to allow borrowing the sprite bank and
level table together; this fixes compilation without changing load order.

### Full browser startup with the SIMD decoder

Chrome 152.0.7977.64, four wasm workers, fresh browser profile per run,
loopback HTTP with COOP/COEP, Leicester `Dem_Lei_MP` q80 data above. Both
builds use `wasm-release`, matching wasm-bindgen 0.2.127 glue, and the normal
`optimize-wasm.mjs` post-processing. Three interleaved runs compare the
four-symbol search with the final SIMD search; both use JXL 0.7.1.

The harness now timestamps events with the page's `performance.now()` and
reports time from navigation as well as from `wasm_boot`. This includes
local wasm fetch/compile, core preloads, mission installation, and engine/
frontend initialization. The endpoint is the existing replay-recording
startup marker, not a measurement of the first presented game frame.

| Measurement | Four-symbol search | Final SIMD search |
|---|---:|---:|
| Navigation → recording, median | 8.154 s | **7.958 s** |
| Navigation → recording, range | 8.148–8.556 s | 7.920–8.385 s |
| wasm_boot → mission installed, median | 2.264 s | 2.086 s |
| wasm_boot → recording, median | 7.769 s | 7.573 s |
| Engine construction, median | 3.709 s | 3.658 s |
| Post-processed wasm bytes | 21,024,590 | 21,024,835 |

Observed total startup improves about 2.4%; ranges overlap, so this small
wall-time change should not be treated as a precise general speedup. Engine
construction is now a larger startup cost than mission installation, and is
unchanged by this decoder patch. TODO: profile that phase separately.
The headless browser uses SwiftShader's CPU GL adapter and initially reports
a 1×1 surface at initialization. These are local startup-marker
measurements, not real-GPU frame timings or internet download estimates.
The demo's existing profile, restart-save, thumbnail-capture, and terminal
mission warnings/errors remain in the logs; this is not a sustained-play
validation. The older 9.3-second full-game `H01_Lin_VL` q70 result elsewhere
in this document uses different data and is not a comparable baseline.

Reproduce with the existing threaded game build instructions, then:

```sh
node wasm-www/scripts/optimize-wasm.mjs /path/to/pkg
node scripts/wasm_mission_install_chrome.mjs /path/to/converted \
  --mission Dem_Lei_MP --pkg /path/to/pkg --wait-ingame
```

Raw browser logs are `browser-{grouped,final}-{0,1,2}.log` in the scratch
directory. All final threaded and serial wasm builds pass.

The standalone single-threaded Node WASM benchmark independently confirms
**4.281 s → 3.686 s (13.9% lower)** median VQ materialization, comparing
committed first-pass code with final SIMD code on the same fixed data.
Three interleaved samples were 4.300/3.735, 4.281/3.686, and 4.261/3.659 s.
All six decoded 47,179 sprites and matched grid FNV `589daa9f51ba962e`.
This isolates VQ decoding; it does not include image decode or browser startup.
Raw results are `node-{after,packed}-{0,1,2}.log` in the scratch directory.

## Startup verification and native projection encoding (2026-09-08)

The Chrome main-thread profile shows that the interval labelled “engine
construction” also includes building the deterministic input projection.
Canonical JSON construction, integer formatting, allocator growth and sprite
opacity hashing dominate that work. The browser harness accepts
`--cpu-profile FILE` to capture a Chrome CPU profile; use an unstripped
wasm-bindgen package to retain Rust function names. Profiled runs are diagnostic,
not comparable to uninstrumented startup timings.

Ordinary (`BrowseOnly`) sessions now construct the engine directly. They skip
projection-only input clones, serialization and opacity hashing on both native
and WASM. Ranked admission and explicit native projection export still prepare
and verify the full input projection. Simulation construction and replay
recording use the same engine and campaign ownership rules.

Static simulation projection artifacts now use native `bitcode::encode` /
`bitcode::decode`, with `RHSC0002` format identity, `.bitcode` filenames and
`application/vnd.robinhood.simulation-content-component-v2+bitcode` media type.
Run projection hashes use `RHRP0002`; the projection schema is 2. There is no
JSON fallback. Existing component artifacts, content manifests and dependent
ranked authorities must be regenerated. Signed public JSON protocol documents
retain their existing encoding; this migration changes simulation fingerprints.

The native representation is a flat preorder stream of typed values. Object
keys are ordered, nonnegative integers are normalized to unsigned values, and
float projections retain exact IEEE bits, including negative zero and NaN
payloads. Readers reject malformed structure, excessive depth, duplicate or
unordered keys, trailing bytes and noncanonical encoding. The internal
`serde_value` projection remains for heterogeneous simulation inputs; replacing
that intermediate tree with directly ordered typed fields is follow-up work.

The WASM build also aligns the direct `wasm-streams` dependency with reqwest's
0.5 dependency. Linking both 0.5 and 0.6 exports duplicate wasm-bindgen symbols
and prevents the current threaded build from linking.

### Matched browser startup results

Baseline: `899a46ad4` plus only the wasm-streams link fix. Both packages use
JXL 0.7.1, the same threaded release build and wasm-opt/wasm-strip processing,
four Rayon workers, and the unchanged converted Leicester demo
(`Dem_Lei_MP`, 22,920,071 compressed bytes in the complete closure).
Chrome 152 uses its SwiftShader GL adapter and a 1×1 initial surface in this
harness. The endpoint is the game's “Recording replay” marker, not first
presented gameplay frame. Each run starts a fresh Chrome profile; three
before/after pairs alternate, with profiling disabled and no task builds
running during measurement.

| Measurement | Before | After |
| --- | ---: | ---: |
| Navigation → recording replay, median | 9.433 s | 5.583 s |
| Individual navigation times | 9.723 / 9.182 / 9.433 s | 5.613 / 5.580 / 5.583 s |
| Engine-construction interval, median | 3,952 ms | 311 ms |
| Individual engine intervals | 4,058 / 3,786 / 3,952 ms | 311 / 316 / 269 ms |
| Postprocessed WASM size | 21,288,696 B | 21,299,854 B |

That is **3.850 s (40.8%) less startup time** and a **92.1% reduction** in the
engine-construction interval. Separate named-WASM profiles have zero samples
in `canonical::write_value`, `canonicalize_serde_value` and
`simulation_opacity_sha256` after the change; all three were prominent before.

Validation: the explicit `robin_engine`, `robin_run_protocol` (102 tests),
`robin_manifest_tool` (144), `robin_ranked_verification`, `robin_replay_verifier`
(36) and `robin_rs --features projection-export` (1,510 active) suites pass,
as do both exporter-example tests and all 83 browser leaderboard/signer tests.
The original Leicester admission test independently prepares matching seals
and rejects a forged component. A 30-second native smoke run records a session;
its 104-frame replay reaches EOF with exit status 0 and no reported hash
mismatch. The normal native game and threaded WASM release build both pass.

### Further startup experiments (2026-09-08)

Two experiments were implemented and tested against `05cf64faf`, then
**reverted because optimized browser startup did not improve**:

- Replace per-sprite binary search/vector insertion in
  `ShippingMission::merge_from` with append, stable sort and duplicate checks.
  Compare dictionaries and shipped sprite fields directly instead of encoding
  them solely for equality. Preserve already decoded grids when later parts
  repeat empty VQ placeholders, and continue rejecting metadata/data conflicts.
- Batch browser audio progress updates and explicit zero-delay timer yields
  at 16 ms intervals instead of yielding after every completed item. Keep all
  489 active-mission items, the three-request concurrency limit and error
  propagation unchanged.

Separate named-WASM diagnostic profiles attributed about 266 ms inclusive to
`ShippingMission::merge_from` before and 5 ms after. These profiles used
unoptimized named packages and ran under build/optimizer contention; they do
not establish a speedup in the shipped optimized package.

The decisive comparison used Chrome 152, the fixed Leicester shipping corpus,
threaded `wasm-release` packages with the same wasm-bindgen/wasm-opt/strip
pipeline, fresh browser profiles, and no concurrent task builds or profiling.
The endpoint remains navigation to the “Recording replay” marker, not the
first presented gameplay frame.

| Experiment | Alternating pairs | Baseline median | Candidate median |
| --- | ---: | ---: | ---: |
| Bulk merge + audio batching | 8 | 4.8925 s | 4.9295 s |
| Bulk merge alone | 3 | 4.824 s | 4.908 s |

The combined experiment was about 0.8% slower; the isolated merge experiment
was about 1.7% slower. Neither supports retaining a startup optimization.
The first three combined pairs ran baseline first, and the remaining five
ran candidate first. Raw logs, profiles, package artifacts and the rejected
patch are in `/tmp/robin-merge-perf/` (local scratch, not versioned).

Before reverting, the native game and threaded WASM release builds passed,
as did all 139 active `robin_assets` tests (four fixture-dependent tests
ignored). Added regression cases covered reverse/interleaved arrivals,
duplicates, materialized-grid ownership, conflicts and out-of-range IDs.
The complete release decode retained SHA-256
`3e80fe96ac7f5f20a422a8e1d60aeffeb4f28388944acb611da129b80a34b6c4`,
covering 28 VQ chunks, 73 RLE/JXL atlases, 1,171 interface images and two
terrain images. Experimental code and its new tests were reverted together.

The debug full-corpus probe hit an upstream JXL 0.7.1 subtraction overflow in
`group_scheduler.rs`: `then_some(Rect { size: (x1-x0, y1-y0), ... })` eagerly
constructs a rejected rectangle. Release pixel verification passed; no
assertions were suppressed and no dependency patch was introduced.
TODO: move to an upstream fix for this debug-only empty-rectangle case.

TODO: investigate motion-grid initialization, JXL context-map validation and
terrain decode/upload using optimized browser measurements. These remain
visible costs; the rejected experiments show why named-profile improvements
must be checked against total startup before retaining them.

### Interior-cell shortcut for motion-grid construction (2026-09-08)

The remaining profile highlighted `initialize_motion_from_level_data`.
Sector registration tests a polygon against each candidate 64-pixel grid
cell. Previously it tested polygon vertices and every edge against the cell
rectangle before testing whether the cell's top-left corner was inside the
polygon. Large interior regions paid for the robust edge predicates even
though the final containment check would accept them.

`GridSector::intersects_bbox` now checks corner containment first. This
reorders the existing boolean alternatives; polygon formulas, boundary
semantics, sector insertion order and serialized grid data are unchanged.
The existing edge tests still handle cells crossing the polygon boundary.

Validation: all 4,441 active `robin_engine` library tests pass; three
original-data tests and one manual measurement remain ignored. A new differential test compares the old
predicate over nearby cells for both polygon windings, concavity, repeated
vertices, degenerate polygons and small offsets around exact boundaries.
The native game builds, and the existing 104-frame Leicester recording
replays to EOF with exit status 0 and no reported hash mismatch.

The threaded WASM release build also passes. Ten alternating Chrome 152
pairs compare the retained baseline (`05cf64faf`; `d2d47182c` only adds
documentation) with this change, using the same fixed Leicester corpus and
wasm-bindgen/wasm-opt/strip pipeline. Five pairs run baseline first and five
run candidate first. Each load starts a fresh browser profile; no task
builds, profiling or replay runs overlap these measurements.

| Interval | Baseline median | Updated median | Observed reduction |
| --- | ---: | ---: | ---: |
| Navigation → recording replay | 4.900 s | 4.8645 s | 0.7% |
| Engine construction | 259.5 ms | 250.5 ms | 3.5% |

Eight of ten startup pairs favor the change. The median paired reduction
is 54.5 ms, while the difference between the two overall medians is
35.5 ms. Baseline startup spans 4.821–5.035 s and updated startup spans
4.762–4.948 s: this is a small observed gain on a noisy shared host, not a
claim of a large or universal startup reduction. The endpoint remains
replay recording, not first presented gameplay frame, and this local test
does not measure production-network download latency. Logs, optimized
packages and the benchmark runner are in `/tmp/robin-grid-perf/`.

TODO: continue profiling the optimized package's JXL validation and terrain
decode/upload costs; the grid shortcut leaves most startup time intact.

### Detailed browser startup timing (2026-09-08)

The Chrome harness now accepts `--timings FILE`. It saves browser-clock
console timestamps, request Resource Timing entries, and Web Audio decode
spans (start/end, success and decoded PCM size). Resource entries cover the
main window, not worker-local requests. Timing capture is opt-in; ordinary
benchmark runs do not wrap Web Audio. The harness also reports the end of
mission bootstrap separately from the earlier replay-recording marker.
Rust logs direct durations for boot decoding, worker-pool initialization,
Rust initialization, window creation, mission dependency planning, streaming
through the last merged part, the VQ/RLE drain tails, mission installation
and audio warmup. Audio timing separates progress-callback time and explicit
yield time from the remaining asynchronous work.

Three optimized baseline runs with these timers gave these medians:

| Measured interval | Median |
| --- | ---: |
| Boot zstd/bitcode decode | 57.4 ms |
| Worker-pool initialization | 34.2 ms |
| Rust initialization | 7.0 ms |
| Window ready | 24.2 ms |
| Mission dependency planning | 0.35 ms |
| Streaming start → all 66 parts merged | 390.2 ms |
| Remaining VQ dependency/worker wait and application | 1498.1 ms |
| Remaining RLE/JXL wait and application | 0.6 ms |
| Mission installation/activation | 3.0 ms |
| Active audio warmup, 489 planned items | 1271.5 ms |
| Within audio: progress callbacks | 1.6 ms |
| Within audio: explicit yields | 17.8 ms |

The streaming interval overlaps fetches, zstd/bitcode decoding, merging and
sprite work. The drain intervals measure only work remaining after all parts
merge; they are not the total CPU costs of their respective codecs. Boot
audio runs concurrently and is not an additional sequential startup phase.

Main-window requests show mission payload downloads finishing in roughly
100 ms on localhost, long before activation. The active audio phase issues
488 decode calls with peak concurrency three. Their summed async latencies
are about 3.46 s inside a 1.27 s wall-time phase; this sum includes overlapping
waits and must not be described as CPU time. Progress rendering and zero-delay
yields are a small fraction of this phase, explaining why the earlier
audio-yield batching experiment did not help.

Correction to the earlier informal phase breakdown: the ~170 ms
`runtime + replay init` timer ends **after** the “Recording replay” marker.
Subtracting the entire bootstrap timer from time-to-recording understated the
earlier audio gap by about that amount. Use the direct audio duration and
keep time-to-recording separate from time-to-bootstrap-completion.

Mission audio warmup now runs as an abortable background task after asset
activation. Engine duration tables continue to use the shipping metadata;
playback uses the existing shared fetch/decode futures and pending-request
handling. No warm-plan entries were removed, and concurrency remains three.
A restart or mission transition aborts the old queued warmup; already
in-flight content-addressed requests may finish and populate the cache.
Planning errors still propagate synchronously; asynchronous warmup failures
are logged and ordinary playback can retry the failed request on demand.
Cold audio may start after gameplay begins, rather than holding the entire
engine behind every voice decode. The existing 96 MiB PCM cache limit and
voice/effect cancellation policies are unchanged.

Use `--wait-audio` with the harness to keep the page alive until background
warmup completes. `--timings` saves separate recording-marker and audio-complete
resource snapshots. `--fail-request URL_PATH` can force a local request to
return HTTP 404 to exercise the background failure path.

Five alternating before/after pairs (three baseline-first, two candidate-first)
compare the instrumented blocking baseline with background warmup, using the
same optimized packages, corpus and Chrome settings. No task builds or CPU
profiling overlap the runs. The shared host was noisier than in the earlier
three-run attribution sample; do not compare absolute times across batches.

| Endpoint | Blocking median | Background median |
| --- | ---: | ---: |
| Navigation → recording replay | 6.279 s | 4.532 s |
| Navigation → bootstrap complete | 6.462 s | 4.716 s |

All five pairs improve; the median paired recording-time reduction is
1.438 s. The difference between overall medians is 1.747 s, which also
reflects variation in unrelated stages. Every background run completes all
489 planned warmup items successfully. Warmup itself now overlaps gameplay
and finishes later (9.0–17.6 s after its task starts in this software-rendered
fixture). Those durations include event-loop contention and explicit yields,
not just decoder time. The captured background intervals also include an
additional on-demand playback decode, so their summed PCM output is not
directly comparable to the foreground-only audio interval.

Representative updated run (the median startup run), disjoint browser-clock
intervals through bootstrap completion:

| Phase | Wall time |
| --- | ---: |
| Navigation → `wasm_boot` | 416 ms |
| Boot, worker pool, initialization and loading UI → streaming start | 268 ms |
| Fetch/decompress/merge through final part, overlapping sprite work | 679 ms |
| Remaining VQ work and application | 1939 ms |
| RLE tail, activation and background-audio launch | 11 ms |
| Level and engine setup | 506 ms |
| Renderer, terrain, sprites and menus | 701 ms |
| Remaining runtime/replay initialization | 196 ms |
| **Total to bootstrap completion** | **4716 ms** |

Recording begins earlier, at 4532 ms. Within level setup, engine construction
is 295 ms; within frontend setup, terrain decode join is 327 ms and map
upload is 129 ms. These are nested intervals, not additional time. Neither
endpoint is a measurement of the first physically presented gameplay frame.

Validation: 1,505 active client library tests pass (five ignored), native
game and threaded WASM release builds pass, and the final Chrome runs
verify both startup and eventual background-audio completion. A forced
HTTP 404 for the mission dialogue bundle produces the expected warmup
warning while still reaching recording and bootstrap completion. The final
change leaves decoder formats and simulation duration metadata untouched.
Raw logs, JSON timelines and packages are in `/tmp/robin-startup-detail/`.

TODO: profile the remaining VQ critical path on workers. Also tune background
warmup pacing separately: per-item yields are cheap before gameplay but
become expensive while sharing a busy rendering event loop.


### Parallel audit of startup phases above 100 ms (2026-09-08)

Seven independent read-only investigations covered every >100 ms row above.
These are opportunities, not measured speedups; their bounds overlap and must
not be added together. The packages and game behavior were unchanged during
this audit. Source references below are relative to the repository root.

| Phase | Findings and next experiments |
| --- | --- |
| Pre-boot, 416 ms | The representative trace contains only 135 ms from module import to boot: 68 ms module/WASM load, 54 ms core preloads, 14 ms boot-data fetch. The preceding 281 ms lacked attribution. Start WASM fetch alongside JS import (`wasm-www/src/main.ts`); overlap independent default datadir fetch with runtime load (`boot-lifecycle.ts`). The harness serially preloads 20 assets while production already uses 12 workers: fixing that benchmark discrepancy is not a product speedup. |
| Boot to streaming, 268 ms | Overlap worker-pool startup (~40 ms) with independent Rust/window initialization (~39 ms), joining before streaming chooses its pooled path (`bin/robin.rs`). Instrument the stable ~74 ms window-ready/loading-pak gap around event polling and surface resize; the missing pak lookup itself is in-memory. Instrument the ~35 ms plan-to-stream gap around `clear_mission`, which can lazily create an AudioContext. Avoid creating absent audio state solely to clear it if confirmed. |
| Mission assembly, 679 ms | All 66 mission resources (14,952,131 bytes) finish delivery within 135 ms of streaming start, leaving 544 ms of overlapping queue/decompression/merge/dispatch/yield work. Batch per-part progress and browser-timer yields within a frame budget (`shipping_mission.rs`); prioritize dependency hubs instead of alphabetical request order. RobinTown's actual request begins ~82 ms after the first mission requests. Instrument shared Rayon queue delays before changing task concurrency. |
| VQ tail, 1939 ms | Compile out disabled exclusion bookkeeping for WASM (`sprite_codec.rs`, runtime `ROBIN_EXCL_CAP` is zero for shipping). Allocate pair/auxiliary model maps only for modes used by a chunk. Try bounded dispatch with downstream dependency-path priority: current sorting only orders each newly ready batch before enqueueing all of it. Larger options are overlapping sprite-independent engine setup, or independently encoded restart groups for oversized chunks (format/compression tradeoff). |
| Level/engine setup, 506 ms | A 174 ms pre-engine interval includes unintended synchronous JXL decoding: `extract_titbit_row_frame_counts` calls `get_pictures`; `get_picture_count` also decodes. Add a metadata-only nonempty-picture count with identical hole/zero-size semantics. Export ground-marker bounds and minimap hit masks to avoid other metadata queries decoding pixels. Motion/grid registration occupies at most 160 ms: row-bucket polygon edges or precompute cell membership while preserving exact boundary behavior and sector order. |
| Frontend, 701 ms | Terrain starts only after VQ finishes; move it earlier to overlap the 327 ms residual join, checking worker contention. The 129 ms map phase contains only 23.5 ms background preparation/upload; the remaining 105.5 ms includes masks/depth/minimap and needs finer timers. Investigate batching mask textures and removing full-map clones/conversions. Renderer construction is 92 ms: build only the single used blit pipeline rather than four variants. Reusing a loading renderer can help when one exists, but this fixture has no loading pak and does not construct that renderer. |
| Runtime/replay remainder, 196 ms | All five runs fail Restart save indexing (`mkdir: operation not supported`), yet still register its frame-0 marker. That computes an engine hash and a complete JSON save identity (`runtime.rs`, `save_file.rs`). Carry successful save creation/indexing status through bootstrap and register only on success. Current runtime timer is 180–186 ms; older named profiles attribute 94–96% of this path to save identity, suggesting ~170 ms potential, requiring a fresh paired measurement. Preserve successful native save markers. Separately migrate identities still needed for real saves to a typed canonical representation. |

The 295 ms engine-construction timer includes sprite-variant/opacity publication
before the constructor. Deterministic audio-duration tables take about 12 ms
and use shipping metadata; they do not decode Opus. The VQ tail is an elapsed
wait, not proven pure codec CPU time. Record ready/enqueue/worker-start/end/apply
timestamps for VQ, RLE and part-decompression jobs to distinguish dependency
stalls, occupied workers and main-thread application delay.

The harness now saves NavigationTiming and its earliest inline-script timestamp.
Two fresh runs with the same optimized background-audio package put script start
at 341.2 and 338.5 ms. Fetch start to domain-lookup start consumes 323.0 and
318.7 ms; DNS/connect take under 0.3 ms, and document response ends at 330.1 and
327.7 ms. This locates most pre-script time before the document connection,
not inside WASM or game code. It does not identify the browser-internal cause,
and these new timings must not be substituted into the older representative
run. Traces: `/tmp/robin-startup-opportunities/navigation-{0,1}.json`.

TODO: first benchmark successful-save gating and metadata-only frame counts;
then exclusion specialization and earlier terrain scheduling. Before a broader
scheduler rewrite, capture the worker timeline. Validate startup through full
bootstrap and first gameplay frame, using alternating optimized browser pairs;
include a visible hardware-GPU canvas because the current SwiftShader fixture
initially configures a 1x1 surface. Preserve corpus hashes, replay behavior,
mission transitions and serial fallback. Do not retry the rejected bulk-merge
or audio-yield experiments as established wins.

Audit validation: both new browser runs reached bootstrap completion and saved
NavigationTiming; `node --check` passed for the harness. No Rust code changed.


### Implementing the parallel startup audit (2026-09-08)

The performance branch was rebased onto rewritten main
`cc36f8d75f5ffcfa18696e96fbb766878df5c26e`, preserving its session-owned
browser audio lifecycle. The earlier measurements above predate that rewrite;
they are not the baseline for the new comparison.

Implemented candidates cover each audited phase:

- Production boot overlaps default data download with runtime loading and WASM
  fetch with JavaScript import. Worker initialization also starts before window
  setup, and identical surface resizes no longer reconfigure the surface.
- Optional DEBUG worker timestamps distinguish readiness, queueing, execution,
  receipt and application; normal INFO runs omit those clock reads. An 8 ms
  budget between cooperative yields was tested in the first candidate bundle
  and subsequently removed pending isolated evidence of benefit.
- WASM sprite decoding compiles out disabled exclusion bookkeeping and reserves
  auxiliary context maps only for modes actually referenced by a chunk.
- Picture counts and dimensions use metadata without decoding JXL pixels. Grid
  registration filters polygon edges once per row, preserving cell boundaries
  and registration order.
- Frontend setup builds only the used opaque blit pipeline, borrows cached map
  buffers and starts terrain work before mission-resource environment setup.

Browser Restart now captures an immutable, session-owned persisted-state
checkpoint instead of trying to create a filesystem save. The checkpoint is
published only after capture and validation succeed, and is cleared at mission
entry or when its owning save manager is replaced. It has a process-local
identity tied to the immutable payload, avoiding full JSON serialization for
replay save identity. Transporting or serializing a checkpoint strips that
identity. The required frame-0 simulation hash remains. Durable browser saves
continue to use their existing localStorage backend; native saves retain disk
and payload-identity behavior.

Restart tests exercise capture with an unavailable filesystem, comparison with
an actual disk round-trip, restore of engine/host/game persisted state, identity
lifecycle, failed capture, and replay load-back to frame 0. The integrated native
client suite passes 1,556 tests (five ignored); native and threaded WASM release
builds pass. Metadata, codec and grid changes also passed their affected package
suites, including frozen decoded-output checks and differential cell registration.

TODO: opacity metadata export, independent sprite restart groups, broader
terrain/VQ overlap and dependency-aware scheduling remain research opportunities.
Do not infer their benefits from the implemented bundle. The direct WASM harness
bypasses production TypeScript boot and cannot measure its fetch-overlap change.


Five fresh-profile, alternating baseline/candidate browser pairs compare the
rebased pre-candidate WASM (`cd89a12e7`) with the integrated optimized WASM
(`3b0cebc62`). Both use the same local Leicester shipping corpus and optimization
pipeline. No builds or other task browser runs overlap this comparison.

| Endpoint, navigation-relative | Baseline median | Candidate median |
| --- | ---: | ---: |
| Mission activation | 2.794 s | 2.575 s |
| Recording begins | 3.901 s | 3.711 s |
| Bootstrap complete | 4.070 s | 3.718 s |

Four of five bootstrap pairs improve. The median paired reduction is 292 ms;
the difference between overall medians is 352 ms (8.6%). One pair regresses by
131 ms, so these are noisy local results, not a guaranteed per-run saving.
Recording-to-bootstrap drops from a median 168.3 ms to 7.3 ms with the real
session Restart checkpoint in place. This comparison tests the whole bundle,
not an isolated Restart change.

The candidate run at the median bootstrap time has this disjoint breakdown:

| Phase | Wall time |
| --- | ---: |
| Navigation to `wasm_boot` | 424 ms |
| Boot and initialization to streaming | 204 ms |
| Part fetch/decompression/merge, overlapping sprites | 488 ms |
| Remaining VQ wait and application | 1455 ms |
| RLE/activation plus level and engine setup | 364 ms |
| Frontend assembly and intervening work | 760 ms |
| Remaining checkpoint/runtime setup | 23 ms |
| **Bootstrap complete** | **3718 ms** |

Across runs, level-load timer medians are 424 → 396 ms and terrain-join medians
340 → 318 ms. Frontend assembly does not improve in this sample (677 → 686 ms),
nor does all-parts-merged time (394 → 488 ms), while the VQ-tail median falls
1665 → 1533 ms. Those phases overlap and shift when dispatch timing changes.
The bundle result therefore does not establish the 8 ms yield budget, pipeline
change or any individual decoder change as a separate speedup. The yield-budget
experiment was subsequently reverted; its independent benefit is unproven.

The new mask-texture timer measures a median 100 ms, compared with 123 ms for
all map upload work. Mask preparation/upload is the dominant measured part of
that phase. A worker trace is collected separately at DEBUG to avoid including
its logging cost in the paired comparison.

The fixture uses SwiftShader and initially configures a 1x1 surface; it is not
a production hardware-GPU measurement. Bootstrap completion also does not
measure the first physically presented gameplay frame. Raw packages, ten run
logs/JSON files and `comparison.json` are in `/tmp/robin-perf-rebased/`.

Native replay validation reaches the matching frame-0 hash but the selected
fixture fails its mission on the first tick, after which the true-headless
adapter panics on unsupported terminal campaign/profile promotion. The same
failure is reproduced with the pre-grid binary. This does not validate replay
through EOF, and must not be reported as doing so.


The separate DEBUG trace (not an idle-host timing sample) records 66 part jobs,
28 VQ jobs and five RLE/JXL jobs. Part worker execution is at most 8 ms per job,
while part queue waits reach 304 ms. Soldier A01 waits 967 ms before a 778 ms
VQ execution interval; WillScarlet waits 950 ms before 806 ms execution.
RobinTown takes 1100 ms after a 165 ms queue wait. These are worker wall-clock
intervals, not sampled codec CPU. They justify investigating shared-pool
scheduling and long chunks before optimizing part decompression allocations.
RLE result receipt is intentionally delayed until its drain phase, so its
receipt lag must not be mistaken for worker execution. Queue/wall values from
this diagnostic trace must not replace the idle paired results above.

The optimized candidate also reaches bootstrap with the worker pool disabled
(`--serial`). A separate `--wait-audio` run completes all 489 background mission
warmup items. No Restart filesystem-creation error appears in candidate runs.


Real CDP input (a 1024x768 viewport, Enter on Mission Lost, then the Restart
seal) verifies that the browser checkpoint restores timeline 1 → 0 without
mission reconstruction. A repeated-cycle regression exposed an additional
process-lifetime bug: terminal leaderboard preparation had been consumed by
the first attempt. Successful restore now re-arms that lifecycle before the
next simulation tick. It creates a fresh browse-only attempt; it does not
reuse the completed attempt's ranked admission. Ordinary mid-mission loads do
not replace unconsumed preparation, and duplicate terminal capture without a
restore remains an error.

The harness now supports `--verify-restart FILE` and `--restart-cycles N`
(default two). It uses actual CDP input, rejects reconstruction fallback and
panics, verifies each rewind to frame 0, and exports the final completed
restored attempt. It also caught a blocked typed-terminal RPC path, now fixed
by distinguishing campaign handoff from an active terminal modal.


Post-terminal Restart now opens a separate replay recording using the exact
original pre-engine header and bootstrap marker. Its first boundary pins and
restores marker 0 before real input; the first recorded engine hash is checked
after that restore. The sole same-ordinal load-back allowed is this initial
0 → 0 boundary with a timeline-0 marker. Other self/future targets remain
invalid. This reproduces persisted-state projection and post-load fixups
without an invented simulation frame or an embedded state payload. Restarted
runs retain explicit state-load taint and do not regain ranked authority.

Native restarted attempts get separate files, including when the first attempt
used an explicit record path. Browser export switches to a fresh in-memory
recording; frozen prior terminal exports remain intact. Arbitrary non-bootstrap
post-terminal loads still require an initial-snapshot replay design: active
export fails explicitly in that case instead of returning the previous attempt.

Integrated validation after these lifecycle changes: 4,452 engine tests,
1,559 client tests, 24 replay-format tests, 36 replay-verifier tests and 23 ranked
verification tests pass (6,094 active tests total, 13 ignored). The new tests
cover two actual checkpoint restores into fresh recordings, immutable prior
exports, compact round-trip, both runtime contracts and corrupt marker/hash
rejection.


#### Final candidate validation and measurements

The final optimized package includes repeated-terminal and replay continuation
fixes and restores per-part yields (source `ec822eed7`; a subsequent diagnostic
formatting change does not alter startup behavior). Real browser input passes
two Restart cycles, each timeline 1 → 0, then another mission end and export of
the new attempt. The same sequence passes through native UI, including dismissal
of the mission-end leaderboard, with graceful exit 0. The final browser package
also passes serial startup and all 489 background audio warmup items.

A new five-pair, alternating-order comparison against the same rebased baseline
runs after all builds and other task test processes finish:

| Endpoint, navigation-relative | Baseline median | Final median |
| --- | ---: | ---: |
| Mission activation | 2.624 s | 2.453 s |
| Recording begins | 3.733 s | 3.544 s |
| Bootstrap complete | 3.902 s | 3.551 s |

All five bootstrap pairs improve. Median paired saving: **271 ms**. Difference
between overall medians: **350 ms (9.0%)**. Recording-to-bootstrap medians are
166.0 → 6.7 ms. Absolute numbers from this batch should not be compared directly
with the earlier candidate batch: host conditions differ. Final all-parts-merged
medians are 404 → 401 ms, VQ-tail 1551 → 1424 ms, level-load 404 → 394 ms, and
frontend assembly 669 → 674 ms. Frontend improvements remain unproven as a bundle
in this fixture; terrain overlap and mask uploads remain useful next targets.

Final run at the median bootstrap endpoint (rounded independently):

| Phase | Wall time |
| --- | ---: |
| Navigation to `wasm_boot` | 378 ms |
| Boot/initialization to streaming | 206 ms |
| Part fetch/decompression/merge, overlapping sprites | 401 ms |
| Remaining VQ wait/application | 1464 ms |
| RLE/activation and level/engine setup | 407 ms |
| Frontend assembly and intervening work | 675 ms |
| Remaining checkpoint/runtime setup | 21 ms |
| **Bootstrap complete** | **3551 ms** |

The frontend interval includes a 323 ms terrain join, 92 ms renderer construction
and 121 ms map upload (100 ms masks). These are nested, not additional costs.
The leading next experiments are bounded dependency-aware worker dispatch,
terrain decoding when its part arrives, independent groups for oversized sprite
chunks, and batched mask textures. Worker-trace execution values remain wall
intervals, not pure codec CPU measurements.

Final raw timelines: `/tmp/robin-perf-rebased/final-comparison-*.json` and
`final-comparison.json`. Browser Restart evidence: `restart-final.log/.json` and
`restart-final.rhrec`; native UI evidence is in
`/tmp/robin-perf-frontend-validation/native-restart-ui2/`. Native playback of its
new recording passes the initial checkpoint pin/restore and post-restore hash
check, then reaches the existing unsupported headless terminal flow; full EOF
replay remains unvalidated in that adapter.


Cross-format playback remains a separate limitation: with matching source
version, the exported browser replay expects bootstrap hash
`d8159306d9d46e0b`, while the legacy native datadir produces
`c031457ca27c5d90`. The browser's earlier first-attempt recording (before the
replay-continuation change) already contains the same `d815...` marker, so this
is not introduced by rotating the Restart recorder. Native initialization of
the identical converted corpus currently fails because it passes an absolute
resolved path to `ShippingDatadir::load_from_vfs`, which requires a relative
mount path. Neither cross-format replay equivalence nor full headless EOF is
claimed by this work. Logs: `restart-final-matched-playback.log` and
`restart-final-shipping-playback.log` in the final artifact directory.

## Independent sprite groups, early terrain and mask uploads (2026-09-08)

Implemented the five follow-up opportunities together, with runtime switches
for the scheduling/terrain/rendering comparisons:

- The streaming loader bounds the combined VQ/RLE queue, reserves capacity
  for short mission-part jobs and an active terrain decoder, and prioritizes
  VQ dependencies by their downstream byte-weighted path. The measured
  `balanced` policy admits one large RLE job before filling VQ capacity;
  VQ-first `bounded` and legacy `unbounded` remain diagnostic alternatives.
- Terrain pixels start decoding when the mission header and exact ambiance
  map arrive. A single-use application cache handoff checks installation,
  mission generation, mission/map/ambiance, and final installed source bytes.
  PNG/reader overrides discard speculative pixels. Cancellation retires the
  result. A browser smoke test caught a broad-generation mismatch caused by
  publishing speech IDs; the regression test now reproduces that exact order.
- The converter defaults to `--vq-group-tiles 1048576 --rle-group-blobs 1`.
  Each whole-grid group restarts its VQ model and recomputes internal temporal
  references, preserving external bases. Existing independent JXL atlases
  become separate jobs without re-encoding pixels. Zero retains the old
  grouping for comparisons; an individually oversized grid stays intact.
- Binary masks upload as raw R8 bytes, with the shader preserving the exact
  zero/nonzero predicate. Atlas pages share texture/view/bind-group handles;
  integer local coordinates preserve nearest sampling and edge clamping.
  Continuous RG8 depth is unchanged. The first atlas prototype regressed
  against sorted standalone uploads; eliminating its full-page alpha
  expansion removed the extra allocation and byte pass in both paths.
- Shipping resource metadata now exports ground-marker opaque bounds and the
  minimap corner's hit mask. Startup reads validated geometry without
  decoding those pictures. Decoded/legacy pictures retain their original
  extraction semantics, including holes and fully transparent frames.

This last change advances shipping schema **15 to 16**, including the compiled
runtime contract and core-overlay inventory. Reconvert shipping data, or use
the offline `robin_assets` example `migrate_picture_metadata` for a trusted v15
boot file. The runtime does not carry the old wire adapter. The benchmark
migration changed no audio/JXL payloads: its normalized boot is 7,968,036 bytes
versus 7,967,940 bytes before. HashMap serialization order means that 96-byte
difference describes this fixture, not a fixed schema overhead.

### Compression and grouping comparison

The three fixture copies use an identical migrated boot file. Rechunking
verified every decoded VQ grid; unsplit regeneration reproduced all 28
original blobs and the original compressed mission-part size exactly.
All nine native corpus checks produced SHA-256
`3e80fe96ac7f5f20a422a8e1d60aeffeb4f28388944acb611da129b80a34b6c4`.

| VQ tile budget / JXL blobs per job | VQ / RLE jobs | Mission-part bytes | Change |
| --- | ---: | ---: | ---: |
| Unsplit / unsplit | 28 / 5 | 14,952,131 | — |
| 262,144 / 1 | 116 / 73 | 16,041,949 | +7.29% |
| **1,048,576 / 1** | **45 / 73** | **15,423,484** | **+3.15%** |

The larger budget won the exploratory optimized-browser comparison and costs
less download space. Native timing measurements were affected by concurrent
builds and were used for correctness/work-size evidence, not a speedup claim.
Leipatch contains 55 independent atlases; its largest remaining individual
atlas is 1,580,256 pixels. Splitting that further is a separate representation
experiment, not safe arbitrary parallelism inside a JXL stream.

### Matched browser results

Fresh Chrome 152.0.7977.64 profiles, four WASM workers, loopback shipping
`Dem_Lei_MP`, `wasm-release` plus Binaryen `-Oz`, same direct-WASM harness.
Baseline source: `f27743e8b`, rebased onto main `9bed99516`. Candidate source:
`347871367`. All task builds had finished before the five alternating pairs.
Host/browser timing still varied, so retain all samples and compare matching
pairs. These measurements precede the subsequent clean rebase onto
`930eebbbc`; the saved artifacts retain the exact measured sources.

| Endpoint | Baseline median | Final median |
| --- | ---: | ---: |
| Navigation → mission activation | 3.099 s | 2.887 s |
| Navigation → recording begins | 4.269 s | 3.745 s |
| **Navigation → bootstrap complete** | **4.276 s** | **3.755 s** |
| WASM boot → bootstrap complete | 3.788 s | 3.183 s |

All five pairs improve: **310–899 ms**, with **520 ms median paired saving**.
The difference between navigation medians is **12.2%**. Absolute navigation
ranges were 4.008–4.874 s before and 3.536–4.450 s after; these are not fixed
latency promises. The WASM-boot-relative medians improve by 16.0%.

Both navigation medians occur in pair 3, whose non-overlapping intervals are:

| Phase | Before | After |
| --- | ---: | ---: |
| Navigation to WASM boot | 488 ms | 593 ms |
| Boot/initialization to streaming | 272 ms | 254 ms |
| Part fetch/decompression/merge, overlapping decodes | 539 ms | 269 ms |
| Remaining critical sprite work and installation | 1801 ms | 1879 ms |
| Level/engine setup and intervening work | 463 ms | 392 ms |
| Frontend assembly and intervening work | 691 ms | 341 ms |
| Remaining checkpoint/runtime setup | 23 ms | 27 ms |
| **Bootstrap complete** | **4276 ms** | **3755 ms** |

The sprite tail now includes RLE work on the bounded paths. It must not be
compared in isolation with the old VQ-only timer or added to overlapped work.
Small phase timers log at DEBUG below 50 ms. The final optimized diagnostic
run measures a **10 ms terrain join** and **11 ms mask upload**, versus the
baseline five-run medians of 311 and 104 ms respectively. Those diagnostic
values are not another five-run median. The six mask pages contain 21,261,316
occupied texels in 22,761,472 uploaded texels (93.4% occupancy).

A separate three-round same-package ablation supports the chosen defaults:

| Configuration, 1M-tile groups | Median navigation → bootstrap |
| --- | ---: |
| **Balanced + early terrain + atlas** | **3.171 s** |
| VQ-first bounded scheduling | 3.432 s |
| Legacy unbounded scheduling | 3.665 s |
| Late terrain | 3.520 s |
| Standalone masks, retaining raw-byte uploads | 3.284 s |

These are interactions on a variable host, not additive independent savings.
Exported picture metadata was not separately isolated. In the final DEBUG
worker trace, RLE finishes before the VQ tail; part queue median/max is 2/12 ms,
and VQ/RLE queue median/max is 0/1 ms. Worker execution intervals include
scheduling effects and are not pure codec CPU measurements.

Validation: 4,464 engine, 1,583 client, 152 asset and 22 converter tests pass;
the pure-assets feature configuration passes 76 tests. Vulkan and OpenGL
exact-pixel contracts cover atlas versus standalone masks, high origins,
1-pixel widths, fractional placement, far-outside UV, noncanonical nonzero
bytes, depth, and main's portrait ownership checks. The native binary builds
and the exported runtime contract matches the checked-in JSON. Intentional
unwind tests require LLVM cleanup and remain explicitly ignored under the
normal Cranelift test profile.

The measured optimized package also passes serial startup, all 489 background
audio warmup items, two actual terminal → Restart cycles, and fresh compact
replay export. The clean rebase to main `930eebbbc` retains all 42 commits;
`8cd529f26` is the rebased code snapshot. Its client suite, both GPU contracts,
native binary build, and compiled runtime contract check pass again. The rebuilt
optimized WASM package also passes startup (confirming the early-terrain
handoff), serial fallback, all 489 audio warmup items, and two actual Restart
cycles with compact replay export after this rebase.

Artifacts: `/tmp/robin-perf-next/` contains `provenance.json`, normalized
`corpus-{unsplit,262k,1m}`, `groups-late-*`, `policy-late-*`, `final-ablation-*`,
`final-comparison-*`, `validation-*`, `post-rebase-*`, and `final-restart.rhrec`. Packages
`baseline` and `final` preserve the exact measured sources. The harness accepts
`--query streaming-scheduler=balanced|bounded|unbounded`,
`--query early-terrain=0`, `--query mask-atlas=0`, and `--viewport 1024x768`.
`robin_assets`'s `rechunk_sprite_groups` example creates verified experimental
copies; the production converter writes the selected groups normally.

TODO: measure hosted network and hardware-GPU startup separately. This direct
WASM harness uses loopback, a fresh profile, software WebGL, and an initially
1×1 surface; it bypasses the production TypeScript loader. The endpoint is
bootstrap completion; first-frame presentation is not measured. The 3.15%
mission-byte cost can offset decode savings on a bandwidth-limited connection.
Worker wall times and overlapping phases must not be added as CPU savings.

## Production-loader startup and bandwidth scheduling (2026-09-08)

This round measures the actual built TypeScript frontend, compressed WASM path,
and normal demo auto-start. The historical direct `--mission Dem_Lei_MP` fixture
forced a different starting campaign: it spawned one PC, fetched 66 parts
(15,423,484 bytes), and failed after bootstrap. Normal demo launch selects the
demo party, spawns four PCs in this corpus, and fetches 72 parts (20,129,803 bytes).
Its existing missing-Ferris profile diagnostic is retained. Do not interpret the
larger normal-demo/network timings as a regression against the earlier loopback
forced-mission table.

Implemented changes:

- Mission transfers retain all-at-once admission but use priority ordering.
  Mission headers and terrain come first; required audio metadata remains in the
  activation closure. A scoped session-owned pause prevents new speculative audio
  warmup requests until the parts merge. Actual playback bypasses the pause,
  already-started requests finish, and cancellation/error/retirement release waits.
  Same-package controls: `mission-downloads=unbounded` restores alphabetical order,
  `mission-downloads=prioritized` caps admission at eight, and
  `audio-downloads=eager` disables the pause. A two-round experiment found the
  eight-request cap about 126 ms slower than alphabetical all-at-once admission;
  it was therefore not selected as the final default.
- The ordinary VQ decoder level is inlined separately from the uncommon exclusion
  scans, allowing constant-level specialization without changing the encoded stream
  or grouping. Candidate selection used native perf samples and retired-instruction
  counts; discarded singleton and generic-inline prototypes showed no benefit.
- Engine door endpoint resolution borrows its static grid instead of retaining an
  Arc clone across mutation, avoiding an otherwise forced full grid copy. A
  regression preserves allocation identity and exact gate lists. New timers split
  initial sprite/opacity work out of the misleading old engine-construction bucket.
- Renderer pipelines are prepared after initial streaming progress or transferred
  from an existing loading renderer. Handoff clears queued/frozen/cached pixels and
  effect state; a pre-world Lost-Sherwood modal regression checks the retained
  targets cannot expose loading artwork. `renderer-preparation=late` restores late
  preparation and discards the loading renderer for comparison.
- Unopened menu surfaces retain validated RGB565 data, dimensions and hit masks,
  postponing conversion and GPU texture upload until first draw. GPU contracts
  cover five drawing modes, repeated opening and pending/resident ownership.
- Production measurement exposed an additional first-frame cost: synchronous
  browser GPU polling timed out while capturing the initial autosave thumbnail.
  That caller now awaits nonblocking polling with browser event-loop yields.
  Native capture remains unchanged; a 30-second failure bound reports a stuck
  callback. Synchronous screenshot callers can still return AsyncRequired when
  mapping needs a later turn; migrating that broader screenshot stack is a TODO.

Full engine construction was not moved before sprite activation. Most of it
requires a closed, activated mission catalog, canonical audio dependencies and
published immutable sprite generation. A further audit identified pure legacy
pathfinder-graph parsing as a separable preparation stage; a new DEBUG timer
measures that subset rather than treating the entire constructor as overlapable.
The final optimized DEBUG run parses its 127,854 bytes in **5.04 ms**; a new
worker/handoff is not justified by that cost in this fixture.
TODO: if its measured cost warrants it, add a validated, single-use preparation
handoff with mission/installation identity, cancellation, and graph-source checks.
This round removes the confirmed grid copy without constructing incomplete inputs.

No shipping schema, mission bytes, or decoded pixels change in this round.
A pre-existing non-threaded WASM compile issue in worker timing diagnostics was
also fixed: js_sys-dependent timestamps now require the wasm-threads feature.

### Measurements

Five alternating baseline/final pairs used fresh Chrome 152.0.7977.64 profiles,
four WASM workers, normal demo auto-start and the same immutable converted
corpus. All task builds and optimizers had finished before sampling; other host
activity was not controlled. All samples are retained.

| Endpoint / mode | Baseline median | Final median |
| --- | ---: | ---: |
| **Bootstrap, 16 Mbit/s** | **20.136 s** | **19.662 s** |
| Screenshot complete, 16 Mbit/s (settled image, not first presentation) | 21.511 s | 20.850 s |
| Bootstrap, unshaped production loader | 4.287 s | 4.223 s |

At 16 Mbit/s, four of five matching pairs improve: savings are 563, 474, -102,
574 and 170 ms. The median paired saving and difference of medians are both
**474 ms (2.35%)**. Baseline ranges 20.098–20.190 s; final ranges 19.593–20.218 s.
Both median launches occur in pair 2, with these non-overlapping intervals:

| Phase | Before | After |
| --- | ---: | ---: |
| Navigation through module/boot loading and boot decode | 8,336 ms | 8,358 ms |
| Initialization to mission streaming | 157 ms | 168 ms |
| Part transfer/decompression/merge, overlapping sprite decoding | 10,455 ms | 10,140 ms |
| Remaining critical sprites and installation | 368 ms | 294 ms |
| Level, engine, frontend and runtime setup | 820 ms | 703 ms |
| **Bootstrap** | **20,136 ms** | **19,662 ms** |

Final first-mission-present-return is a 20.135 s median in this run set; baseline
has no corresponding marker. It must not be subtracted from baseline bootstrap
as a comparative first-frame measurement. All final runs retain game screenshots.

Transfer remains dominant. Recorded payload bytes at bootstrap are 36,992,122
before and 36,999,284 after; both include 882,037 audio-category bytes. Pausing
speculative audio changes timing rather than the amount ultimately transferred
before bootstrap. In the earlier same-package two-round controls, eager audio
was about 331 ms slower than paused audio; the renderer-preparation control was
within noise (about 33 ms in the opposite direction). These small experiments
do not establish additive per-feature savings. The earlier eight-request package
had a 19.580 s median versus its paired 20.128 s baseline, but a separate
all-at-once control beat the cap. The final default therefore retains prioritized
all-at-once admission rather than claiming eight is an optimal transfer limit.

The five unshaped pairs are inconclusive for overall startup: baseline ranges
3.749–4.671 s and final 3.744–4.777 s. Three pairs improve; paired savings are
-490, -155, 448, 5 and 248 ms, giving only **5 ms median paired saving** despite
the 65 ms difference of medians. This does not support a robust unshaped startup
speedup. The earlier eight-request candidate likewise varied substantially
(5.172 → 5.061 s medians in its separate five-pair set). Do not pool those
separate sampling windows or attribute full-startup changes to isolated VQ alone.

The isolated VQ comparison used ten alternating fresh-profile serial Chrome
runs after task builds finished. Both packages decoded 45 chunks / 47,179 sprites
with identical FNV `589daa9f51ba962e`. Median materialization fell from **4,150 to
3,848 ms (7.28%)**; all five matching pairs improved, with a 203 ms median paired
saving. This isolates serial VQ materialization, not four-worker game startup.
Native corpus SHA-256 remained
`3e80fe96ac7f5f20a422a8e1d60aeffeb4f28388944acb611da129b80a34b6c4`.
The native perf profile attributed 22.96% of samples to `decode_level`; retired
instructions fell about 2.6–2.9%. Concurrent-build native wall times were not used
as performance evidence.

The measured final code is `5db35596c`, compared against the prior rebased code
`8cd529f26` (documentation snapshot `db860eeee`), both on main `930eebbbc`.
Final optimized WASM is 21,446,680 bytes, versus 21,424,388 before. With the
fixture's reproducible `gzip -9 -n`, it is **7,721,211 versus 7,714,049 bytes**:
7,162 extra bytes, about 3.6 ms of transfer at 16 Mbit/s. The final WASM SHA-256 is
`71150073aa6bb49a6c35f5494a5857459aea6642da8f4a1b1d3a73f873273373`.
The 72 mission parts and boot manifest are byte-identical between packages.

Validation: the integrated client suite passes 1,586 tests (7 ignored); the
engine suite passes 4,464 (4 ignored). The final scheduling change also passes
all 14 focused mission-loading tests. The full release assets suite passes,
as do 11 codec tests with each exclusion cap (32 and 256). Browser lifecycle
checks pass 26 tests, including nested audio pauses, cancellation and session
retirement; non-threaded WASM builds pass with `audio` and `audio,multiplayer`.
Final loading-handoff exact-pixel tests pass software Vulkan and OpenGL. Native
and optimized threaded WASM builds, production frontend build/typecheck, three
throttle tests, formatting and syntax checks pass.

The final optimized package passes normal demo startup, serial fallback, audio
warmup completion, and two actual terminal → Restart cycles with a fresh compact
replay export. The DEBUG production run and the median 16 Mbit/s screenshot were
visually checked: both show the Leicester world and opening briefing. There is
no thumbnail polling timeout in the final diagnostic trace.

Artifacts: `/tmp/robin-startup-next/` contains `final-source.json`,
`integration-provenance.json`, immutable `candidate` and `final` packages,
`final-comparison-{16,unlimited}-*` (all logs, JSON, screenshots and analyses),
`final-package-debug.*`, `final-validation-*`, and `final-restart.rhrec`.
Earlier eight-request measurements are retained as `comparison-*`; two-round
controls are `ablation-*`. The baseline package remains
`/tmp/robin-perf-next/rebased`. Isolated VQ evidence is in
`/tmp/robin-startup-vq/`, including `provenance.json`, `wasm-results-0.json`,
retained packages, decoded hashes and native perf data. Browser lifecycle gate
evidence is `/tmp/robin-startup-downloads-browser-gate-fixed/summary.json`.

The harness is `scripts/wasm_production_startup_chrome.mjs`; its adjacent Markdown
file describes reproduction and endpoint limits. One shared server queue shapes
all response payloads, including worker requests, to 2,000,000 B/s at 16 Mbit/s.
This is fresh-profile loopback HTTP/1.1 with software SwiftShader, no added RTT,
packet loss or TCP/header cost, not a Cloudflare HTTP/2 or HTTP/3 simulation.
`--mbit unlimited` bypasses pacing through the same production loader.
Bootstrap is an engine log endpoint. The first-present marker is return from a
normal mission render/present call, not GPU/compositor completion. Screenshots
are inspectable rendered-game evidence after bootstrap, two RAFs and a 500ms
settle, not a timestamp of the first physically displayed frame.


## Replay startup and boot payload reduction (2026-09-08)

This round follows the production-loader measurements above, with the URL replay
entry path as the target. Replay admission, command acceptance, bootstrap and
first mission presentation are separate endpoints. The previous normal-demo
bootstrap numbers are not measurements of this replay path.

### Boot inventory and verified audio removal

The actual baseline `Data/datadir.bin` is **7,968,036 B**. Removing redundant
locale WAV payloads produces **3,702,350 B**, saving **4,265,686 B (53.53%)**.
The decoded native-bitcode payload falls from 19,773,549 to 13,718,305 bytes.
All 606 removed files (6,021,324 source bytes) have catalog-backed playback and
match the source selected for conversion byte for byte. Distinct translations
and unmapped audio remain. This applies to the browser Opus publication recipe;
native/source recipes retain their original resources.

The probe verifies all 1,095 external audio catalog ranges, unchanged audio
references, unchanged mission references, and semantic encode/decode parity.
HashMap iteration can change re-encoded bitcode ordering, so a re-encoding estimate
must not replace the actual input file size. Production conversion trims before
publishing its content inventory and hashes. A scratch boot rewrite alone does
not refresh an authenticated web-content manifest.

Local evidence: `/tmp/robin-startup-more/boot/{inventory.tsv,trim-report.json}`
and `datadir-trimmed.bin`; input is
`/tmp/robin-perf-next/corpus-1m/Data/datadir.bin`. These are session-local artifacts,
not repository fixtures or hosted downloads.

### WASM transport: offline ratios versus HTTP responses

For the 21,446,680-byte baseline WASM, the offline Node Brotli quality-11 probe
produces **5,418,754 B**. This is a compression experiment, not the payload observed
from the production HTTP path. The local Wrangler HTTP Brotli capture is
**6,617,749 B**, versus the fixture's reproducible system `gzip -9 -n` sidecar
at **7,721,211 B**. Node's gzip probe produces a different 7,782,556-byte stream;
keep those recipes separate. An older deployed build has its own captured
ratio and cannot stand in for this build.

The tested Chrome 152 lacks `DecompressionStream("brotli")`, while native HTTP
Brotli decoding successfully feeds `WebAssembly.compileStreaming`. The loader
therefore uses negotiated HTTP WASM compression on that browser, with the gzip
fallback retained; optional raw Brotli streams require actual API support.
Setting `Content-Encoding: br` on a precompressed static sidecar was rejected:
the tested Wrangler path transformed it again, leaving compressed bytes after
HTTP decoding. No such header override is part of the selected implementation.

Local evidence: `/tmp/robin-startup-more/wasm/compression.json`,
`captured-baseline.br.json`, `chrome-http-compile.json`, `live-compression.json`
and `transport-findings.md`. Offline size savings are not browser latency claims.

### Replay work and engine diagnostics

Replay admission loads alongside the main runtime. The production package now
includes its isolated admission module and verifies the published WASM memory
contract: owned, nonshared memory capped at 6,144 pages (384 MiB). Replay build
identity remains mandatory. The matching baseline admission module was built
from `5db35596cc88c27fbcc9568a0545c070be6e5076`; it accepts that build's compact
sample and rejects a substituted identity. This makes the baseline replay URL
exercise real admission instead of substituting a bypass.

Playback skips boot menu-audio prefetch and menu music, while retaining mixer
initialization, volume/mute policy and mission audio. It also skips live Restart
save creation during replay initialization. Immutable replay frame maps are
shared through `Arc` instead of deep-cloned. These changes preserve replay
commands and identities; they do not remove deterministic replay verification.

Native mask decoding uses spans, and pathfinder initialization avoids redundant
default construction. Sampled retired-instruction diagnostics support those
local optimizations: the mask routine's samples fall from 2,933 to 220 and
motion initialization from 3,040 to 309 in the retained profile. These are native
profiling counts, not browser milliseconds or independently additive startup
savings. Source and profile conditions are recorded in
`/tmp/robin-startup-more/engine/{provenance.json,instruction-summary.json}`.

Baseline admission provenance and functional validation are in
`/tmp/robin-startup-more/replay-baseline-build.json` and
`replay-baseline-admission-test.json`; the isolated package is
`replay-baseline-pkg/`. Its one-shot Node validation time is not a browser startup
measurement.

### Sprite pixel deferral experiment: removed

The former threaded-browser experiment `?sprite-residency=first-frame` retained
complete simulation opacity masks in the initial payload and deferred selected
pixel data. It has been removed at the user's request because of the additional
download and slower playback. Its partitioner, mask format and presentation
waits have also been removed. The following measurements describe the historical
build `c7244b5ddca8`; the query flag is no longer supported. The ordinary corpus
never included these extra masks, so removal does not save another 10 MB on
the default path.

Across the 27 eligible parts, initial compressed bytes fall from **16,635,002 to
12,419,123**, a **25.34%** reduction. Deferred tails add **14,182,095 B**, so the
combined payload is 26,601,218 B: **9,966,216 B more** than the original. This is
an initial eligible-parts saving, not a whole-startup or total-download saving.
Exact grid hashes and initial-plus-tail metadata parity are checked. The browser
comparison below confirms a faster first pose but substantially slower playback.

Local corpus evidence is
`/tmp/robin-startup-more/mission/first1-opacity/partition.jsonl` and its `Data/`
payloads. The historical implementation validated publication conflicts and
preserved failures across session changes.

### Final validation and build provenance

The integrated runtime is built from `c7244b5ddca81ebcafd92a9440cd3f553c369912`,
after merging main's `92c95e04c` ownership and mission-stage refactors. Its
optimized threaded WASM is 21,511,574 B (SHA-256
`60d96a212aed25d59ec746c3b93314e86dd08f5eb86ba0c92eb7e6a05c8ce6f5`).
The actual captured HTTP Brotli body is **6,639,157 B**; reproducible gzip is
7,748,552 B, and offline Brotli quality 11 is 5,435,816 B. The module itself
is slightly larger than the baseline; the measured transfer improvement comes
from the transport selection. The isolated validator is 1,163,616 B, served as
315,768 B of captured HTTP Brotli, with its matching build identity checked.
Package/capture hashes are in `final-package-provenance.json`,
`final-admission-check.json`, and `wasm/final-*-http.br.json` under the local
evidence root `/tmp/robin-startup-more/`.

The fixture was recorded by the actual baseline browser runtime using 300 manual
forward steps, then exported with 306 replay records. The baseline and final
build both play to record 306, pause at EOF, and retain the expected logical
frame 305 without a hash mismatch. For the cross-build benchmark only the compact
envelope build identifier is explicitly repinned; its compressed recorded payload
is unchanged. Admission and simulation verification remain enabled. Provenance
and the original payload are retained in `replay-fixture/`.

The short Restart recording additionally exposed a baseline terminal bug: playback
created a second local campaign update after consuming the recorded one. Playback
now uses the recorded timestamp/nonce and terminal command, including pre-command
or delayed multiplayer echoes. Live play still emits its single command. The
original duplicate assertion remains. The final browser replay restores frame 0,
consumes all six records, and displays Mission Lost without that panic.

Validation includes the merged engine/replay/client suites (1,603 client tests
before the two added terminal regressions), 166 asset tests, seven terminal tests,
150 manifest-tool tests, 102 run-protocol tests, and 389 frontend tests. Native
build, threaded release WASM build, Vulkan/OpenGL rendered ownership/capture gates,
and the production frontend build pass. A frozen-source browser lifecycle gate
also passes all **28** tests, including audio, multiplayer protocol, identity and
persistence, with both audio-only and audio-plus-multiplayer WASM target checks.
Evidence is `final-browser-lifecycle-frozen/summary.json`; the earlier gate attempt
is retained as rejected because source changed during that attempt.

### Final replay comparison

Five alternating baseline/candidate pairs at each rate (20 runs total) all pass;
every candidate is faster than its paired baseline. All 279 input files remain
unchanged across the series. Fresh Chrome 152 profiles, four decode workers,
software SwiftShader, the production frontend and actual replay URL are used.
No task builds or profiling run during the timing series. The 16 Mbit/s model
shares 2,000,000 payload bytes/s across requests on loopback HTTP/1.1 with no added
RTT, packet loss, or TCP/header cost. It is not a live-CDN latency estimate.

| Rate | Endpoint | Baseline median | Candidate median | Median paired saving |
|---|---|---:|---:|---:|
| 16 Mbit/s | Bootstrap | 19.866 s | 16.998 s | 2.863 s |
| 16 Mbit/s | First mission present returned | **20.010 s** | **17.137 s** | **2.874 s** |
| Unlimited loopback | Bootstrap | 3.928 s | 3.800 s | 100 ms |
| Unlimited loopback | First mission present returned | **4.059 s** | **3.930 s** | **93 ms** |

The first-present medians improve by **14.36%** and **3.18%**, respectively.
The median of paired savings differs from the difference of the two medians;
the table labels that statistic explicitly. First-present return is a submission-
side endpoint, not GPU/compositor or physical display completion. Screenshots
and actual replay RPC state confirm the rendered mission and unpaused playback.

Adjacent intervals for each arm's median-first-present **16 Mbit/s** run follow.
These form an additive waterfall; overlapping worker, renderer and engine
subspans must not be added to it. WASM boot start is inferred from its measured
decode duration and log timestamp, with console-delivery rounding. Mission-stage
refactors can move work between level-load and frontend-assembly intervals.

| Interval | Baseline | Candidate |
|---|---:|---:|
| Navigation → inferred WASM boot | 8,244 ms | 5,769 ms |
| Boot → mission streaming | 667 ms | 317 ms |
| Streaming → all parts merged | 10,119 ms | 10,119 ms |
| Remaining decode/install | 228 ms | 222 ms |
| Installation → level loaded | 396 ms | 190 ms |
| Level loaded → frontend assembled | 191 ms | 365 ms |
| Remaining bootstrap | 22 ms | 16 ms |
| Bootstrap → first present returned | 144 ms | 139 ms |

Pre-present payload drops **37,153,972 → 31,095,658 B**, a **6,058,314 B** saving:
4,265,686 B from boot, 1,082,054 B from main WASM transport, and 711,419 B from
speculative menu audio, offset by small shell/admission changes. The baseline
actually requests its 7,721,211-byte gzip WASM sidecar; the candidate requests the
6,639,157-byte captured HTTP Brotli representation. Admission uses captured HTTP
Brotli in both. Required audio metadata remains, while the candidate requests no
Opus payload before first present. The remaining roughly ten-second mission
stream is the dominant 16 Mbit/s cost; CPU-side gains are much smaller than the
transfer savings in the whole replay path.

The baseline frontend did not expose admission/module-ready timing marks. Those
endpoints remain unavailable rather than being inferred from transfer completion.
Candidate admission/queue marks, per-run overlapping spans, exact byte categories,
all paired samples and input hashes are retained in
`/tmp/robin-startup-more/replay-matched/{summary.json,inputs-before.json,inputs-after.json}`.
The adjacent harness documentation explains reproduction. Final normal and Restart
EOF evidence is in `replay-fixture/final-eof.*` and `final-restart-terminal.*`.
Main's subsequent `7be45078e` follow-up was merged as `0423e45e9`; game, engine,
assets, replay, frontend and Cargo inputs remain byte-identical to the measured
`c7244b5dd` source. Only parity tooling and audit documentation changed.

The sprite-deferral follow-up uses two sequential samples per mode at 16 Mbit/s,
with the same final package, replay and HTTP captures. Ordinary-corpus controls
bookend the series; eager/deferred partitioned runs alternate. Both partitioned
modes use the same complete simulation masks and tail references, with identical
trimmed audio and unchanged source corpora.

| Mode | Median first present returned | Median first observation of replay record 10 |
|---|---:|---:|
| Normal corpus (selected default) | **17.151 s** | **17.894 s** |
| Partitioned corpus, eager pixels | 22.071 s | 22.831 s |
| Partitioned corpus, deferred pixels | 14.971 s | 22.842 s |

Deferral shows the first pose 2.179 s earlier, but reaches record 10 **4.947 s
later** than the ordinary corpus. Both deferred runs stall behind required sprite
pixels for about 3.3 s and then 3.8 s. Eight seconds after first presentation,
deferred playback is only at record 14, versus 152–154 for the controls. RPC
samples record their real delayed reply times; these waits are not treated as
on-time playback. All runs preserve correctness, and the experimental path also
passes all 306 records to EOF, but this transfers latency into playback stalls.
**Keep the normal corpus and eager pixel path as the default.**

The partitioned boot retains tail references and is 3,702,731 B (381 B larger
than the ordinary trimmed boot). The experiment is now removed; its original
implementation remains in git history. Full input hashes, six samples, wait logs and delayed
RPC measurements are retained under
`/tmp/robin-startup-more/experimental-final-progress/`; the matched corpus is
`/tmp/robin-startup-more/corpus-first-frame/Data`.

## Fabri18 PNG mod: shipping VQ families and JXL map (2026-09-10)

The first custom-mod encoder used the shipping entropy codec but restarted
both its dictionary and model every 60,000 tiles, with no family or self
references. Its complete Fabri18 gallery ZIP was **124,071,515 bytes**.
This was not the full shipping compression recipe.

The replacement keeps one exact, frequency-ranked four-pixel dictionary per
character, selects two family hubs using sampled conditional entropy, and
uses `sprite_groups::encode_vq_groups` with the production 1,048,576-tile
budget. Standalone hubs derive temporal/adjacent-direction references from
their RHS metadata; other members predict against the two hubs. Family files
carry the production `ShippingSpriteBank` and decode through its existing
dependency-aware materializer. Unlike a family-unified dictionary (the
negative result above), each character retains its own alphabet. The custom
PNG hub proxy supports 16-bit indices rather than requiring 12-bit indices.

Fabri18's nine families contain 51 characters / **266,984 unique frames**.
Their exact dictionaries contain about 4,000 entries each. These particular
recolours are much more predictable than the independently rendered retail
colour families: most non-hub streams require only 24–38 KB per character,
and cavalry variants roughly 67 KB, including model restarts. These numbers
must not be projected onto the retail corpus.

The 2508×2508 OpenBattlefield PNG becomes an **883,998-byte JXL** using
`cjxl -q 80 -e 7 --num_threads=4`, stored as `OpenBattlefield.map` so the
ordinary terrain loader detects its JXL signature. The PNG override is omitted.
The minimap, preview, mission descriptor, and soldier stats remain unchanged.

| ZIP component (compressed entry bytes) | First version | Family VQ + JXL |
|---|---:|---:|
| Sprites | 112,102,948 | 17,368,395 |
| Battlefield map | 11,774,190 | 884,142 |
| **Complete ZIP, including metadata and ZIP overhead** | **124,071,515** | **18,435,980** |

The new ZIP is **17.58 MiB**, **85.14% smaller**. The nine family files total
17,378,750 bytes before ZIP. All decoded RGB565 sprite pixels, dimensions,
and animation fields were checked against the original PNG import. The
shipping terrain decoder verifies map dimensions and full decode; an
independent `djxl` decode was visually compared with the source (RGB PSNR
33.82 dB). Sprite conversion is lossless; JXL quality 80 terrain is lossy.
ZIP CRCs and every extracted entry were also checked against the staged files.

Build and reproduce with:

```sh
cargo build -p robin_modding_tools --bin encode_mod_sprites
target/debug/encode_mod_sprites mods/fabri18-sprite-gallery OUTPUT
```

The example also supports `--family OUTPUT INPUT_RHS_DIR...` for independent
family jobs and `--map INPUT_PNG OUTPUT_MAP`. Its three-member family test
passes, including exact reconstruction and preservation of other authored
assets. The stale `ui_task_state.rs` test initializer was subsequently
corrected to use `None`, allowing the client tests to compile.

Local artifact: `.tmp/fabri18-sprite-gallery-vq-jxl.zip`; SHA-256
`b615e8a9fa9867bb47ef1177082799e620a4e661b129fb2c2c536f0319b2c631`.
The exact byte ledger is `.tmp/fabri18-vq-v2-report.json`.

The regenerated archive `.tmp/fabri18-sprite-gallery-vq-jxl-flat.zip` places
`details.json` and `Data/` directly at its root and includes direct-ZIP install
instructions. It is **18,435,228 bytes** (17.58 MiB), with identical sprite and
map payloads. SHA-256:
`76c1c484d8c73e4d04639665c71a1d9a145cff352493fd8f7f12d813eb12375a`.
ZIP overlay support now includes JSON mission discovery, profile patches, PNG
characters, and both custom VQ encodings without extraction.

The opt-in `fabri18_archive_matches_directory` client test takes
`FABRI18_MOD_DIR` and `FABRI18_MOD_ZIP`, verifies nine families and 266,984
frames, and compares decoded runtime sprite/profile digests for both storage
forms. Synthetic regressions also exercise the shared runtime loader with
flat and wrapped archives, mission-scoped selection, and missing PNG errors.

The flat package also explicitly binds each soldier addition to the animation
profile in its authored manifest. Previously the mod inherited that name from
the retail stats template, which caused the green officer to request a missing
RHS profile during the demo-data smoke run. Combat stats and sprite bytes are
unchanged. Validation passed: 37 data-I/O tests, 1,906 client tests (16 opt-in
tests ignored), and the full 266,984-frame directory/ZIP comparison.

The gallery also uses retail enemy squads absent from the Leicester demo
(e.g. `Officier B00.rhs`); full mission launch requires full-game data. ZIP-only
loading was separately exercised using the native headless game.

The isolated ZIP-only full-game headless smoke run loaded all 51 custom
profiles, the JSON mission and JXL terrain, and advanced beyond 500 simulation
ticks before being stopped intentionally. No archive extraction was used.

### RFC 6902 package update (2026-09-10)

Following the JSON Patch merge (`12552c5f2`), the current package is
`.tmp/fabri18-sprite-gallery-vq-jxl-json-patch.zip` (**18,437,937 bytes**,
17.58 MiB). SHA-256:
`5264903ab9fc9cc5bde62401536f515bc5f91c7707cab50cc22a2c607bb3fd4f`.
It replaces the removed `soldier-profiles.patch.json` with 579 standard
operations in `Data/Configuration/profiles.patch.json`, using the migrated
source-mod patch plus explicit authored animation-profile names for all 51
additions. The mission descriptor also explicitly marks the 51 Fabri18 allies
with `command_interface: "tactical_orders"` and `mission_role: "tactical_ally"`;
friendly allegiance alone does not enable tactical control. The 42 enemy
soldiers, sprites, JXL map and other payloads are unchanged. The profile patch,
mission descriptor and README differ from the previous flat archive.
The archive remains flat and is installed directly into `mods/`.

The current Rust `cpf_to_json --patch` loader accepted the patch
against both GOG full-game and Leicester demo CPF catalogs. All 51 final
profile objects exactly match the prior package, including progression stats
and explicit animation names; existing soldier profiles remain unchanged.
The rebuilt native game also applied this patch directly from the ZIP and
advanced the gallery through 100 simulation ticks with full-game data; the
smoke process was then stopped intentionally.
