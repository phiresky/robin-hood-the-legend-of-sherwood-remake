# Shared brief — wave C (the last 60 divergences)

## Worktree base — check FIRST, in BOTH directions
`git rev-list --count HEAD..main` AND `git rev-list --count main..HEAD`. Seven agents in a row were
created ~250 commits BEHIND main while the usual "am I ahead" check read zero, which made unrelated
traces look like regressions from their own fix. Merge current main before measuring anything.

## Authoritative data
Runner `26f395142`, release profile, sha256 `a624082243bb7c2b…`. **60 traces remain.**
`/home/phire/robinhood/output/parity-audits/batch22-fail97-26f395142/`
- `bundles/<your bundle>.txt` — your traces, grouped
- `remaining-60.snapshot` — the full remaining set
- `logs/<trace path with / -> __>.log` — per-trace failure logs
- `classification.json`, `groups.txt`
Read-only data lives in the main repo (traces, datadirs, original-code) — use absolute paths.

## Triage before fixing
Every cluster label taken at face value in this campaign has decomposed into unrelated bugs: a 17-trace
direction_goal group was four, a 15-trace RNG bucket was nine, a 12-trace animation/motion pair was eight,
a 20-trace melee family was five, a 16-trace idle-RNG bundle was seven. **Your bundle is a THEME, not a
claim of shared cause.** Extract each member's real first-divergence signature (entity kind, posture,
command, animation, action state, AI substate) from the trace's `elements` array at
`frame_before = N-1` (the runner reports `frame_after`), group by that, report the grouping, then fix the
largest genuine sub-group you can prove. One solid sub-group plus clean per-member attribution for the
rest is a good outcome; a forced single story is not. **24 of the 37 remaining groups are singletons** —
expect one cause per trace.

## Facts — do not re-derive
- C++ `RHsubstate` and Rust `ai::model::Substate` align **1:1**, 256 entries from 0. The old "+1" note was
  WRONG. Anchors: 9 `DefaultGotoRoute`, 155 `AttackingRunningToEnemy`, 160 `AttackingSwordfight`,
  162 `AttackingSwordfightParade`, 250 `RunToAvengerOnRoof`.
- `actor.animation` IS `mpOrder->action` (`original-code/RHelementactor.h:209`) — an animation divergence
  is an ORDER-ADVANCE TIMING divergence; 283 = `RHNONANIMATION_END` = no order installed.
- The sector classifier is faithful (`vector_to_sector_0_to_15_with_aspect` vs
  `SBGeoVector2D::GetSector0to15`), verified by five investigations. Direction bugs are always *which
  vector*, *which origin*, or *whether/when the write happens*.
- **Read float width from DISASSEMBLY, not the C++ source.** `cos(float)` binds to the float overload in
  C++ (no double promotion). `GetRotated`/`operator*`/`Det` are all single-precision (`mulss`); only
  `Angle` goes double, matching its explicit cast. A wrong f64 assumption already caused one bad merge.
- **`Position(mpMe)` SNAPS to the gate endpoint during a door pass.** C++ sites using `GetPosition()` /
  `GetPositionGround()` (e.g. `SquareDistance`, `MaxNormDistance`, `RHartificialintelligence.cpp:6919-6922`)
  must read the RAW element position — `FighterSnapshot::raw_position` / `ctx.self_body_position_world`.
  This bug class has already produced three separate fixes; more sites are owed an audit.
- `MaxNormDistance` (`RHartificialintelligence.cpp:6950-6953`) is a **3D Chebyshev norm with Y stretched**
  (`sb3dstuff.cpp:70-88`), not a flat 2D distance.
- schema-12 traces are FULLY TRUSTWORTHY oracles (audio draws are domain-tagged; the runner consumes only
  `TraceRngDomain::Simulation`). An earlier claim otherwise was disproven.
- RNG-site labels are frequently artifacts — the classifier keys on the first RNG site of the frame.
- `INVERSE_SWORDFIGHT_ASPECT_RATIO` is 1 in the shipping build; the half-circle/lateral `mY *=` asymmetry
  is a **no-op**, don't chase it.
- Before reporting any trace as failing or regressed, check `docs/PARITY_RETIRED_TRACES.txt` (187 entries).
  Two agents have already wasted effort on retired traces.

## Tooling
`original-code/build/native-full/robin-schema14-capture` reproduces **schema14** traces bit-exactly and
gdb against it works. Debug build prints `[DBG f=N …]` for `SendCondolationCard`, `FaceTo(dir=D) curDir=C`,
`Think`, `MOVEPOST`, `PFCHANGE` — the condolation stream shows exactly which command terminated or was
interrupted each frame (`state 6` = `RHSEQ_INTERRUPTED`).
```
env ROBINHOOD_DATA_DIR=/home/phire/robinhood/datadirs/fullgame_linux SDL_AUDIODRIVER=dummy \
  SDL_VIDEODRIVER=dummy \
  /home/phire/robinhood/original-code/build/native-full/robin-schema14-capture \
  -PARITYSAVE <reference-save> -PARITYTRACE <out>.jsonl -PARITYSEED 1 -PARITYFRAMES 1500 \
  -PARITYRANDOMINPUT <trace header random_input_seed>
```
Verify the save's sha256 against the trace header's `initial_save` first.
**A REBUILT binary is now available and is the one to prefer:**
`original-code/build/rebuild-20260816/robin` — built 2026-08-16 from current sources and **verified
equivalent** to the pinned capture binary (300 frames of Savegame_linux3/Profile_001/Savegame_008 seed 14:
every simulation field and RNG draw value identical; only `draws.callsite_offsets` differ, as raw code
addresses must). It CONTAINS the hooks the pinned binary lacks — `PARITY_DEBUG_REACHABILITY` (8 sites),
`PARITY_DEBUG_ORIGINAL_RECONSIDER_SWORDFIGHT` (3), `PARITY_DEBUG_ORIGINAL_PROJECTILE` (7). Use it for any
Original-side reachability / route / reconsider / projectile question.
**Run it WITHOUT `LD_LIBRARY_PATH`** — it links against the system i386 libraries. (The repo's bundled
`lib32/` has been deleted; it was proven to make no difference to output.)
**Caveats that still apply:** neither binary reproduces schema-12 traces (no `Think` in the `[DBG]` stream
on those frames) — schema-14 only; both ship a symtab but **no DWARF**, so gdb needs
`break *(&'<mangled>' + offset)` off hand-read disassembly
(`RHArtificialIntelligence::mpUniversalFrameCounter` is the frame counter); and `set $sp = …` in a gdb
script silently clobbers the stack pointer and segfaults the inferior.
Callsite offsets are **not** comparable across capture generations and do not resolve against
`build/native-full/robin`. Use the runner's learned `PARITY_DEBUG_RNG_SITE_MAP`, which names the draw Rust
SKIPPED even when Rust never reached that site.
Other diagnostics in main: `PARITY_DEBUG_PATH_BARRIER`, `PARITY_DEBUG_DETECTABLE_DIFF`,
`PARITY_DEBUG_BATTLE_DECISION`, `PARITY_DEBUG_FORECAST_IA`, `PARITY_DEBUG_PARADE_TIMER`,
`PARITY_DEBUG_BAD_EXPERIENCE`, `PARITY_DEBUG_SEEK_AREA_OWNER_POSITION`. See
`crates/robin_rs/examples/original_parity_replay.rs`.
**Known blind spot:** `position_goal_map` is compared with a 0.011 ABSOLUTE tolerance
(`original_parity_replay.rs:6131`) while other floats use 1e-5 relative — 1-ULP goal errors are invisible
and only surface via `movement_map`. If your member is a small float divergence, check the goal manually.

## Build / run
`cargo build --example original_parity_replay` (no timeout; never pipe cargo output through filters).
Also `cargo build --release --example original_parity_replay` for deep members and use the release runner
there. **Do NOT end your turn to wait** for a build or replay — use blocking foreground waits and keep
working until you have a committed, validated fix or a documented dead end.
`/tmp` is a shared 32 GB tmpfs that has filled once; write large output to `/var/tmp` (~1 MB/frame) and
delete as you go. A capture fleet is running at idle priority — expect some CPU contention.

## Rules
Faithful ports of original-code C++ only, citing file:line. No per-trace hacks. No invented guards or
defensive fallbacks — fail loud rather than returning fake data, and never let a diagnostic print
placeholder values as if they were measured. (Three "invented invariants" have already been removed where
Rust enforced at runtime what the Original only `assert`s in debug builds — watch for more.) Match C++
statement order. Never `git stash`. No clippy. `cargo test -p robin_engine --lib` before committing
(~3344 pass). `cargo fmt`. Every commit message must accurately describe its WHOLE diff.

## Validation contract
A trace passes only if the runner prints exactly `parity trace matched every recorded frame`:
```
ROBINHOOD_DATA_DIR=/home/phire/robinhood/datadirs/fullgame_linux <runner> --no-auto-dump <abs trace>
```
Prove causality: after a fix, revert ONLY your change on the same base, rebuild, and confirm the old
frontier returns exactly. Run 2 already-passing controls (any 15-no-input trace NOT in
`remaining-60.snapshot`). **Cap controls at ~10 targeted traces** — identify which passing traces actually
exercise your changed path (scan the corpus for the relevant command/animation) rather than sampling
hundreds; a targeted proof beats a large random sample and the full sweep re-verifies everything later.

## Final report
Triage table for every member (member -> real signature -> sub-group), branch and commit hashes, root
cause with original-code file:line, per-trace validation outcomes (exact EOF or new frontier frame), and
which members belong to a different family.
