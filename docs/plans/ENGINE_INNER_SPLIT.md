# Plan: split `EngineInner` by ownership without changing the tick

## Implementation status (2026-07-18)

The physical ownership split described by this plan is implemented.
`EngineInner` is now a short deterministic root over nine cohesive owners:

```rust
pub struct EngineInner {
    mission_domain: MissionDomain,
    control: SimulationControl,
    ai: AiRuntime,
    world: WorldState,
    script_domains: ScriptDomains,
    orders: OrderRuntime,
    scripts: ScriptRuntime,
    players: PlayerRuntime,
    feedback: FeedbackRuntime,
}
```

The live mission domain owns a required `Campaign`; there is no campaign
lease, ownership guard, or GameHost parking slot. Runtime/static mission
attachments are decoded once into `LevelAssets` and enter through the checked
snapshot prepare/adopt/restore path. We therefore did not add the proposed
`EngineBindings` wrapper. The snapshot DTO preserves the supported save shape,
while protocol version 8 intentionally permits the in-memory state-hash layout
change; historical replay compatibility is not retained.

The logical/API split is not finished. The latest sequence wave extracted
attention and stealth/posture contexts and reduced `hourglass_phase_sequences`
from 2,708 to 2,645 lines, but that method still reaches across most owners.
Transition plus mission/message helpers also commonly accept `&mut
EngineInner`. Continue sequence dispatch by coherent command family, then move
transition and mission-gate helpers to explicit minimal borrows.

The field inventory below is the pre-refactor design record. The staged plan
now doubles as a record of completed physical moves and remaining logical API
work; old baseline descriptions should not be read as current code.

## Decision

Keep `EngineInner` as the single deterministic mission-state root, but replace
its 51-field bag with cohesive, owned substructures. Do **not** turn each source
file into a nominal subsystem that still reaches through a shared `&mut
EngineInner`; the useful boundary is ownership and borrowing, not file size.

The originally proposed root was:

```rust
pub struct EngineInner { /* cohesive deterministic owners */ }
```

`EngineInner` remains the root because rollback, rewind, multiplayer snapshots,
save games, deterministic hashing, and the tick all require one atomic world.
The split is not an ECS rewrite, does not introduce independently ticking
objects, and does not reorder `perform_hourglass`.

The implementation instead keeps host configuration host-owned and uses
`LevelAssets` as the sole decoded attachment source. Save restore and exact
network adoption share one atomic validation/attachment path.

## Why this is the right boundary

The current module split has already made the source tree navigable, but 40+
modules still implement methods on one 51-field type. There are 500+ engine
methods, and the dominant methods can mutate any field. This has three concrete
costs:

1. ownership errors are solved with whole-engine methods, temporary `take()`s,
   two-pass collections, and an unsafe `RequiredCampaignGuard` rather than with
   disjoint borrows;
2. it is difficult to tell whether a field is authoritative simulation state,
   a runtime attachment, a derived cache, or a tick output;
3. `EngineInner`'s derived serde and `StateHash` layouts make an in-memory
   organization change accidentally become a save, replay-hash, and network
   protocol change.

The original engine was also a large singleton, but it supplies ordering and
serialization evidence, not an ownership model to copy. In particular:

- constructor/lifecycle setup is in
  `original-code/RHengine.cpp:323-606`;
- the explicit serialized subset and post-load repair are in
  `original-code/RHengine.cpp:2408-3007`;
- the authoritative tick order is
  `original-code/RHengine.cpp:3446-3777`;
- the host calls `PerformHourglass`, refreshes, advances sound, and only then
  calls `PostInitialize` in `original-code/RHgame.cpp:1798-1842`.

Those boundaries must stay observable after the Rust state is regrouped.

## Pre-refactor field inventory and target owners

Legend: **S** means the field is included in serde snapshots today; **H** means
it participates in `StateHash`. Every target state struct must derive
`Clone`, `Serialize`, `Deserialize`, and `StateHash` unless an explicit snapshot
adapter says otherwise. Nested types can still contain deliberately detached
static attachments.

| Current field | Target owner | Persistence | Tick/lifecycle role and principal consumers |
| --- | --- | --- | --- |
| `sim_config` | `EngineBindings` | S, not H today | Attached by `Game`; read for difficulty-dependent gameplay and mission teardown. It is not mutable world state and should eventually stop serializing. |
| `mission` | `MissionRuntime` | S/H | Win/loss/exit gates in `tick.rs`; scripts, commands, console, session UI read it. |
| `frame_counter` | `SimulationControl` | S/H | Read in every tick phase, AI/combat/sound/script cadence, replay/session presentation. Increment remains in `MissionAndMessages`. |
| `sound_sim` | `FeedbackRuntime` | S/H | Sound-source and exclamation deadlines; drained at tick start and advanced in the deterministic tail. |
| `simulation_gates` | `SimulationControl` | S/H | Engine lock, actor freeze, blocking fade. Controls early returns and subsystem work. |
| `speed` | `SimulationControl` | S/H | Script/camera/sequence playback rate. |
| `speed_int` | `SimulationControl` | S/H | Integer UI/control representation paired with `speed`. |
| `weather` | `WorldState` | S/H | Level ambience/forest state; read by AI vision, combat, sprites, scripts, renderer. Despite stale save-file comments, it currently serializes. |
| `shield` | `WorldState` | S/H | Global protection owner; commands, selection and combat read it. |
| `script_globals` | `ScriptRuntime` | S/H | Script global array initialized at level load and persisted across callbacks. |
| `cheat_used_flags` | `MissionRuntime` | S/H | Mission/profile result metadata, updated from commands/tick. |
| `standard_view_polygon_radius` | `AiRuntime` | S/H | Script-configured AI sight default; detection and display-state consumers. |
| `next_order_id` | `OrderRuntime` | S/H | Deterministic ID allocator shared by movement, animation, jump, posture and combat orders. |
| `chorus_timer` | `SimulationControl` | S/H | Global PC speech suppression countdown, decremented at mission/message phase. |
| `force_check` | `MissionRuntime` | S/H | Forces the next script victory check; written by natives and cleared at the existing script barrier. |
| `messenger` | `OrderRuntime` | S/H | Serialized event queue; producers span commands/camera/combat/scripts, sole ordered drain is in the tick. |
| `fast_grid` | `WorldState` | S/H | Runtime spatial topology and active bits. Static `LevelGrid` attachment is rebound after decode. Nearly all entity/movement/AI systems consume it. |
| `pathfinder` | `WorldState` | S/H | Runtime path graph state, coupled to fast-grid topology and movement requests. Static graph comes from `LevelAssets`. |
| `short_briefings` | `MissionRuntime` | S/H | Mission-script-produced briefing state rendered by the session host. |
| `mission_stat` | `MissionRuntime` | S/H | Authoritative per-mission counters, written by AI/combat/natives and committed to campaign at exit. |
| `ground_mark` | `FeedbackRuntime` | S/H | Deterministic destination-marker animation; movement writes it, tail ticks it, renderer reads it. |
| `entities` | `WorldState` | S/H | Canonical entity slots and legacy script-handle order. Central consumer of nearly every gameplay module. |
| `pc_ids` | `WorldState` | S/H | Ordered index into `entities`; selection, loss checks, AI and HUD consumers. Must never become an independently rebuilt cache. |
| `titbit_manager` | `FeedbackRuntime` | S/H | Deterministic floating indicators; gameplay writes, tick updates, render reads. |
| `seats` | `PlayerRuntime` | S/H | Deterministic per-player selection/hotgroups/locks. Commands and selection own mutations; UI reads. |
| `cutscene_camera` | `FeedbackRuntime` | S/H | Shared script/director camera, camera sequence reference, deterministic display-control state. Local viewport stays host-owned. |
| `rng` | `SimulationControl` | S/H | Single serialized deterministic stream; installed around setup/tick/script scopes. |
| `pending_side_effects` | `FeedbackRuntime` | S/H | Per-tick output buffer. Normally drained at snapshot boundaries; serialization deliberately exposes leakage. |
| `user_locked` | `PlayerRuntime` | S/H | Sim-visible user lock read by input/session gating and messages. |
| `qa_recording_for` | `PlayerRuntime` | S/H | PCs participating in current quick-action recording. |
| `qa_recording_slot` | `PlayerRuntime` | S/H | Slot paired with `qa_recording_for`. |
| `action_before_recording_macro` | `PlayerRuntime` | S/H | Selection action restored when macro recording stops. |
| `fast_forward` | `SimulationControl` | S/H | Sim-visible speed/render-skipping mode; affects camera behavior and deterministic tail. |
| `pending_move_requests` | `OrderRuntime` | S/H | Deduplicated AI movement intents, drained at a named tick barrier. |
| `pending_path_requests` | `OrderRuntime` | S/H | Once-per-frame A* work queue; path construction timing must remain unchanged. |
| `failed_path_requests` | `OrderRuntime` | S/H | Path-failure timeout queue; movement, patches and swordfight consumers. |
| `ai_global` | `AiRuntime` | S/H | Global alert, detection and shared AI state. AI modules, combat and scripts consume it. |
| `macro_store` | `PlayerRuntime` | S/H | Authoritative per-PC quick actions. Commands/scripts mutate; HUD reads. |
| `dead_pc` | `MissionRuntime` | S/H | One-shot default-loss trigger set by damage and consumed by mission gates. |
| `timer_elements` | `OrderRuntime` | S/H | Anonymous sequence timers; decremented at the existing post-element barrier. |
| `sequence_manager` | `OrderRuntime` | S/H | Canonical command graph and immediate-action queues. Coupled to entities across most gameplay modules. |
| `pending_reinforcements` | `OrderRuntime` | S/H | Spawn work drained at `DeferredEffectsStart`. |
| `pending_scroll_amulets` | `OrderRuntime` | S/H | Deferred level-asset-dependent spawn work, drained at tick start. |
| `pending_hero_speeches` | `OrderRuntime` | S/H | Deferred speech dispatch, drained at tick start. |
| `pending_hades_kills` | `OrderRuntime` | S/H | Deferred full death cascades, drained at tick start. |
| `pending_concussion_side_effects` | `OrderRuntime` | S/H | Deferred KO/wakeup cascades, drained at tick start. |
| `mission_script` | `ScriptRuntime` | S/H | SCB VM heaps/instances plus the former mirrored `GameHost`; bytecode is reattached from `LevelAssets`. |
| `script_zone_data` | `WorldState` | S/H | Runtime zone occupants/classes parallel to static `LevelAssets::script_zone_grid_indices`. |
| `dynamic_sight_obstacles` | `WorldState` | S/H | Per-frame shield/other dynamic occluders rebuilt in tick order. |
| `static_sight_obstacle_active` | `WorldState` | S/H | Runtime active flags parallel to immutable static obstacle geometry. |
| `campaign` | `MissionDomain` | S/H | Required campaign during an active mission. The migration removed the former `Option` and temporary GameHost ownership transfer. |

This inventory deliberately treats deterministic presentation state as
simulation state. `ground_mark`, titbits, director camera, sound deadlines and
side-effect production cannot move host-side merely because they are rendered
or played there: rollback currently depends on their exact state or outputs.

## Proposed state structs

### `SimulationControl`

```rust
struct SimulationControl {
    frame_counter: u32,
    rng: SimulationRng,
    gates: SimulationGateState,
    speed: f32,
    speed_int: u16,
    chorus_timer: u16,
    fast_forward: bool,
}
```

This owns time and global suspension/rate controls. It does not own the tick
scheduler: `EngineInner::perform_hourglass` retains that responsibility.

### `MissionRuntime`

```rust
struct MissionRuntime {
    state: MissionState,
    campaign: Campaign, // required after the GameHost adapter is removed
    stats: MissionStat,
    short_briefings: ShortBriefings,
    force_victory_check: bool,
    dead_pc: Option<EntityId>,
    cheat_used_flags: u32,
}
```

The implemented `MissionDomain` owns `Campaign` directly for the lifetime of a
live `Engine`; mission teardown consumes the engine and returns that campaign.
This removed the unsafe root-level `RequiredCampaignGuard` and the generic
`take_campaign<T>` compatibility trick.

### `WorldState`

```rust
struct WorldState {
    entities: Entities,
    pc_ids: Vec<EntityId>,
    fast_grid: FastFindGrid,
    pathfinder: PathFinder,
    weather: WeatherState,
    shield: ShieldState,
    script_zones: Vec<ScriptSectorData>,
    dynamic_sight_obstacles: Vec<SightObstacle>,
    static_sight_obstacle_active: Vec<bool>,
}
```

This is the authoritative entity/spatial world. `pc_ids`, zone indices and
obstacle-active arrays remain stored rather than being opportunistically
rebuilt because their order/parallel indexing is observable. Add validation
methods for those relationships; do not silently resize or default them.

### `AiRuntime` and `ScriptRuntime`

```rust
struct AiRuntime {
    global: AiGlobalState,
    standard_view_polygon_radius: u16,
}

struct ScriptRuntime {
    globals: Vec<i32>,
    mission: Option<MissionScript>,
}
```

`ScriptRuntime` owns VM state, not the state on which natives operate. The
GameHost mirror-removal plan should make a script call borrow `WorldState`,
`AiRuntime`, `MissionRuntime`, `OrderRuntime` and `FeedbackRuntime` through a
scoped native context. Nested actor/zone/target/scroll/waypoint calls stay
inside the same outer borrowed context so re-entry sees same-call mutations.

### `OrderRuntime`

```rust
struct OrderRuntime {
    messenger: Messenger,
    sequences: SequenceManager,
    next_order_id: u32,
    timers: Vec<TimerEntry>,
    pending_moves: Vec<(EntityId, AiOrderIntent)>,
    pending_paths: PendingPathRequestQueue,
    failed_paths: Vec<FailedPathRequest>,
    deferred: DeferredGameplay,
}

struct DeferredGameplay {
    reinforcements: Vec<Option<EntityId>>,
    scroll_amulets: Vec<PendingScrollAmulet>,
    hero_speeches: Vec<(EntityId, u16)>,
    hades_kills: Vec<EntityId>,
    concussion: Vec<(EntityId, ConcussionOutcome)>,
}
```

This groups scheduled work but does not make all work asynchronous. Existing
same-call sequence completion, synchronous path construction, script re-entry,
and message drains remain synchronous. A queue may only live here if the
current code already has an explicit barrier for it.

### `PlayerRuntime` and `FeedbackRuntime`

```rust
struct PlayerRuntime {
    seats: Vec<SeatState>,
    macros: MacroStore,
    user_locked: bool,
    qa_recording_for: Vec<EntityId>,
    qa_recording_slot: u8,
    action_before_recording: Action,
}

struct FeedbackRuntime {
    sound: SoundSimState,
    titbits: TitbitManager,
    ground_mark: GroundMark,
    director_camera: CameraState,
    pending: SideEffects,
}
```

The name `FeedbackRuntime` means deterministic feedback production, not
host-owned presentation. Local UI animation and viewport state remain in
`HostDisplayState`; this state remains snapshotted.

## Tick contract: ownership is not scheduling

The existing Rust tick has a test-visible ten-phase spine. Preserve this exact
sequence and all current early returns:

| Order | Existing `HourglassPhase` | Principal state groups |
| --- | --- | --- |
| 1 | `DeferredEffectsStart` | `orders`, `world`, `feedback`, `control` |
| 2 | `MissionAndMessages` | all groups through script/victory/message dispatch; increments `control.frame_counter`; may return |
| 3 | `NpcOrders` | `ai`, `world`, `orders`, `scripts` |
| 4 | `Paths` | `orders`, `world` |
| 5 | `Entities` | `world`, `orders`, `feedback`; storage-order iteration/removal |
| 6 | `EntitySystems` | `world`, `orders`, `feedback`, `ai` |
| 7 | `Npcs` | `world`, `ai`, `orders`, `scripts`, `mission` |
| 8 | `GameplaySystems` | `world`, `ai`, `orders`, `players`, `feedback` |
| 9 | `Sequences` | `orders` plus borrowed access to every group a command can mutate |
| 10 | `DeferredEffectsEnd` | `orders`, `world`, `feedback`, `players` |

The deterministic tail after `perform_hourglass_inner` also stays in order:
overall villain alert, host display transitions/highlights, macro display
phases, ground-mark animation, delayed sound sources, director camera display,
RNG reclaim, and `SideEffects` drain.

The blocking-fade early return must remain before RNG installation and before
any of these phases. `perform_post_initialize` remains a separate lifecycle
method called after first refresh/sound, with its own RNG scope and immediate
action drain. Do not fold it into the first tick while extracting structs.

The original tick is less decomposed, but the observable anchors still match:
mission/script/clock/lock gates precede loss checks, paths and entity
hourglasses; sequence hourglass follows entity hourglasses; titbits, selection
repair and anonymous timers follow sequences. The Rust-only deterministic
adaptations must keep their documented tail positions.

## Borrowing and method design

### Keep root orchestration methods

These remain methods on `EngineInner` or `Engine` because they coordinate
multiple owners or establish an atomic lifecycle boundary:

- construction and level-load phase ordering;
- `attach_level_assets` and snapshot restore;
- `perform_hourglass` and `perform_post_initialize`;
- `apply_command(s)` dispatch;
- mission finish/return-campaign;
- serialization and deterministic hashing.

They should be short orchestration methods, not homes for gameplay rules.

### Put owner-local invariants on the owner

Examples:

- `SimulationControl::{enter_rng_scope, advance_clock, consume_fade_frame}`;
- `MissionRuntime::{record_win, should_exit, finish}`;
- `WorldState::{entity, entity_mut, add_entity_in_order,
  validate_level_attachments}`;
- `OrderRuntime::{allocate_order_id, enqueue_move, tick_failed_paths}`;
- `PlayerRuntime::{seat, selected_ids, begin_macro_recording}`;
- `FeedbackRuntime::{emit, drain_side_effects}`.

Do not add broad `world_mut()`, `orders_mut()` or public fields to the
cross-crate `Engine` facade. Queries can return narrow immutable views; host
mutations remain commands or lifecycle operations.

### Use phase-specific disjoint contexts

For multi-owner behavior, split the root once and pass only the needed borrows:

```rust
struct SequencePhase<'a> {
    control: &'a mut SimulationControl,
    mission: &'a mut MissionRuntime,
    world: &'a mut WorldState,
    ai: &'a mut AiRuntime,
    scripts: &'a mut ScriptRuntime,
    players: &'a mut PlayerRuntime,
    feedback: &'a mut FeedbackRuntime,
}

fn run_sequences(orders: &mut OrderRuntime, cx: SequencePhase<'_>);
```

Use smaller contexts for narrower systems (`MovementContext`,
`DetectionContext`, `NativeContext`). Contexts are ephemeral capabilities, not
state and not serializable. They must not contain `&mut EngineInner` or a raw
pointer back to the root.

This enables normal disjoint field borrowing and prevents a subsystem from
acquiring unrelated state later without changing its signature. It also
removes the need to `take()` an owned subsystem simply to call back into the
engine.

### Commands versus direct borrows

- Use direct borrowed mutation for same-call simulation semantics: damage,
  entity activation, AI focus, path state, sequence completion, and script
  native results.
- Use `SideEffects` only for sim-to-host outputs after a tick.
- Keep an existing deferred queue only at its existing named barrier.
- Do not solve borrow errors by adding a new end-of-frame command queue. That
  would change Original/Rust same-frame behavior.

### Entity order is an invariant

`Entities` keeps legacy slots, and level loading deliberately creates patch FX,
proto animations, civilians, rescue placeholders, soldiers, targets, bonuses,
scrolls and beam-me PCs in script-observable order
(`engine/level_loading.rs:1265-1315`). `WorldState::add_entity_in_order` must
remain the sole slot allocator. Do not split entities into independently
iterated stores or rebuild `pc_ids` from type scans during this refactor.

## Serialization, hashing, rollback and multiplayer

Regrouping fields must not silently change an external schema.

### Introduce an explicit compatibility schema first

Before moving fields, replace the direct derives on `EngineInner` with an
explicit flat snapshot adapter whose fields appear in the current declaration
order. The adapter maps to/from the nested owners. Keep a manual `StateHash`
implementation that hashes the same values in that same order, including the
skipped-field marker at the current `sim_config` position.

This is preferable to `#[serde(flatten)]`:

- saves are JSON today, but multiplayer snapshots use bincode;
- serde flattening is a self-describing-map technique and is not a safe
  bincode compatibility plan;
- replay files record periodic hashes, so changing only field grouping must
  not invalidate old deterministic recordings.

Add a golden test that compares pre-split and adapter JSON shape, bincode bytes
for a fixture, and `state_hash`. If exact bincode compatibility is not retained,
make the incompatibility explicit: bump `SAVE_FORMAT_VERSION` in
`robin_rs/src/save_file.rs` and `NET_PROTOCOL_VERSION` in
`robin_engine/src/multiplayer.rs`. Do not claim compatibility based only on a
successful same-version round trip.

### Snapshot boundaries

At every snapshot boundary, the following must hold:

- RNG is owned by `SimulationControl` rather than installed in TLS;
- `MissionRuntime` owns the required campaign;
- no script call or GameHost lease is active;
- `pending_side_effects` is normally empty, but any leak remains visible in
  serde/hash;
- parallel fast-grid and obstacle arrays match the attached level;
- sequence lookup indices may be rebuilt after decode without inventing
  sequence state.

Rollback clones the complete `Engine`, including required cloneable bindings.
Network/save decode must call one `Engine::attach_level_assets` path before any
tick or script native. That method should delegate to:

- `WorldState`: reattach `LevelGrid`, validate lengths, and reattach every
  entity sprite runtime;
- `ScriptRuntime`: reattach SCB bytecode and static native bindings;
- `OrderRuntime`: rebuild sequence indices;
- `FeedbackRuntime`: restore loaded level size only through the documented
  restore policy.

No attachment may use an empty/default substitute. Missing script programs,
sprite runtime, geometry, or mismatched parallel arrays must error or panic
with context, matching the existing repository rule.

### Post-load fixups

Split `post_load_fixups` by owner but keep one ordered root call. Its current
order matters: reset control/path scratch; force redraw/abort zoom; discard
partial side effects; clear sequence timers; reconcile seat selection against
loaded entities; enqueue stature/action resynchronization. Add an ordered
test so owner extraction cannot commute these steps unnoticed.

## High-coupling seams that drive merge order

### Script/GameHost transaction

`MissionScript::swap_engine_state` and `ScriptContext` currently swap five
authoritative fields into a serialized `GameHost`: `entities`, `ai_global`,
`fast_grid`, `campaign`, and `mission_stat`. Script synchronization then touches
camera, orders, sound, patches, sight obstacles and other state. This is the
largest obstacle to making the groups independently borrowable.

The Engine split should introduce `ScriptRuntime` and the owner groups first,
but must not finalize their APIs around the swap. The GameHost mirror-removal
branch should replace it with a borrowed `NativeContext<'_>` plus explicit
effect capabilities. Once that lands, `ScriptRuntime::call` can borrow the
owners in place and the campaign can become required.

Nested VM calls are not deferrable. `MissionScript::call_actor_function` can
yield a `PendingNestedCall`, recursively call another actor script, and resume
the outer VM while preserving `script_this` and same-call mutations. One outer
native context must remain installed for that full recursion. A context per
nested call that reacquires or snapshots owners risks hiding mutations.

### Sequences and entities

`sequence_manager` and `entities` are jointly mutated across animation,
movement, combat, melee, camera, scripts and transitions. The correct target is
not to put sequences inside `WorldState`; keep their distinct ownership and
make sequence execution accept a `SequencePhase`. Existing two-pass snapshots
can then be reduced locally, but only after tests cover immediate termination,
recursive completion, WAIT, invalid references and actor storage order.

### Fast grid, pathfinder and movement

These belong to one `WorldState` because their topology and entity positions
must agree, but `OrderRuntime` owns the pending/failed request lifecycle. A
`MovementContext` should borrow `&mut WorldState`, `&mut OrderRuntime`, and the
minimal feedback/script capabilities. Preserve synchronous path construction
during `Sequences` and the existing prior-tick retry maintenance in `Paths`.

### HostDisplayState input closure

`perform_hourglass` still mutates `HostDisplayState` while rollback snapshots
clone only `Engine`. Existing timeline code bridges some zoom state. The state
split must not expand this leak. Keep current display arguments unchanged in
the mechanical grouping PRs, then complete the separate snapshot-input audit
before claiming each new group is a closed deterministic unit.

## Historical staged implementation plan

The physical owner extraction, campaign ownership, snapshot attachment, and
facade work have landed. The implementation used `MissionDomain` and
`ScriptDomains` as the final names, retained `SimConfig` on the host rather
than introducing `EngineBindings`, and completed attachment ownership through
`LevelAssets` plus fallible snapshot adoption/restoration. Phase-context work
has extracted movement, target/animation interaction, NPC state, direct
ability, position/lift, and door-pass launch contexts, but the largest sequence
dispatcher and several transition/mission helpers remain future work.

Each stage should compile and have focused tests before the next begins. Avoid
large mechanical method moves in the same commit as an ownership change.

### PR 1: lock the compatibility and ordering contracts

Files:

- `crates/robin_engine/src/engine/mod.rs`
- `crates/robin_engine/src/engine/tick.rs`
- `crates/robin_engine/src/engine/tests.rs`
- `crates/robin_engine/src/engine/rollback_safe.rs`
- `crates/robin_engine/src/replay.rs`
- `crates/robin_rs/src/save_file.rs`
- `crates/robin_rs/src/sim_timeline.rs`

Work:

1. Add a fixture that gives every top-level engine field a distinguishable
   value where practical.
2. Record flat JSON keys/order-independent values, bincode decode/round-trip,
   and deterministic hash.
3. Extend existing phase traces to cover early locked exit, normal tick,
   mission exit, first-frame `PostInitialize`, and deterministic tail outputs.
4. Add ordered level-load entity-slot and post-load-fixup tests.

Invariant: no production behavior or schema changes.

### PR 2: make snapshot/hash layout explicit

Files:

- new `crates/robin_engine/src/engine/snapshot.rs`
- `crates/robin_engine/src/engine/mod.rs`
- `crates/robin_engine/src/engine/rollback_safe.rs`
- `crates/robin_engine/src/replay.rs`

Work:

1. Introduce the flat compatibility snapshot adapter in the current field
   order.
2. Implement `Serialize`/`Deserialize` and `StateHash` for the root through
   that schema.
3. Centralize snapshot-boundary validation and level attachment.

Invariant: fixture JSON meaning, bincode compatibility, state hash, replay
hashes and restore behavior are unchanged. If bincode cannot remain compatible,
bump the save and network versions in this PR and document it explicitly.

### PR 3: extract low-coupling owners mechanically

Files:

- new `engine/state/{control,players,feedback}.rs`
- `engine/mod.rs`, `engine/types.rs`, `engine/simulation_gate.rs`
- direct consumer modules (`camera.rs`, `selection.rs`, `commands.rs`,
  `display_state.rs`, `tick.rs`)

Work:

1. Move fields into `SimulationControl`, `PlayerRuntime`, and
   `FeedbackRuntime` without moving behavior-sensitive phase bodies.
2. Add temporary private forwarding accessors on `EngineInner` where needed.
3. Move only owner-local methods after field moves compile.
4. Delete each forwarding accessor once all internal callers use the owner.

Invariant: phase trace and compatibility fixtures are byte/hash identical.

### PR 4: extract mission, AI and script owners

Files:

- new `engine/state/{mission,ai,script}.rs`
- `engine/mod.rs`, `engine/script.rs`, `engine/types.rs`
- `engine/ai/*`, `engine/level_loading.rs`, `engine/rollback_safe.rs`

Work:

1. Introduce `MissionRuntime`, initially preserving the optional campaign
   adapter.
2. Introduce `AiRuntime` and `ScriptRuntime`.
3. Change `MissionScript::script_context`/`swap_engine_state` signatures to
   accept fields through those owners, without changing the transaction yet.
4. Delegate level-asset attachment and post-load repair to owners under one
   ordered root method.

Invariant: same campaign allocation returns on normal, error and unwind paths;
nested scripts observe same-call changes; PostInitialize boundary is unchanged.

This PR is expected to conflict with GameHost mirror removal in
`engine/types.rs`, `engine/script.rs`, `natives/*` and `level_loading.rs`. Prefer
landing the target borrowed native context first if that branch is already
ready; otherwise keep this PR mechanical and rebase the mirror-removal branch
immediately afterward.

### PR 5: extract `WorldState`

Files:

- new `engine/state/world.rs`
- `engine/mod.rs`, `engine/level_loading.rs`, `engine/rollback_safe.rs`
- direct geometry/entity consumers across movement, combat and AI

Work:

1. Move entity/spatial fields as one unit.
2. Add invariant validation for `pc_ids`, script-zone parallels, static sight
   flags and fast-grid runtime lengths.
3. Preserve the exact level-load sequence and entity slot allocator.
4. Make `attach_level_assets` delegate to `WorldState` and fail loudly on
   missing/mismatched attachments.

Invariant: entity IDs/legacy handles, iteration order, level-load hash and first
tick are identical.

### PR 6: extract `OrderRuntime` and introduce phase contexts

Files:

- new `engine/state/orders.rs`
- `engine/tick.rs`, `engine/movement.rs`, `engine/commands.rs`
- `engine/animation.rs`, `engine/combat.rs`, `engine/melee/*`,
  `engine/transitions.rs`, `engine/script.rs`

Work:

1. Move queues, sequence manager, messenger, timers and order allocator.
2. Introduce minimal `MovementContext`, `SequencePhase`, and AI/script command
   contexts.
3. Convert the highest-coupling functions from `&mut EngineInner` to an owner
   plus context, one subsystem at a time.
4. Remove temporary forwarding accessors and whole-root helpers.

Invariant: no new queue or drain; immediate sequence/script/entity mutations
remain immediate; the ten phase trace is identical.

### PR 7: finish runtime attachments and required campaign

Status: implemented with the attachment design noted above. The live mission
domain owns a concrete `Campaign`, snapshot decoding requires or migrates one
before constructing `EngineInner`, and mission teardown consumes the engine to
return the same allocation.

Depends on the GameHost mirror-removal work.

Files:

- `engine/rollback_safe.rs`, `engine/mod.rs`, `engine/state/mission.rs`
- `robin_rs/src/game.rs`, `game_session/setup.rs`, save/multiplayer restore
  callers

Work:

1. Keep host configuration host-owned and require validated `LevelAssets` at
   snapshot decode/adoption boundaries.
2. Make live `MissionRuntime` own `Campaign` directly.
3. Replace install/take pairs and unsafe guards with consuming mission
   construction/finish APIs.
4. Ensure clone-based rollback carries bindings, while network/save decode
   requires reattachment.

Invariant: every mission exit returns the exact campaign allocation; saves,
rewind, rollback and join snapshots cannot tick while detached.

### PR 8: consolidate facade and module APIs

Files:

- `engine/mod.rs`, `engine/rollback_safe.rs`, `engine/state/*`
- downstream read-only call sites in `robin_rs`

Work:

1. Keep `Engine`'s cross-crate mutation API narrow: tick, commands, explicit
   lifecycle and drains only.
2. Replace downstream field-shaped getters with stable query methods/views.
3. Remove obsolete `EngineInner` forwarding accessors, `take()` workarounds,
   raw-pointer guards and stale ownership comments.
4. Update `docs/REFACTORING.md` after all invariants are actually satisfied.

Invariant: downstream code cannot obtain `&mut EngineInner` or an unrestricted
mutable owner.

## Verification for every stage

Run the smallest relevant set first, then the full ladder before merge:

1. focused owner/phase/serialization tests in `robin_engine`;
2. `cargo test -p robin_engine`;
3. affected `robin_rs` save/timeline/multiplayer tests;
4. `cargo test` and `cargo build --bin robin`;
5. replay a fixed demo and full-game corpus, comparing per-frame hashes and
   ordered traces from the same initial snapshots;
6. save/load before tick, during normal play, during zoom, with active
   sequences, and immediately around script callbacks;
7. two-peer multiplayer plus mid-mission join/adoption and late-input rollback.

Do not update a golden hash or trace merely because a regrouping changed it.
First prove either exact compatibility or an intentional versioned break.

## Risks and explicit non-goals

### Highest risks

- changing tick or entity iteration order while moving methods;
- invalidating recorded replay hashes by deriving `StateHash` over newly
  grouped fields in a different order;
- making bincode network snapshots incompatible while JSON tests still pass;
- hiding same-call script or sequence mutations behind a new deferred queue;
- temporarily allowing campaign or level assets to be absent and filling them
  with defaults;
- treating deterministic feedback state as host-only presentation;
- producing broad context structs that simply recreate `&mut EngineInner`.

### Non-goals

- no ECS conversion or per-entity storage split;
- no scheduler/tick reorder;
- no rewrite of AI, combat, movement or sequence behavior;
- no change to the RNG algorithm or draw order;
- no Lua semantics work beyond accommodating its eventual borrowed native
  context;
- no save-format break unless explicitly versioned and justified;
- no source-file-only shuffle presented as an ownership refactor.

## Coordination with the other two refactors

### GameHost mirrored-state removal

That refactor defines the final `ScriptRuntime` boundary. Agree on the owner
names and a borrowed `NativeContext` before either branch moves script-facing
fields. The mirror-removal work should land before PR 7 and preferably before
PR 6's script contexts. Engine splitting must not preserve the five-field swap
behind a prettier owner facade.

### `run_mission` refactor

The session refactor should depend only on the public `Engine` facade and a
mission-level owner such as `MissionRuntime` in `robin_rs`; it must not reach
into the new engine owner structs. Coordinate lifecycle APIs for:

- fully attached construction;
- first refresh/sound/PostInitialize;
- save/restore and multiplayer snapshot adoption;
- tick/command entry;
- consuming finish that returns campaign.

The two branches can proceed in parallel if `run_mission` keeps current facade
calls initially. Land facade signature changes after its owning runtime struct
exists, to avoid recreating the monolithic local-variable list around new
engine internals.

## Completion criteria

The split is complete when:

- every former top-level field has one documented owner;
- `EngineInner` is a short deterministic root over those owners;
- major systems accept minimal owner/context borrows rather than `&mut
  EngineInner`;
- campaign and runtime bindings are required at every tick boundary;
- save, hash, replay, rollback and network contracts are explicit and tested;
- level attachments have one validated reattachment path;
- original/Rust tick phase and entity-creation order are unchanged;
- the GameHost no longer owns mirrored engine state;
- no unsafe ownership guard, fake default, or new timing-changing queue was
  introduced to make borrowing compile.
