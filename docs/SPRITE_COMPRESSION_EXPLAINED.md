# How the sprite compression works

A standalone explanation of the sprite compression pipeline built for the
Robin Hood port in August 2026 — what the data is, why general-purpose
compressors leave half the bytes on the table, and how the custom codec
recovers them. Measurement provenance and the full experiment log live in
`docs/COMPRESSION.md`; this document explains the *ideas*.

## 1. What we are compressing

The game's characters are pre-rendered 3D: every character was rendered once
per **action × animation frame × 16 camera directions**, producing 4,000-8,000
unique small images per character (RobinTown: 7,584 frames, average 37×56
pixels). Pixels are RGB565 (16-bit color) with two magic values: a transparent
key (0x07C0) and a shadow key (0x001F).

Three generations of compression have touched this data, and it matters to
keep them apart:

| era | what it did |
|---|---|
| **original game (2002)** | invented the sprite *representation*: RLE and VQ (below). The bank then shipped essentially raw — 602 MiB on disk for the fullgame |
| **this port, schemas v1-v8** | added the first *entropy stage*: bitcode serialization + zstd-22 over the bank, plus JXL maps, Opus audio, and per-mission chunking |
| **this campaign, schema v9** | replaced zstd for the VQ grids with the custom codec this document explains |

The original engine's two sprite representations:

- **RLE sprites** (menus, patches, accessories): each scanline stores
  `[first_x, last_x, literal pixels…]`, skipping transparent margins.
- **VQ sprites** (all characters): *vector quantization*. Each character gets
  a **dictionary of 4,096 tiles**, where a tile is 4 horizontally adjacent
  pixels. A sprite is then a grid of `(width/4) × height` dictionary indices.
  Twelve bits describe four pixels — 3 bits per pixel before any entropy
  coding, and the dictionary itself (32 KB) carries all the color knowledge.
  This was a smart 2002 trade: decode is a table lookup, and the quantizer
  ran once at authoring time.

So a "character" on disk is: one dictionary + a few thousand index grids +
animation metadata. The index grids are the bulk — RobinTown's grids hold
3.9 million 12-bit indices (7.8 MB as raw u16 words). The original game paid
those 7.8 MB as-is.

## 2. Why zstd stalls: the order-0 wall

The port's first improvement (long before this campaign) was simply to zstd
the serialized bank — level 22, long-range matching. That already beat the
original's raw storage 2.5×: RobinTown's grids went from 7.8 MB to 3.09 MB.
Is that good?

Information theory gives a precise yardstick. If you ignore all structure and
just count how often each of the 4,096 index values occurs, the **order-0
entropy** — the number of bits an ideal coder needs per symbol using only
frequencies — is 6.14 bits/tile for RobinTown, or **3.01 MB**. zstd landed at
3.09 MB: it reached the order-0 wall and stopped.

Why can't LZ do better? An LZ compressor (zstd, and the LZMA in xz) saves
bytes by finding *exact byte repeats* earlier in the stream. The index grids
have enormous structure, but it is **two-dimensional**: the tile above a grid
cell is a far better predictor than any recent byte sequence, because
"above" means "the same image content, one pixel row up." When the stream is
walked row by row, vertically correlated tiles sit `width/4` symbols apart,
interleaved with everything else on the row — almost never forming exact
1-D repeats long enough for LZ to use. Measured:

```
context                bits/tile   ideal size (RobinTown)
none (order-0)             6.14    3.01 MB   <- where zstd sits
left neighbor              4.75    2.32 MB
above neighbor             3.61    1.77 MB
above AND left             1.81    0.88 MB
```

Everything below the order-0 line is invisible to a byte-stream compressor
and is exactly what the custom codec harvests.

## 3. The codec in one sentence

For every tile, predict a probability distribution over the 4,096 possible
indices from the tiles **above and to the left** (already decoded), feed that
distribution to an arithmetic coder, and update the statistics — with the
decoder running the *same* statistics in lockstep so no model ever needs to
be transmitted.

The three pillars:

### 3a. Adaptive contexts (the PPM family)

A **context** is "what we know when predicting this tile" — here, the pair
(above-tile, left-tile). For each context the codec keeps counts of which
symbols followed it before. RobinTown develops ~500,000 distinct contexts;
most are extremely sharp (a context that has produced tile #1712 fifty times
in a row predicts it at >98%).

Sharp contexts have a weakness: the first time a context sees a *new* symbol,
it has no count for it. PPM (Prediction by Partial Matching, the algorithm
family behind RAR and 7z's PPMd) solves this with an **escape chain**: every
context reserves a little probability for "none of my known symbols" and, on
escape, falls back to a simpler context:

```
(above, left)  ->  above  ->  left  ->  no context  ->  uniform over 4096
```

Two refinements matter measurably:

- **Exclusion** (−3%): when `(above,left)` escapes, the fallback level must
  not waste probability on symbols the escape already ruled out. The codec
  subtracts them from the fallback's interval.
- **SEE, secondary escape estimation** (−1..4%): how much probability to
  reserve for escapes is itself learned, in buckets keyed by chain level,
  context size, context maturity (total observations), and how dominant the
  context's top symbol is. Fixed heuristics (PPMC's "one count per distinct
  symbol", PPMD's half-counts) both measured worse than learning it.

Adaptation is deliberately slow (increment 1, halve counts on saturation):
these streams are stationary within a character, and faster adaptation
measured 4-9% worse.

### 3b. Arithmetic (range) coding

Given "the model says this tile has probability p," an arithmetic coder emits
exactly −log₂(p) bits — fractions of a bit per symbol when predictions are
sharp. Ours is the classic LZMA-style range coder: a 32-bit interval is
repeatedly narrowed proportionally to symbol probabilities and renormalized a
byte at a time. Total coder overhead is a handful of bytes per million
symbols; effectively all the codec's intelligence lives in the model.

A note on rANS, the fashionable alternative: rANS emits symbols
last-in-first-out, which is perfect for *static* probability tables but
fights *adaptive* models — the decoder must replay model updates in encoder
order, which is exactly the order rANS refuses to give you. A FIFO range
coder pairs with adaptation naturally; that choice is deliberate.

### 3c. The decoder is the encoder

There is no model in the file. Encoder and decoder run identical code over
identical state: both start empty, both update counts after every symbol,
both halve at the same thresholds, both consult SEE identically. Every
model improvement is automatically "free" in the format — and every model
change is a format change, which is why the bitstream is versioned by the
chunk schema.

Result for RobinTown: **1.99 MB (4.07 bits/tile), −35% vs zstd-22** — and
still measurably above the 1.81 bits/tile two-context bound, which is partly
model overfit (see COMPRESSION.md) and partly remaining headroom.

## 4. Palette families: coding characters against each other

The game ships 9 *families* of palette-swap soldiers (Archer00-05,
Knight01-03, Guard A/B, …) — 48 of 117 characters. Surprisingly, they are
**not** palette swaps at pixel level: the variants were re-rendered from
re-textured 3D models, so lighting and dithering diverge in ~70% of opaque
pixels, and no per-pixel color mapping exists (a best-fit map leaves up to a
third of pixels wrong). This is why generic tricks fail on them — zstd
finds no cross-variant byte repeats at all, and lossless video codecs
(AV1 with screen-content tools, FFV1) lose by 15× when fed the variants as
frames.

But at the **tile level** the mapping is nearly functional: if Knight01 has
tile #a at some position, Knight02 has one of a *small, consistent set* of
tiles there. Conditional entropy: H(Knight02-tile | Knight01-tile) = 0.96
bits — versus ~5.7 bits standalone. So variants are coded with the base
character's aligned tile as the primary context:

```
(base-tile, above)  ->  base-tile  ->  above  ->  order-0  ->  uniform
```

Guard A01 drops from 2.34 MB (zstd) to **459 KB**; the 39 variant characters
compress 3.9× compared to coding them independently.

Positional alignment is what makes this safe: family RHS metadata is
byte-identical apart from frame ids, so frame k of variant B pairs with frame
k of base A, with dimensions verified sprite by sprite (mismatches fall back
to standalone coding — never silently wrong pixels).

Which member should be the base? Not the alphabetically first: a full
pairwise matrix showed the best "hub" saves 4% over name order, so the
converter picks it per family by a sampled conditional-entropy proxy.

## 5. Shipping it: schema v9

The mission-chunked delivery format (schema v8: content-addressed chunks
fetched per mission) gains, per character chunk:

- the sprite rows (id, dims, dictionary index) as before, but VQ payloads
  replaced by **one codec blob per chunk** (grids concatenated in bank-id
  order — measured better than any fancier ordering);
- for variant chunks, the base RHS name and per-sprite base ids, plus a
  **dependency edge** so the base chunk is always fetched and decoded first
  (a variant chunk without its base is a hard error, not a fallback);
- dictionaries stay in the boot manifest, now **frequency-ranked** (most
  used tile = index 0). That permutation is invisible to the decoder and
  saves ~3-5% for anything still zstd-compressed alongside.

Everything is verified end-to-end by decoding every shipped sprite back to
pixels and comparing with the original bank — byte-identical across all
65,058 demo sprites / 146.6 million pixels.

Corpus effect (fullgame characters): 161.2 MB under the old zstd pipeline →
**69.0 MB** (2.34×). Demo web payload `Data/`: 51.2 → 42.4 MB.

## 6. What we tried that *didn't* work — and what each failure teaches

| idea | result | lesson |
|---|---|---|
| Image/video codecs (JXL, WebP, lossless AV1) over any layout, incl. direction-interleaved atlases | 3-17× worse | the similarity between frames/directions/variants is not pixel-exact; motion compensation and 2-D pixel prediction can't grip re-rendered content — only tile-symbol statistics can |
| Byte-plane splitting, numeric index deltas | up to +31% | tile ids are *names*, not quantities; arithmetic on them destroys the exact-match structure both LZ and CM rely on |
| LZMA-style match layer ("same as above?" bit + run contexts) | +3..12% | a strong context model already codes the copy case near its true probability, with more specificity than any flat match model; bolting LZ machinery onto a CM double-charges |
| PAQ-style bitwise context mixing | +1..7% | hashed, binarized models with a logistic mixer could not beat exact-keyed symbol contexts + exclusion on this alphabet size; mixing is not magic, it's a different point in the memory/precision trade |
| Order-3 contexts, faster adaptation, PPMD escapes | ±0 to +9% | more specific ≠ better once escape costs and stationarity are priced in; every model knob must be measured |
| RDO tile re-assignment (lossy) | −0.2% at best | the original VQ dictionaries are clean — no duplicate or near-duplicate tiles to exploit |
| Shared family dictionaries / unified tile ids | +0.1..0.3% | family dictionaries are pixel-disjoint; the PPM is invariant to id permutations, so re-labeling buys nothing |
| Mirrored-direction prediction | ~1 bit/tile weaker than `above` | pre-rendered lighting is directional; bilateral symmetry exists in geometry, not in pixels |
| Pixel-domain CM for the RLE bucket | loses to xz by 7% | animation overlays repeat *whole regions* across frames — genuine LZ territory; dithered literal pixels are near-random locally. Match the tool to the redundancy type |

The meta-lesson: this data has three different kinds of redundancy —
2-D statistical (tile grids → context modeling), cross-stream functional
(families → conditional coding), and long-range exact (RLE animations → LZ).
Each bucket gets the coder that matches its redundancy; no single tool wins
everywhere.

## 7. Where the remaining headroom is

- The standalone gap to the (optimistic) two-context bound is ~2× — richer
  escape modeling and sibling-context coding (using *several* already-decoded
  family members as context, which ships nothing) are the plausible next
  steps.
- Decode speed: the model runs ~1-2 M tiles/s single-threaded; chunks are
  independent, so parallel decode at install is the practical lever
  (deferred, with wasm, by explicit scope decision).
- The animation/patch bucket would take an xz entropy stage (−12% vs zstd)
  rather than any custom modeling.

## 8. Pointers

- `crates/robin_assets/src/sprite_codec.rs` — the codec (range coder, PPM
  chain, SEE, exclusion, cross-variant support), ~1,000 lines, no deps
  beyond a hasher.
- `crates/robin_rs/examples/sprite_compression_probe.rs` — every measurement
  mode used in this campaign (`--stats`, `--entropy*`, `--code*`, `--corpus`,
  `--verify-shipping`, …).
- `docs/COMPRESSION.md` — the dated experiment log with all numbers.
