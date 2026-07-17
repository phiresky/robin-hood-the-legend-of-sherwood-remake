# Gameplay Parity Audit

Audited on 2026-07-16 at Rust revision `15e5ffd6f`. This audit compares the Rust gameplay runtime with the checked-in original
sources. It covers tick ordering, AI, sequences, scripting/Lua, RNG, snapshots,
and mission ownership. It is evidence, not a claim that a subsystem is fully
equivalent merely because a matching symbol exists.

## Evidence rules

- **Original** means a path and symbol in `original-code/`. Lua/Spellforge is a
  post-original extension and cannot be labelled Original provenance.
- **Confirmed** means the relevant order or state transition can be traced on
  both sides.
- **Adapted** means Rust changes the mechanism while preserving a stated
  observable contract.
- **Open risk** means no trace test proves the observable order yet.
- **Mismatch** means the sources show different observable behavior.

Line numbers describe the audited revision. Symbols are included because they
remain useful if nearby comments move.

## Executive findings

| Priority | Finding | Evidence | Required disposition |
| --- | --- | --- | --- |
| P0 | Rollback snapshots omit `HostDisplayState`, although the tick reads it to decide whether to return during zoom. Replay starts from default display state. | Original serializes `mbackgroundTransform` in `RHEngine::Serialize`, `original-code/RHengine.cpp:2463-2531`. Rust reads it in `crates/robin_engine/src/engine/tick.rs:558-564`; `SimSnapshot` stores only frame and `Engine`, then creates scratch display state in `crates/robin_rs/src/sim_timeline.rs:22-41,105-120`. | Put every tick input in the deterministic snapshot or make it a derived immutable input. Add a non-default zoom replay test. |
| P0 | Lua is unsnapshotted and incompletely dispatched. Comments promise Timer/victory/finalize, but the only live `run_event` calls are Initialize/PostInitialize. | `crates/robin_rs/src/lua_session.rs:16-36`; `crates/robin_rs/src/game_session/mod.rs:898-906`; `Host::lua_session` at `crates/robin_rs/src/host.rs:507-512`. | Reject Spellforge missions in replay/rollback/multiplayer until a tested state policy exists; wire the complete event surface separately. |
| P0 | Lua Initialize can use `math.random` outside an installed sim-RNG scope. Lua startup/runtime errors also become absence or zero. | RNG shim: `crates/robin_lua/src/state.rs:241-299`; missing scope panics: `crates/robin_engine/src/sim_rng.rs:77-84`; startup calls: `game_session/mod.rs:891-906`; fallback paths: `:783-790` and `lua_session.rs:188-196`. | Pass an explicit Engine RNG context and propagate required-script failures. Missing optional callbacks may remain no-ops; interpreter failures must not become zero. |
| P1 | Original WAIT-priority sequence elements call `Go()` during launch; Rust queues them for a later sequence-manager hourglass. | `original-code/RHsequence.cpp:235-289`, especially `:280-286`; `crates/robin_engine/src/sequence.rs:2195-2211`. | Fix with a launch-time ordered-trace test. |
| P1 | Original updates actors in storage order and runs each NPC's AI before the sequence-manager hourglass. Rust uses global subsystem passes, AI snapshots, and deferred drains. | `original-code/RHengine.cpp:3715-3727`; `original-code/RHelementactornpc.cpp:3495-3659`; `crates/robin_engine/src/engine/tick.rs:1088-1136,5360-5650`; `engine/ai/snapshots.rs:13-19`. | Prove same-frame visibility and re-entrant stimulus order with traces. |
| P1 | Mission exits use `take_campaign().unwrap_or_default()`. A missing required campaign can silently become a fresh campaign. | Original requires `RHCampaign::GetCampaign()`, e.g. `RHEngine::QuitMission`, `original-code/RHengine.cpp:16310-16316`. Rust stores `Option<Campaign>` (`engine/mod.rs:422-425`), swaps it into `GameHost` (`engine/types.rs:1572-1592`), and defaults it in several session/mouse exit paths. | Introduce one mission-runtime owner and checked transfer/lease APIs. |
| P1 | Restore repairs malformed fast-grid activity arrays by marking every entry active. | `crates/robin_engine/src/engine/rollback_safe.rs:686-701`. | Reject invalid state or rebuild from a documented authoritative source; add corrupt-length tests. |
| P2 | Replay seed preload failure falls back to multiplayer seed or zero. | `crates/robin_rs/src/game_session/setup.rs:1224-1244`. | Make requested-replay header failure fatal before Engine construction. |
| P2 | Rust RNG is deterministic and snapshot-safe but deliberately is not the original C RNG stream. | `original-code/launcher.cpp:761-766`, `RHScript.cpp:6492-6505`, `RHartificialintelligence.cpp:3833-3863`; Rust `sim_rng.rs` and `EngineInner::rng`. | Define parity at ranges/call sites, not bit-identical rolls. |

## Tick ownership and order

`RHGame::GameLoop` calls the engine only when the console is hidden, the
start/quit window is not transitioning, dummy pause is false, and the operation
is neither level-next nor level-load (`original-code/RHgame.cpp:1801-1809`). It
then simulates widgets, refreshes, advances sound, and performs one-shot
PostInitialize (`:1812-1843`).

`RHEngine::PerformHourglass` has this observable order
(`original-code/RHengine.cpp:3446-3777`):

1. Guard-widget state, mission exits, and cheats (`:3485-3580`).
2. Script Hourglass each 25 frames and victory each three seconds or forced
   check (`:3582-3621`).
3. Increment the universal frame, then return early for zoom/engine lock
   (`:3625-3632`).
4. Default loss, reinforcement, sequence cleanup, paths, collision
   (`:3634-3703`).
5. Iterate `marrayElements` in storage order; call Hourglass and remove dead
   elements inline (`:3715-3724`).
6. Run `RHSequenceManager::Hourglass` (`:3726-3727`).
7. Titbits, selection validity, anonymous timers (`:3737-3774`).

Rust's outer gate is recognizable in `Game::should_run_hourglass` and
`run_engine_tick` (`crates/robin_rs/src/game.rs:497-541,647-659`). RNG surrounds
`perform_hourglass_inner` (`engine/tick.rs:117-230`). The inner tick preserves
script-before-counter-before-lock (`:461-564`) and the default-loss position
(`:566-627`).

The rest is **open risk**. Rust expands the original element loop into global
passes: sequence dispatch begins around `tick.rs:1136`, while movement, AI,
combat, abilities, titbits, selection, timers, condolations, and PostInitialize
continue through `:5650`. This is a valid Rust strategy, but may change which
mutations later entities observe in the same frame.

Rust runs PostInitialize inside the deterministic tick tail
(`tick.rs:5634-5640`); Original calls it after render and sound
(`RHgame.cpp:1830-1843`). Treat that as an adaptation requiring a first-frame
trace, not confirmed parity.

`HostDisplayState` is a correctness boundary: zoom flags alter whether the sim
runs, so they are sim input even if the pixels are presentation. Default display
during replay is not equivalent to live zoom state.

## AI

Original NPC AI is actor-owned:

- `RHElementActorNPC::Hourglass` refreshes patrol/human state, view, detection,
  ambush/deafness, busy/ladder state, shifts deadlines while locked, executes
  staggered 16th-frame work, and drains stimuli FIFO
  (`original-code/RHelementactornpc.cpp:3495-3659`).
- `RefreshDetection` uses `universal frame + creation order`, mutates detectable
  lists, then calls deferred `Think` stimuli FIFO before returning (`:1371-1675`).
- `RHArtificialIntelligence::StartThink` filters and queues events under locks
  (`original-code/RHartificialintelligence.cpp:914-1270`). `EndThink` can recurse
  synchronously with couldn't-reach, reach-point, or done (`:1468-1519`).

Rust `tick_enemy_ai` builds snapshots, executes detection phases, and drains
pending work (`crates/robin_engine/src/engine/ai/mod.rs:3814-3915`). The snapshots
are explicitly read-only (`engine/ai/snapshots.rs:13-19`). Same-tick self-stimuli
are deliberately drained at `engine/tick.rs:5612-5632`.

Status: **adapted, high-risk**. Required traces include NPC A changing state
before NPC B's storage-order turn; detection launching an immediately completed
sequence; entity removal/spawn during the element loop; locked FilterAIEvent
FIFO; and creation-order phase wrap. Predicate tests do not prove these
transaction boundaries.

## Sequences

Original launch advances all elements sharing the next command level and
proceeds when the running count reaches zero (`original-code/RHsequence.cpp:
199-220,235-314`). The manager drains FIFO (`original-code/RHsequencemanager.cpp:
931-945`). `ExecutedImmediately` commands run during registration rather than
entering that queue (`RHsequencemanager.cpp:961-970`; command switch in
`original-code/RHsequenceelement.cpp:736-779`).

Rust has matching command-level grouping, FIFO work, deterministic IDs, and an
adapted immediate-action buffer (`crates/robin_engine/src/sequence.rs:2180-2214,
2437-2647`). Existing tests cover grouping, launch/advance, termination, and
immediate-before-deferred ordering around `:3928-4485`.

Confirmed gaps:

- Original WAIT calls `Go()` directly; Rust queues WAIT. This is a frame-timing
  mismatch.
- `register_element_to_go` silently returns for missing sequence/element refs
  (`sequence.rs:2437-2443`). A stale required internal reference must error or
  panic with context, not make an order disappear.
- Every immediate-action entry point must prove it drains before an observer
  that Original placed after the immediate side effect.

## Original script VM and Lua

Original mission scripting is the compiled VM, not Lua. Exact global entry
points are in `original-code/Profile/GEngineScript.cpp:8-146`: Initialize,
Hourglass, CheckVictoryCondition, ProcessMessage, Finalize, PostInitialize. Tick
cadence is `original-code/RHengine.cpp:3582-3621`; PostInitialize is
`RHgame.cpp:1834-1843`; VM globals are serialized at `RHengine.cpp:2787-2818`.
Lua claims must cite Spellforge/upstream evidence separately.

Rust SCB state is engine-owned and serialized. `MissionScript` swaps entities,
AI globals, fast grid, campaign, and mission stats into `GameHost` around calls
(`crates/robin_engine/src/engine/types.rs:1572-1592`). This borrow adapter needs
an exception-safe ownership invariant.

Lua is host-owned because `mlua::Lua` is not serializable
(`crates/robin_lua/src/state.rs:14-18`). At this revision only
Initialize/PostInitialize are called; Lua globals/module cache/callback IDs are
absent from snapshots; required startup can continue without Lua; runtime errors
become zero; startup `math.random` has no installed Engine RNG; and comments
refer to nonexistent `docs/lua.md`.

Status: **incomplete new feature**. Disable it for deterministic modes until its
contract/state policy is tested, and fail launch when a mission marked
`requires_spellforge` cannot create its required interpreter.

## RNG

Original uses process-global C RNG. Production seeds from wall time, tests from
zero (`original-code/launcher.cpp:761-766`). Script `Rand(max)` is
`rand() % max` (`original-code/RHScript.cpp:6492-6505`). Saving creates a time
seed and reseeds immediately; loading restores it, repeating the post-save
sequence (`original-code/RHartificialintelligence.cpp:3833-3863`). Gameplay and
presentation share this stream, so portable bit-identical reproduction is not a
useful target.

Rust's Engine-owned `fastrand::Rng` is serialized, installed for the tick, and
panics on missing/nested scope (`crates/robin_engine/src/sim_rng.rs`; field at
`engine/mod.rs:253-260`). This is an intentional deterministic extension.
Preserve one stream, no host feedback, exact next-state snapshots, fatal replay
decode errors, and explicit Engine RNG context for scripts.

## Saves and rollback snapshots

Original `PerformSnapshot` is only the 160x120 save thumbnail
(`original-code/RHengine.cpp:11140-11158`). Restart persistence is
`RHGame::SerializeForRestart`, which takes that thumbnail then serializes
campaign and engine (`original-code/RHgame.cpp:1265-1297`). Campaign backup then
restart serialization must be last at mission start (`RHgame.cpp:1463-1466`).
Full saves serialize campaign separately, then engine (`RHgame.cpp:2325-2380`).

`RHEngine::Serialize` normalizes active zoom then persists camera/background,
frame/locks, elements, grid, paths, selection, sequences, ground marks, titbits,
VM globals, timers, and AI (`original-code/RHengine.cpp:2408-2875`). These are
baseline state categories, not a reason to retain the binary format.

Rust full saves serialize Engine, host sound, and optional Game persistent state
(`crates/robin_rs/src/save_file.rs:420-475`). Rollback snapshots are narrower:
`SimSnapshot { frame, engine }`. That is correct only if everything else read by
commands/tick is immutable assets or deterministically derived. External display
reads and Lua violate the condition.

Additional risks:

- `game_persistent` is optional for old saves, but the header accepts only the
  current version. Confirm an old version can reach this fallback; otherwise
  require the field.
- `Engine::restore` appropriately reattaches static matching-level data but
  invents all-active grid state for length mismatches.
- Rewind, rollback checking, multiplayer history, and EngineManager should use
  one snapshot schema and replay primitive, with different retention policies.

## Mission ownership

Original uses one required campaign singleton. `RHGame::Serialize` persists it
separately; `RHEngine::QuitMission` obtains the same object and updates it
(`original-code/RHgame.cpp:2360-2378`; `RHengine.cpp:16310-16316`).

Rust intends “outer session owns campaign between missions; Engine owns it
during a mission”:

| State | Current holder | Transfer |
| --- | --- | --- |
| Between missions | outer `Campaign` | `Engine::install_campaign` |
| Mission tick | `EngineInner::campaign: Option<Campaign>` | none |
| SCB call | temporary `MissionScript::game_host.campaign` | `swap_engine_state` |
| Exit/load | outer `campaign_ref` | `Engine::take_campaign` |
| Lua call | SCB `GameHost`, borrowed by host-side Lua | no Lua-owned campaign |

The `Option` helps a checked transfer but is too weak as the mission invariant.
Repeated `unwrap_or_default` makes loss of the authoritative object valid.
`Game::finalize_mission` also uses `std::mem::take` for a test convenience
(`crates/robin_rs/src/game.rs:617-642`).

Target: `MissionRuntime` owns required Campaign and Engine; script hosts receive
scoped access; consuming teardown returns the same campaign. A normal mission
method must not observe `None`.

## Focused test requirements

This audit slice is restricted to docs and Rust comment/TODO edits, so it does
not add executable Rust tests. These focused tests gate the implementation
slices in `REFACTORING.md`:

| Test | Assertion | Covers |
| --- | --- | --- |
| `rollback_preserves_zoom_lock_gate` | Non-default live/replayed zoom produces identical clock, sequence/timer state, display transition, hash. | Snapshot input closure |
| `lua_initialize_random_uses_engine_rng` | Initialize random does not panic and advances the snapshotted Engine stream exactly. | Lua/RNG ownership |
| `lua_required_failure_aborts_launch` | Missing/invalid required Lua returns typed error, never vanilla fallback. | No fake defaults |
| `lua_event_cadence_and_snapshot_policy` | Trace Initialize, PostInitialize, Timer, victory, Finalize; deterministic mode reproduces or rejects before tick zero. | Lua completeness |
| `wait_priority_goes_during_launch` | WAIT side effect exists before launch returns. | Sequence mismatch |
| `sequence_immediate_reentrant_trace` | Immediate completion advances sequence in exact ordered trace. | Immediate barriers |
| `ai_storage_order_visibility_trace` | NPC A's mutation is visible to later NPC B at the Original boundary. | AI global-pass risk |
| `ai_detection_reentrant_done_trace` | EVENT_DONE recursion and final substate occur in the same frame. | AI/sequence coupling |
| `restore_rejects_fast_grid_length_mismatch` | Mismatched arrays return error; no all-active repair. | Snapshot integrity |
| `mission_teardown_requires_campaign` | Missing campaign errors/panics with context; no default object. | Ownership |
| `replay_bad_header_never_uses_seed_zero` | Engine construction is not reached on requested replay decode failure. | RNG integrity |
| `first_tick_post_initialize_trace` | Exact script/entity/sequence/side-effect/PostInitialize order. | Tick adaptation |

Existing grouping/immediate tests in `sequence.rs`, RNG state tests in
`sim_rng.rs`, hourglass/lock tests in `engine/tests.rs`, and rewind retention
tests in `rewind.rs` remain necessary but do not replace cross-subsystem traces.

This was a static source/ownership audit. No runtime trace is treated as
evidence until the focused trace tests above are implemented.
