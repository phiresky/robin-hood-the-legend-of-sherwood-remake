# Typed runtime sentinel boundaries

## Decision summary

- **Owner decision requested:** Accept the revised implementation, then
  reconcile it onto rolling `main` before merging.
- **Implementation status:** Complete at `b938d6734` on
  `codex/accepted-45-typed-runtime-sentinels`.
- **Verification confidence:** High for typed state, strict serialization,
  Original-save adoption, replay/rollback/network round trips, and engine
  behavior covered by automated tests. Medium for an authentic shipping-data
  gameplay smoke test, which has not been performed on this final branch.
- **Direct-merge status:** **Not safe as-is.** The branch and rolling `main`
  independently consumed the same save/replay/network version numbers for
  different layouts. The exact reconciliation plan is below.

This is an internal correctness and maintainability change. It does not add a
visual or gameplay option, so the project's settings-toggle requirement does
not apply. Its observable effect is to reject ambiguous/corrupt current-native
state instead of interpreting magic maximum integers or entity slot zero as
absence.

## Accepted scope implemented

The branch finishes the `docs/NEW_FEATURES.md` Code Quality item that moves
legacy sentinel values to explicit translation boundaries. Runtime state now
uses nominal handle types and `Option` when absence is meaningful:

- `Layer` and `PathfinderIndex` use `NonMaxU16`; `0xffff` cannot become a live
  value and nullable position state uses `Option`.
- `TitbitId`, `DoorIndex`, `SightObstacleIndex`, and `SectorIndex` use
  `NonMaxU32`; `0xffff_ffff` cannot become a live identifier in these domains.
- AI references that were previously overloaded raw integers use
  `Option<AiEntityHandle>`. `AiEntityHandle(0)` is intentionally valid because
  arena slot zero is a real entity, commonly Robin; only `None` means no
  entity. Human-readable current state encodes a present handle as
  `{"entity": N}` so bare `0` can no longer collapse a live actor into null.
- `SectorHandle` carries the script-facing signed `SectorNumber` and optional
  exact `SectorIndex` arena identity. This represents the Original's real
  out-of-map sector number `-1` while retaining pointer-equivalent identity for
  gameplay that compares `RHSector*` values rather than public numbers.
- Position changes install sector number and arena identity atomically.
  Number-only writes clear unproven provenance instead of leaving a stale arena
  pointer attached.
- Nullable AI targets, stimulus owners, friend/body/object references,
  antagonist links, checkpoint/synchronization links, combat neighbours,
  phalanx/cross-NPC actions, door combat adversaries, and related retained
  references are propagated as typed optional handles through AI, combat,
  movement, scripts, and snapshot lookup boundaries.
- `NoiseOrigin` structurally represents optional sector and layer information.
  A one-shot effect with no world layer no longer creates a fake `0xffff`
  layer-bearing position.
- Position, door, obstacle, pathfinder, titbit, projectile, purse/coin, and
  renderer/UI consumers were updated to use the typed forms rather than
  recreating sentinel checks downstream.
- JSON and native bitcode state require the new fields explicitly. A missing
  nullable field is not silently treated as `None`; outer schema gates reject
  old native state, and malformed current state fails during decoding.

The final validation checkpoint also corrected fixture regressions uncovered
by the full engine suite: exact hiking-path and sector topology identity,
sprite/pathfinder rebinding, purse water-impact ordering, and a realistic
minimal Sherwood replay motion-sector graph. The brawl missing-target behavior
now follows the Original's assert-plus-release fallback: it emits an error and
returns the soldier to duty instead of inventing a friend target or changing
behavior only in debug builds.

## Persistence and migration semantics

There are deliberately three separate boundaries:

1. **Current Rust saves, replays, rollback snapshots, and network snapshots**
   use only the typed layout. Their header/protocol version must match exactly.
   There is no adapter for older Rust layouts and no attempt to infer omitted
   fields or reinterpret legacy raw sentinels.
2. **Original C++ v48 saves** remain importable through `legacy_save`. The
   importer decodes the Original pointer and integer conventions first, then
   resolves them to typed runtime handles. Incomplete Original history remains
   subject to the project's separate campaign-import policy; this branch does
   not add native-save compatibility.
3. **Authored/binary level data** retains its source-format integers, including
   `0xffff` where the format defines it. Translation happens while adopting
   assets into runtime objects. The raw asset structures are not falsely
   rewritten as if the on-disk format were typed Rust state.

This matches the owner's storage decision: old Rust campaigns/saves/replays do
not require compatibility, while legacy C++ saves remain explicitly
importable. Failing closed is necessary here because the old AI representation
cannot distinguish a live arena slot-zero reference from its historical Rust
null convention, and old sector JSON may omit pointer-equivalent provenance.

## Implementation map

- `crates/robin_engine/src/ai/types.rs`, `ai/model.rs`, and
  `ai/controller.rs`
  - define current AI handles and tagged nullable serialization;
  - replace ambiguous target/owner/reference fields with typed optional
    handles.
- `crates/robin_engine/src/ai_enemy/`, `ai_friendly.rs`, `ai_vision.rs`, and
  `engine/ai/`
  - carry those identities through snapshots, event dispatch, detection,
    combat relationships, and cross-NPC actions without zero fallbacks.
- `crates/robin_engine/src/position_interface.rs`, `fast_find_grid.rs`,
  `sight_obstacle.rs`, `gate.rs`, and `sector.rs`
  - define typed spatial indices, signed public sector numbers, exact arena
    provenance, and explicit nullable position state.
- `crates/robin_engine/src/engine/{movement,door_pass,commands,combat}.rs` and
  their submodules
  - preserve pointer-equivalent sector/door/obstacle identity across routing,
    elevation, projectiles, interactions, and exact command outcomes.
- `crates/robin_engine/src/titbit.rs`, `engine/titbit_sync.rs`,
  `engine/purse.rs`, and `crates/robin_rs/src/{titbit_renderer,game_render}.rs`
  - use typed titbit IDs and optional layers end to end.
- `crates/robin_engine/src/legacy_save/`
  - translates Original v48 pointer and maximum-integer encodings at the
    adoption boundary.
- `crates/robin_engine/src/engine/rollback_safe.rs`, `replay.rs`,
  `multiplayer.rs`, and `crates/robin_rs/src/save_file.rs`
  - enforce the typed layout across native persistence, deterministic replay,
    rollback, and multiplayer initial snapshots.

The complete change from merge base `6509ddc1453a0498983942a1d65fe9bdd938a1f9`
through `b938d6734` touches 117 files with 5,490 insertions and 3,105
deletions. Most of that surface is mechanical type propagation through the
engine; the semantic decisions are the boundary rules listed above.

## Original-code evidence

The implementation was checked against `./original-code`, not inferred from
the pre-existing Rust sentinels:

- `original-code/RHartificialintelligence.cpp:435-450` initializes primary
  target, friend-in-trouble, detected body, antagonist, last stimulus actor,
  door, and other AI pointer fields to `NULL`.
- `original-code/RHartificialintelligence.cpp:4222-4229` serializes those
  retained references via `SerializePointerToElement`.
- `original-code/RHartificialintelligence.cpp:4992-5040` proves that the v48
  stream's null pointer marker is decimal `54321`, not entity index zero. A
  non-null pointer is serialized as its element-table index, and an unresolved
  deleted pointer is deliberately written as the same null marker.
- `original-code/RHpositioninterface.cpp:50-75` initializes sector, obstacle,
  plane, and door pointers to null while initializing both pathfinder indices
  to `(UWORD)-1` (`0xffff`).
- `original-code/RHpositioninterface.h:75-104` shows that layers are scalar
  `UWORD` values while sectors, obstacles, planes, and doors are distinct
  pointers. Conflating all of those namespaces into one numeric sentinel loses
  information.
- The Original repeatedly compares sectors by pointer identity, for example
  `original-code/RHartificialintelligence.cpp:135` includes `pSector` in
  `RHposition` equality and `RHartificialmalignity.cpp:14562-14649` compares
  friends' sector pointers. This is why a public sector number alone is not an
  exact runtime identity.
- `original-code/RHtitbit.cpp:360-384` returns `0xffffffff` when no titbit is
  created and reserves `0xffffffff` to mean “no forced ID”; the live ID domain
  therefore excludes that maximum value. `RHtitbit.cpp:516` and
  `RHelementactorpc.cpp:181` use the same value for absent stored quick-action
  titbits.
- `original-code/RHartificialmalignity.cpp:1527-1532` handles a null
  `mpFriendInTrouble` at `EVENT_REACHPOINT` by asserting and calling
  `ReturnToDuty`. The Rust revision logs the invariant failure and preserves
  the release behavior instead of manufacturing a target.

These references also explain why the conversion is intentionally selective.
Some maximum values elsewhere are real protocol values (for example unlimited
ammunition or missing animation conversions), not nullable runtime handles.
They remain unchanged until their semantics are independently proven.

## Verification evidence

Validation was run on the exact final code commit `b938d6734`:

- `cargo test -p robin_engine` passed:
  - 4,067 unit tests;
  - 8 engine-facade integration tests;
  - 3 geometry guardrail tests;
  - 3 method-footgun tests;
  - 1 `r007` probe test;
  - one documentation example remained intentionally ignored.
- `cargo test -p robin_rs save_file` passed all 23 selected save-file tests.
- `cargo build --bin robin` passed for the required native binary.
- `cargo fmt --all` and `git diff --check` passed.

Focused coverage within those suites includes:

- current JSON rejecting legacy bare AI integers while tagged slot zero and
  null round-trip distinctly;
- current position JSON rejecting missing provenance fields and raw sentinel
  values;
- JSON and bitcode sector handles preserving signed out-of-map values and
  exact arena identity;
- atomic sector topology updates and number-only writes clearing provenance;
- save capture/apply preserving a live AI reference to arena slot zero;
- replay state hashes distinguishing live slot zero from absence;
- rollback and multiplayer initial snapshot bitcode preserving typed AI and
  spatial state;
- legacy v48 AI pointer adoption resolving optional references explicitly;
- brawl absent-target behavior and ordering-sensitive purse/titbit effects;
- exact hiking/pathfinder and duplicated-public-sector fixture behavior.

The build reports two pre-existing non-fatal warnings: an unused
`clear_layer_goal` helper and an `unused_mut` in the shipping-datadir asset
code. The local `sccache` server also stopped repeatedly, so compilation fell
back to local work; this did not affect correctness.

## Current-main schema collision and reconciliation plan

Feature 45 forked at `6509ddc`, where the native boundaries were save v57,
replay v17, and network protocol v24. This branch advances them to **58/18/25**
for the typed runtime layout.

Rolling `main` independently used **58/18/25** for per-mission achievement
state and then advanced further for other accepted deterministic changes. At
the time of this review document, `main` is
`25c30b354cea6422b1c4361f4b1e8a4ef3b66db4` and uses:

| Boundary | Feature 45 branch | Current `main` | Required integrated value now |
| --- | ---: | ---: | ---: |
| Native save | 58 | 60 | 61 |
| Replay | 18 | 20 | 21 |
| Multiplayer protocol | 25 | 27 | 28 |

Therefore a direct merge that merely accepts `main`'s constants would label a
new typed layout with an already-shipped schema number. A direct merge that
takes Feature 45's constants would regress versions and collide even more
obviously. Neither is acceptable.

After owner acceptance, integration must:

1. Merge/rebase the latest rolling `main` into this branch and resolve the
   overlapping AI, rollback snapshot, save, replay, multiplayer, and renderer
   edits semantically.
2. Set each boundary to `max(latest-main, feature-branch) + 1`. With the exact
   `main` captured above that is **save 61, replay 21, protocol 28**. If `main`
   advances before reconciliation, use the then-current maximum plus one
   instead of the numbers in this table.
3. Update version history comments and exact-version tests to state that the
   new version combines all rolling-main fields with typed runtime sentinel
   state.
4. Keep the exact-match fail-closed behavior. Do not add a v58/18/25 adapter:
   those numbers describe two incompatible development layouts and cannot be
   disambiguated reliably from the version alone.
5. Rerun the full `robin_engine` suite, focused `robin_rs` save tests, required
   native build, formatting, and diff checks on the reconciled commit. Add
   focused tests for any rolling-main snapshot fields touched during conflict
   resolution.
6. Run one authentic C++ v48 import smoke test and one shipping-data native
   launch before final merge; neither changes the no-old-Rust-migration policy.

No reconciliation has been performed in this review branch, by design, so the
owner can judge the Feature 45 delta independently before the integration
commit is produced.

## Risks and known limitations

- This is a broad internal refactor across AI and spatial hot paths. Automated
  coverage is extensive, but the final branch has not had a manual
  shipping-data mission playthrough.
- Exact sector arena provenance cannot be guessed safely when multiple live
  sectors share a public number. Current state must carry it; legacy adoption
  resolves it from loaded topology only where the Original data proves the
  identity. Ambiguity fails instead of choosing a plausible sector.
- Required accessors intentionally panic if a legacy no-layer state escapes
  the small set of optional-layer boundaries. This exposes invalid runtime
  state early rather than returning fake layer zero.
- Several broad `u32` aliases remain for required, historically non-null
  element parameters. Feature 45 converts fields where absence was overloaded;
  it does not claim to complete a global entity-system newtype migration.
- Maximum integer values used as genuine Original protocol remain unchanged,
  including selected animation and ammunition states. A global search-and-
  replace would be incorrect.
- Integration conflicts are expected because rolling accepted features changed
  `rollback_safe`, save/replay/network schemas, and some renderer/gameplay
  surfaces after the branch fork. Conflict resolution must preserve both sides'
  fields and tests, not select one entire file version.
- Old native Rust saves, replays, and peers intentionally stop working at this
  boundary. Only explicit Original C++ import remains supported.

## Review recommendation

**ACCEPT THE REVISION, THEN RECONCILE AND MERGE.**

The implementation removes the ambiguous runtime sentinels identified in the
feature list, preserves live entity slot zero, retains Original pointer-level
sector semantics, keeps legacy encodings at explicit import/asset boundaries,
and passes the full engine plus required save/build validation. It should not
be merged at `b938d6734` unchanged only because rolling `main` has since
consumed the same schema numbers for unrelated layouts. The required
post-acceptance reconciliation is mechanical in policy but must be tested as a
new integrated schema.
