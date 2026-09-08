# Replay mission payload audit, 2026-09-08

The retained Leicester corpus has no byte-identical duplicate mission parts and
no shipped RHS profiles outside the converter's declared global closure. This
audit does **not** establish a safe payload reduction, so it changes neither the
converter nor the runtime. In particular, it does not restore separate opacity
masks, drop frames, or substitute a first-image measurement for usable playback.

Base source: `2c504019a`. Inputs are the ordinary, non-partitioned
`/tmp/robin-startup-more/corpus-trimmed/Data` and the retained
`mission/{parts,roots}.jsonl` inventories. File sizes and SHA-256 hashes were read
again from the actual compressed corpus. The inventories describe the previous
decoded data; this audit checks their file sizes, not a fresh independent decode.
The generated report hashes both inventory files, the plan, boot and every
selected payload so that a future fresh decode can be compared to these inputs.

## Actual compressed bytes

| Authored `Dem_Lei_MP` closure | Bytes |
| --- | ---: |
| RHS sprites and scripts | 13,249,772 |
| Terrain | 1,786,597 |
| Mission | 380,516 |
| Audio metadata parts | 6,599 |
| **66 unique parts** | **15,423,484** |

Boot, external audio range requests and character dependencies added from the
replay campaign are excluded from this table. This is not the complete browser
network total. Explicitly adding character profiles 2, 3 and 6 yields 69 parts /
20,128,960 bytes: Little John adds 1,962,073 bytes, Friar Tuck 1,376,509 bytes,
and Marian 1,366,894 bytes. These are selectable characters, not proven unused
assets in a particular replay. Runtime already selects the team and eligible
uninstanced non-VIP reinforcements in `shipping_mission::required_dependencies`.
Their presence in the publication directory does not mean every replay fetches
them.

Across all 69 inventoried parts, all 110 shipped animation profiles belong to
the global conversion plan. `sprite_pipeline::transform_rhs` already filters
profiles and collects the unique frame IDs reached by all of their scripts.
The largest character payloads each contain a single required profile; deleting
other profiles from those payloads cannot save bytes. This does not prove the
global plan minimal for each individual mission. It rules out the proposed
simple removal of profiles that are absent from the plan.

## Rejected removals and further work

- **Whole-part deduplication:** all 66 authored parts have different SHA-256
  hashes; redundant compressed bytes from identical parts are zero. This does
  not claim there are no duplicate pixels inside otherwise different parts.
- **Accessory / bonus / relic closure:** all 30 such authored parts total only
  **140,938 bytes**, about **70 ms** of ideal transfer at 16 Mbit/s. Even deleting
  all of them would be that small, and would break the engine's eager object
  master creation. Most are also actually required. The converter explicitly
  includes the closure for this reason in `mission_planning.rs`.
- **Unused animation frames:** absence in the first 300 recorded frames is not
  proof of absence over the replay or simulation's opacity/hash queries. No
  frame removal is justified by the retained short replay.
- **Dictionary duplicate removal:** the earlier exact dictionary probe in
  `docs/COMPRESSION.md`, “RDO tile assignment: closed,” found zero saved bytes
  for its RobinTown / Knight01 / Guard A00 sample. That is historical evidence,
  not a new whole-corpus pixel audit. Near-identical replacement changes pixels
  and is outside this exact-dependency task.

TODO: if pursuing a format change, measure shared sprite-bank IDs and exact
grid aliases after decoding all candidate chunks; preserve every bank ID and
opacity/hash result, include any new cross-part dependencies in the byte total,
and compare full replay playback. The current inventory does not expose those
grids, so claiming a savings estimate for that change would be unsupported.
The present audit gives no evidence that a dependency-trimming change would
beat the parallel WASM transport and scheduling opportunities.

## Reproduction and validation

The new standard-library script produces the table and a reproducible hash
inventory without rebuilding or re-encoding the corpus:

```sh
python3 scripts/audit_replay_payload.py \
  /tmp/robin-startup-more/corpus-trimmed/Data \
  /tmp/robin-startup-more/mission/parts.jsonl \
  /tmp/robin-startup-more/mission/roots.jsonl Dem_Lei_MP
```

Add `--character 2 --character 3 --character 6` for the explicitly augmented RHS
closure. This is not automatic replay campaign inspection and excludes added
character audio dependencies. To generate fresh inventory streams, build
`cargo build --locked -j2 -p robin_assets --example mission_sprite_audit`, then
run `target/debug/examples/mission_sprite_audit DATA` with stdout and stderr
saved separately. Keep compilation separate from the JSONL capture.

Validation ran the real 66-part audit, asserted the augmented 69-part total,
checked the zero excess-profile result and deliberately changed an inventory
size to verify rejection. `git diff --check` passed. No Rust or game behavior
changed, so there is no new replay timing claim or native/WASM build requirement.
