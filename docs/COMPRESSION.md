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

The production converter path for the artifact named `v4-q80.rhdata.zst` is:

```
convert_datadir --format shipping \
  --map-format jxl-q80 \
  --zstd-window-log 30
```

Only map quality is lossy. Interface pictures are left in the raw RGB565
shipping representation so transparent/keyed UI art remains exact. The
generated v4 artifact is 35 213 242 B.

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

Shipping format v4 applies the trimming recommendation without breaking the
compression properties measured above. The converter now emits a file tree:

```
Data/datadir.bin
Data/missions/<mission>-<content-hash>.rhmission.zst
Data/rhs/<rhs>-<content-hash>.rhmission.zst
```

`datadir.bin` is the boot manifest: profiles, shared UI/text resources, level
descriptors, the sprite-bank dictionary/index shape, and a mission dependency
graph. Each mission file contains its parsed level and script, terrain/minimap,
and loading resources. Each RHS file contains that character/accessory's RHS
metadata, raw compatibility copy, and only its reachable sprite-bank slots.

RHS payloads are intentionally **not** split into one file per sprite. The
measurements in this document show that hundreds of small related sprites gain
substantially from cross-sprite zstd matching, and a file per sprite would also
pay format and HTTP overhead for every frame. One zstd stream per RHS keeps
those within-character matches while allowing heroes, accessories, and common
effects to be shared by several mission dependency lists. Browser requests for
a mission's missing files run concurrently, and decoded files stay cached for
later missions. The content hash in each filename makes those cached files safe
to reuse across shipping-manifest updates.

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
