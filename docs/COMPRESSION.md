# Compression investigation — sprites and maps

Summary of a benchmark sweep looking at whether we can shrink the shipping datadir. Tools: `crates/robin_rs/examples/sprite_size_bench.rs` (codec sweep), `crates/robin_rs/examples/datadir_breakdown.rs` (where-does-the-shipping-blob-budget-actually-go), `cargo run --bin convert_datadir -- --map-format jxl-{lossless,q90}` (the actual production conversion). Data: `datadirs/fullgame_gog` and `datadirs/demo_leicester_ecoste`.

## TL;DR

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

- **Background maps** (`Data/Levels/*/<name>.map`): `SBPictureSixteen` format — bzip2-compressed rectangular RGB565 pixels. Sizes 1.6–8.8 MiB, dimensions 1408×960 to 2304×3520.
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
   simple enough that pulling in the C++ framework is not warranted for this
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
materialize (12.3 -> ~2-4 s; needs an atomics build plus COOP/COEP via
a service worker on GitHub Pages); overlap chunk decode with the fetch
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
download/compile, assets, datadir, boot) and, for static hosts, the
coi-serviceworker so threads work on GitHub Pages after one automatic
first-visit reload.

## Shipping integration: schema v14 — runtime locale overlays (2026-08-29)

Datadir schema v14 (`RHDDNA14`) adds the canonical per-locale resources and
raw byte maps used by runtime language switching. The serialized form keeps
owned `Vec<u8>` values; loading converts those bytes to the VFS's shared
`AssetBytes` representation only at the atomic locale-mount boundary. This
keeps the on-disk manifest portable while avoiding repeated runtime copies.

Mission payloads are unchanged at `RHMISN07`/v7. Because bitcode is not
self-describing and the top-level datadir shape changed, older datadir
manifests are rejected with a regeneration error instead of being decoded as
the new shape or assigned an invented locale identity.
