# Retired parity recording replacement audit

Audit date: 2026-08-17.

`docs/PARITY_RETIRED_TRACES.txt` currently names 192 retired recordings. The
retirement list is a deny-list for historical or incomplete oracles; retirement
does not mean that a save fixture is unusable. A replacement is a new recording
from the current Original source with the old save, RNG seed, input seed, and
duration, published under a new schema-specific corpus after structural checks.

## Inventory

- 177 retired compressed traces are still present and have readable headers.
- All 177 corresponding reference-save fixtures are present.
- 13 retired legacy 10-second trace artifacts are absent, but all 13 named save
  fixtures are present. Their canonical no-input mode and 250-frame duration can
  be reconstructed from the corpus convention; their old headers cannot be
  independently rechecked.
- 43 schema-12 60-second paths have a same-relative-path schema-14 replacement.
  The 30-second Savegame_026 duplicate is also covered by its longer schema-14
  seed recording, for 44 already-superseded retired paths in total.
- All 98 traces eligible for a reliable replacement now have schema-15
  replacements: the three initially requested current-source-inconsistent
  traces plus the 95 direct-recapture candidates described below.

## What can be replaced now

After excluding the 44 already superseded paths, the first three replacements,
and the categories below that require more than a recapture, 95 retired
recordings were eligible for direct schema-15 recapture. They cover:

- stale/current-source-inconsistent simulation captures;
- truncated JSONL or missing-terminator captures;
- schema-12 selected-view/presentation contamination;
- renderer-only visibility-query contamination; and
- older capture trajectories that current Original no longer reproduces.

The resumable batch completed on 2026-08-17: 94 recordings were newly published
and one verified 10-second control was skipped on the final pass because it had
already been published while testing the driver. Together with the first three,
the replacement corpus contains 98 traces:

- 61 under `60s-random-input/schema15-replacements-20260817`;
- 24 under `30s-random-input/schema15-replacements-20260817`; and
- 13 under `10s-no-input-schema15-replacements-20260817`.

Every batch publication has a schema-15 header, the requested frame count,
exactly one terminal RNG suffix, a tested long-window zstd stream, and a
completion marker containing its SHA-256. All 95 marker hashes and all 98 zstd
streams were independently rechecked after the batch. The compressed corpus is
approximately 754 MiB; no raw or partial attempt files remain.

A fresh current-source trajectory may reveal a real new Rust mismatch; that is
a valid replacement and should become a new frontier rather than being retired
again.

## Rust validation status

All 98 schema-15 replacements now match every recorded field through exact EOF
with the Rust runner built using Cargo profile `parity`. This is 100% strict
schema-15 telemetry and simulation parity. The result combines the completed
targeted reruns from the parity-fix campaign; at the user's request, the final
recaptured S004 replay was validated alone rather than redundantly rerunning the
other 97 already-exact replacements.

The fixes restored the recorder/runner contract as well as simulation parity:

- active PassDoor telemetry now compares Original's selected movement element,
  not Rust's separate physical traversal latch;
- AI views read the mutable assigned post mirrored by `GetInitialPosition()`,
  clearing all 36 visibility-only differences;
- six newly exposed actor, sequence, animation, melee, and RNG frontiers were
  fixed against the current Original source; and
- schema 15 now records the beggar cooldown side effect lost when a double-click
  interaction is canonicalized as `make_pc_fast`. The affected S004 replay was
  recaptured as 1,500 frames ending at frame 17156 and validated through exact
  EOF (SHA-256 `bca94e766cfecc4b3c43a68742e6b8fbfa263ecdcbbd979e813e237e5b4c6ed4`).

The resumable validator is `scripts/validate_schema15_replacements.sh`. Its
generated results and per-trace logs live under
`parity-save-replays/schema15-replacement-validation-20260817/` and are ignored
corpus artifacts. The earlier visibility-only classification and each
subsequent cleared frontier were checked through EOF, not inferred from a
first-frame comparison.

## What a schema-15 recapture does not fix yet

Forty-eight paths need a recorder/source-policy change before a new recording
would be a reliable replacement:

- 42 exercise Original undefined or uninitialized state: 36 out-of-range
  `ShootType` captures, two Bonus `old_elevation` captures, and four sprite
  timing/out-of-bounds captures. Harden or explicitly record the Original input
  first; simply recapturing can produce another machine-dependent oracle.
- Five omit input needed to replay the action: one sword-strike gesture/seek
  distance and four beggar click targets. Add those resolved inputs to the
  recorder before recapturing.
- One depends on Original auxiliary sound-restart draws while Rust deliberately
  isolates audio RNG. This is a policy difference, not a stale recording.

The schema-15 wishlist also retains deeper diagnostic additions (straight-move
authorization/move boxes and strike-proposal inputs). The three replacements in
the 2026-08-17 batch are structurally valid current oracles, but those additions
would make future divergences in the same systems substantially easier to
explain.

## Replacement procedure

1. Read the retired trace header for `rng_seed`, `random_input_seed`, simulation
   rate, start state, and duration; resolve the corresponding file below
   `reference-saves/`.
2. Build the current Original schema-15 recorder and record with
   `-PARITYSAVE`, `-PARITYSEED`, `-PARITYRANDOMINPUT`, and `-PARITYFRAMES`.
3. Reject the result unless its header says schema 15, the frame count is exact,
   and the final record is the sole matching `rng_suffix`.
4. Compress with long-window zstd, test the compressed stream, then remove the
   raw JSONL.
5. Run `original_parity_replay` built with Cargo profile `parity` against the
   result. Exact EOF is ideal; a deterministic new mismatch is retained as a
   new parity frontier.
6. Record old-to-new path mapping, seeds, final frame, checksum, and validation
   result beside the replacement corpus.

## 2026-08-18 anti-collision oracle addendum

Two additional schema-12 recordings were proven inconsistent with the current
Original source rather than evidence of a Rust gameplay defect:

- nicouzouf Profile_001 Savegame_032 replay-002 retains its cached deviated
  increment after a recovery corridor with no candidate line. Its existing
  same-relative schema-14 replacement matches Rust through exact EOF.
- linux3 Profile_003 Savegame_042 replay-012 records `increment_map.x` as
  `0xbefcbc1d` at frame 6743. Replaying its exact save, RNG seed 1, and random
  input seed 9012 with both the current Original binary and the pinned
  schema-14 capture binary produces `0xbefcba91` at the same committed
  position, exactly matching Rust. `RHPositionInterface.cpp` also requires
  deviation clearing and increment recomputation after the reachable recovery.

Do not add a Rust compatibility branch for either stale value. Savegame_042
needs a 1,500-frame schema-16 replacement with RNG seed 1 and random-input seed
9012; Savegame_032's schema-14 replacement is already sufficient. Schema 16 is
preferred for future anti-collision investigations because `RHParity.cpp`
records the move box, anti-collision latch, deviation flag, blocked count,
blocked box, and radius. Those fields distinguish a genuine recovery decision
from an old recorder trajectory without intrusive gameplay instrumentation.

For the targeted Savegame_042 recapture, stage only
`reference-saves/Savegame_linux3/Profile_003/Savegame_042` in the capture input
tree, then run `original-code/scripts/capture_parity_save_replays.sh` with
`PARITY_TRACE_SCHEMA=16`, `PARITY_SEED=1`, `PARITY_INPUT_SEED_BASE=9012`,
`PARITY_RANDOM_REPLAYS=1`, and `PARITY_FRAMES=1500`, publishing to a dated
schema-16 replacement corpus. Apply the normal header/frame/suffix, zstd, and
parity-profile exact-EOF gates before accepting it.
