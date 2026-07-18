# GameHost mirror-removal plan

## Implementation status (2026-07-18)

The direct GameHost mirrored-state and ownership-transfer scope is implemented.
`GameHost` is no longer a second world: it owns no entities, AI state, grid,
campaign, mission stats, doors, patches, buildings, scrolls, UI flags, query
caches, or immutable level data. `swap_engine_state`, its refresh caches,
campaign leases, and engine-side guard/reach-through access have been removed.
Native dispatch now borrows canonical engine domains.

The remaining serialized `GameHost` name denotes only a seven-queue script
adapter/effect shell:

```text
commands                    completed_sequences
production_registrations    sound_commands
production_points           deferred_commands
pending_objective_changes
```

The first semantic cleanup wave is also complete. Dynamic sight obstacles and
active flags are live borrowed world capabilities rather than copied script
bindings. Scroll status and PC selection now mutate canonical state before the
native returns, with same-callback tests; their queued follow-up work is limited
to engine/sequence/UI effects that require the existing callback barrier.

The broader queue audit is not finished. Direct `LockAI` and the Honolulu branch
of `SetActorLocation(NULL)` now mutate the canonical NPC AI before returning,
so same-callback `UnlockAI` observes them. The serialized deferred enum slot is
retained only to consume undrained requests from older saves; it has no live
producer. Completed sequence launch, actor stop, global freeze,
posture/location side effects, patch work, and other deterministic variants
still need Original-order characterization before they can move or be
certified as true barriers. The next wave should migrate them in coherent
command families with same-callback tests. Renaming the queue shell and
narrowing the legacy Lua adapter can follow once that boundary is stable.

The inventory and PR sequence below are retained as the pre-refactor design
record. References there to current swaps, mirrors, or a world-sized GameHost
describe the old implementation.

## Outcome

`GameHost` has stopped being a serialized second world. The architectural
target is a
short-lived `NativeContext<'_>` which borrows the authoritative simulation
state for one VM resume, a small serialized `ScriptState` for data that truly
belongs to the script subsystem, immutable `ScriptBindings<'_>` borrowed from
`LevelAssets`, and typed outputs for presentation-only effects.

The important constraint is behavioral, not just structural: a script native
in the Original calls directly into live engine objects. A mutation followed
by a query in the same SCB callback must observe the mutation. Moving a field
out of `GameHost` is not successful if it turns a same-call mutation into a
post-script drain.

The end state is:

```text
EngineInner / extracted engine subsystems       MissionScript
  Entities                                        VM instances
  AiGlobalState                                   ScriptState
  FastFindGrid                                      globals
  Campaign                                          computed locations
  MissionStat                                       sequence recorder
  WorldInteractables (doors, patches, buildings)   post_initialized
  MissionUiState (deterministic UI controls)
  SoundSimState
          ^                                      LevelAssets
          | borrowed                               static script tables
          +--------- ScriptSession ---------------- profiles / paths
                         |
                         +-- NativeContext<'resume>
                               live engine capabilities
                               &mut ScriptState
                               ScriptBindings<'resume>
                               ScriptCallFrame
                               presentation effects
```

There must be one authoritative owner at every migration step. A temporary
ownership transfer is acceptable while the old adapter exists; a refreshed
copy which can disagree with its source is not.

## Pre-refactor baseline

`MissionScript` serializes a concrete `GameHost` alongside the global and
per-object VMs (`crates/robin_engine/src/engine/types.rs`). Before script
execution, callers transfer `Entities`, `AiGlobalState`, `FastFindGrid`,
`Campaign`, and `MissionStat` into that host with `swap_engine_state` or
`ScriptContext`. After execution they transfer those values back and run
`EngineInner::sync_game_host_post_script`.

That adapter has accumulated four additional mechanisms:

1. `refresh_game_host_entity_state` copies live entity, selection, animation,
   alert, sound, ambiance, and forest values into query caches.
2. `install_script_static_data_into_game_host` clones immutable level tables
   and installs sight obstacles in process-thread TLS.
3. natives mutate queues (`commands`, `sound_commands`, `deferred_commands`,
   completed sequences, objectives) which are drained later.
4. engine code outside script dispatch reads and writes doors, patches,
   buildings, scrolls, and UI flags by reaching through
   `mission_script.game_host`.

The resulting type is simultaneously a VM host, a save-game root, a world
subsystem, a cache, a call stack, and an effect buffer. The paired swaps are
not copies, but they make ownership depend on call phase, require an identity
lease for `Campaign`, and leave empty/default parking values on one side.
The refresh maps are real mirrors and can be stale within a frame.

### Current call sequence

The main one-second callback in `engine/tick.rs` is representative:

```text
refresh_game_host_entity_state
set GameHost.frame_counter
swap five engine fields into GameHost
MissionScript::hourglass -> ScriptInstance -> Vm -> GameHost::call
swap five fields back
sync_game_host_post_script
```

Zone, actor, target, scroll, waypoint, victory, finalization, and initialization
callbacks repeat variants of the same transaction. Some paths use the RAII
`ScriptContext`; many still use the paired public swap API. The campaign lease
proves allocation identity, but it does not make the phase-dependent owner easy
to reason about.

## Field inventory and destination

The table accounts for every current `GameHost` field. “Canonical engine” also
includes state which is currently authoritative only in `GameHost` but is used
by normal engine systems and therefore belongs in an extracted engine
subsystem, not in the native dispatcher.

| Classification | Current fields | Target owner and notes |
| --- | --- | --- |
| Persistent script state | `globals` | `MissionScript::state.globals`. Remove the parallel, currently disconnected `EngineInner::script_globals` API after save migration; do not keep both. |
| Persistent script state | `computed_locations`, `computed_location_layers` | A single `Vec<ComputedScriptLocation>` in `ScriptState`; handles stored in VM heaps require this to serialize and hash. |
| Persistent script state | `recording` | `ScriptState::sequence_recorder`. An open `Start`/`Thanx` recording can cross native invocations and is deterministic state. |
| Persistent script state, pending audit | `sequence_id` | It is assigned by `Start`/`Then` but currently never read. Characterize shipped scripts/saves, then delete it if confirmed vestigial; otherwise fold it into `SequenceRecorderState`. |
| Canonical engine state, currently transferred | `entities`, `ai_global`, `fast_grid`, `campaign`, `mission_stat` | Borrow their one engine-owned instances through `NativeContext`. Delete `CampaignIdentity`, `campaign_lease`, swaps, and parked defaults only after the last native is migrated. |
| Canonical engine state, currently mirrored | `ambiance`, `is_forest_level`, `sound_source_alive`, `entity_active`, `current_animations`, `pc_auth_bits`, `pc_handles`, `selected_pc_handles`, `robin_handle`, `pc_profile_map`, `any_civilian_dead`, `any_enemy_dead`, `overall_enemy_alert`, `overall_civilian_alert`, `frame_counter` | Query `WeatherState`, `SoundSimState`, `Entities`, `SequenceManager`, `pc_ids`/seat selection, and the frame clock directly. Delete both refresh methods and their call sites. Derived queries may use functions, not stored snapshots. |
| Canonical engine state, duplicate with wrong owner | `campaign_values` | Use `Campaign::values[Custom1..Custom20]`, as `RHScript::Get/SetCustomCampaignValue` does in `original-code/RHScript.cpp:7225-7266`. Migrate old save data and reject conflicts. |
| Canonical engine state, duplicate with wrong owner | `npc_values` | Use the existing NPC entity `custom_values: [i32; NpcCustomValue::COUNT]`, matching `RHElementActorNPC::maslCustomValues` (`RHelementactornpc.h:114,426-427`). Migrate old save entries by stable actor handle and reject non-NPC/conflicting entries. |
| Canonical engine world state, misplaced | `doors`, `patches`, `building_occupants`, `arrow_reserves`, `actor_building`, `building_active`, `building_gates`, `scroll_status`, `scroll_attachments`, `scroll_attachment_dirty`, `men_to_blazon_conversion_mode`, `blinking_blazons`, `blink_expire_frame` | Extract to named engine-owned state (`WorldInteractables`, `BuildingState`, `ScrollState`, `MissionUiState`, or the equivalent agreed with the EngineInner split). These values are read and mutated by movement, AI, input, rendering facades, patch effects, titbit sync, and commands outside VM calls. |
| Canonical engine state, known duplicate | `zone_occupants` | Delete it. `EngineInner::script_zone_data[*].occupant_indices` is already authoritative. Native add/remove/query operations must use that list directly. `IsInside` still performs its live geometric check after teleports. |
| Canonical engine state or direct initialization work | `production_registrations`, `production_points` | Apply synchronously to canonical production-sector state during the initialization session. If initialization ordering requires buffering, put a typed buffer in `ScriptSession`, consume it before the session ends, and assert it is empty at snapshot boundaries. |
| Canonical deterministic UI state | `outline_display`, `force_check` | Borrow/write `MissionUiState`/engine victory scheduling directly. `SetOutlineDisplay` followed by `GetOutlineDisplay` must round-trip within one callback while emitting a presentation change only when the canonical value changes. |
| Immutable level binding | `profile_manager`, `script_location_count`, `script_point_count`, `script_building_count`, `script_hiking_path_count`, `hiking_paths`, `location_positions`, `location_layers`, `location_sectors`, `script_zone_polygons`, `map_bbox`, `sector_kinds`, `patch_animation_entities` | Read through `ScriptBindings<'_> { assets: &LevelAssets, level_grid: ... }`. `script_zone_polygons` is derivable from the indexed grid; `sector_kinds` is a query over sectors; `patch_animation_entities` already exists as `LevelAssets::patch_entity_handles`. Do not serialize cloned copies. |
| Immutable mod binding | `lua_actor_names`, `lua_item_names`, `lua_location_names`, `lua_patrol_names`, `lua_scroll_names` | Move to immutable level/mod metadata, ideally `LevelAssets::script_names`, and borrow it. If loading proves names are generated after `LevelAssets` construction, create one immutable `Arc<ScriptNameBindings>` at mission setup and reattach it after load. |
| Derived canonical sound state | `sound_source_count` | Query the stable slot table in `SoundSimState`; source destruction changes liveness, not handle range. Keep one API which distinguishes invalid index from destroyed slot. |
| Transient call context | `script_this`, `current_scroll` | `ScriptCallFrame`, pushed by `ScriptSession` and restored structurally. It is not serialized state. Prototype calls need an explicit “inherit outer This” policy. |
| Transient nested-call control | `pending_nested_call`, `nested_call_depth` | Return a typed VM yield carrying the request; derive depth from `ScriptSession`'s call stack. Do not store the request in both host and VM. Assert no active call/yield exists when taking a save or rollback snapshot. |
| Runtime configuration | `verbose` | Interpreter/native tracing configuration, attached to the session and excluded from simulation state/hash. |
| Emitted presentation/external effect | `commands`, `sound_commands`, `background_invalidated`, `pending_objective_changes` | Typed `ScriptEffects`/existing `SideEffects`. Only renderer, audio backend, dialogs, camera presentation, and objectives belong here. Deterministic state changes paired with them happen synchronously first. |
| Emitted deterministic work, not automatically safe to defer | `deferred_commands`, `completed_sequences` | Replace each variant with a synchronous method on the relevant canonical engine subsystem unless an Original-order audit proves it unobservable until callback return. Sequence launch, stop/preemption, messages, patch application, selection, AI locks, posture side effects, scroll status, and actor location are presumed observable and need same-call tests. |

This inventory intentionally does not preserve `GameHost` as the owner of
doors/patches merely because it is their current single owner. A type used only
while interpreting a native must not be the world database used by movement
and rendering for the rest of the frame.

## Target APIs

### Script-owned state

```rust
#[derive(Clone, Serialize, Deserialize, StateHash)]
pub struct ScriptState {
    pub globals: BTreeMap<i32, i32>,
    pub computed_locations: Vec<ComputedScriptLocation>,
    pub sequence_recorder: SequenceRecorderState,
}

#[derive(Clone, Serialize, Deserialize, StateHash)]
pub struct ComputedScriptLocation {
    pub position: MapPoint,
    pub layer_sector: Option<(u16, u16)>,
}
```

`MissionScript` owns this beside its VMs. It must not contain entity, campaign,
grid, door, patch, sound, or selection copies.

### Borrowed bindings and live capabilities

The first implementation may use one engine borrow while `MissionScript` is
leased out of `EngineInner`; after EngineInner is split, this should narrow to
domain capabilities.

```rust
pub struct ScriptBindings<'a> {
    pub assets: &'a LevelAssets,
    pub names: Option<&'a ScriptNameBindings>,
}

pub struct NativeContext<'call, 'level> {
    script: &'call mut ScriptState,
    engine: &'call mut EngineInner, // transitional; narrow after subsystem split
    bindings: ScriptBindings<'level>,
    call: ScriptCallFrame,
    effects: &'call mut ScriptEffects,
}
```

The final form should expose methods/capabilities such as `entities()`,
`sequence_control()`, `world_interactables()`, `campaign()`, `sound_sim()`, and
`presentation_effects()` rather than public fields. This keeps native code from
creating a new cache and makes required-state failures contextual. Do not put
`&mut EngineInner` behind an unsafe raw pointer to avoid borrow errors.

`MissionScript` must be temporarily removed from `EngineInner` before creating
the transitional context. Use an unwind-safe `MissionScriptLease`, analogous
to the required campaign guard, which restores the exact script allocation and
panics if a second script was installed. Once EngineInner owns logical
substructures, `ScriptSession` can borrow disjoint fields without taking the
whole engine.

### HostFunctions without a `'static` trap

`ScriptInstance`, `Vm::run_up_to_with_host`, and related production APIs
already accept `&mut dyn HostFunctions` for the duration of execution. The
trait itself prevents a borrowed context because it inherits `Any` (`Any`
requires `'static`) and requires `as_any`, `as_any_mut`, `clone_dyn`, and
`Send` for owned test/tool convenience.

Change the production trait to a lifetime-neutral dispatcher:

```rust
pub trait NativeDispatch {
    fn call(&mut self, index: u32, stack: &mut NativeStack) -> NativeOutcome;
}

pub enum NativeOutcome {
    Return(i32),
    CallScript(PendingNestedCall),
}
```

Then make the convenience wrapper generic:

```rust
pub struct VmWithHost<H> { pub vm: Vm, pub host: H }
impl Vm {
    pub fn with_host<H: NativeDispatch>(self, host: H) -> VmWithHost<H> { ... }
}
impl<H: NativeDispatch> VmWithHost<H> {
    pub fn take_host(self) -> H { self.host }
}
```

The examples and native tests can inspect `GameHost`-like fixtures directly;
they no longer need `Any` downcasts. `clone_dyn` has no call sites and should
not survive. Keep `Send` only on an owned wrapper which actually crosses a
thread, not on synchronous native dispatch.

During migration, `GameHost::call` and `NativeContext::call` can share
per-native helper functions. Do not add a blanket delegation which copies a
whole context back into `GameHost`.

### Effects versus immediate mutations

Use three explicit channels:

- `NativeOutcome::Return` for a completed synchronous call;
- `NativeOutcome::CallScript` for a VM yield requiring nested script dispatch;
- `ScriptEffects` for presentation/external outputs which cannot be queried as
  simulation state by the running script.

Everything else is a live engine method. For example, `SetActorLocation`
updates map position, layer/sector, obstacle, display order, Honolulu state,
playability/AI lock, swordfight, and titbits before it returns. A renderer
invalidation may be emitted, but the actor mutation may not wait for
`sync_game_host_post_script`. The same rule applies to patches: patch state and
collision/sight consequences are synchronous; background redraw is an effect.

## Nested `PrototypeFilterEvent`

The current mechanism is conceptually sound but stores the request twice:

1. `GameHost::call` writes `GameHost.pending_nested_call` and returns placeholder
   zero.
2. the interpreter takes it, stores it in `Vm.pending_nested_call`, advances
   past `NativeCall`, and returns `StopReason::PendingNestedCall`;
3. `MissionScript` takes it again, recursively calls the target VM, writes the
   result into the outer VM's `native_return_value`, and resumes at
   `Aff1NativeGetReturn`.

Replace this with `NativeOutcome::CallScript(request)`. The VM advances its IP
and returns `VmYield::NestedCall(request)` by value. At that point the
`NativeContext<'_>` borrow has ended, so `ScriptSession` can recursively run the
target VM against the same live engine state and `ScriptState`, patch the outer
return register, then construct a fresh short-lived context to resume it.

Depth is `ScriptSession.call_stack.len()`, not serialized host state. The stack
also restores `ThisActor` and `ThisScroll` on every Rust return/unwind path.

There is a parity issue to resolve before making the new trace golden:
`RHScript::PrototypeFilterEvent` says “the script-This is not changed” and
directly invokes the prototype NPC (`original-code/RHScript.cpp:6508-6535`).
The current recursive `call_actor_function` sets `script_this` to the prototype
for the nested call. Its test also documents a second divergence: an unbound
prototype returns `0`, while the Original base `FilterAIEvent` allows with `1`.
Add characterization tests for nested return propagation, outer `ThisActor`,
source/event parameters, A→B→A recursion, missing override, error unwinding,
and depth limit. Land any parity correction as a small, provenance-backed
behavior PR before the mechanical context migration.

## Lua integration

`robin_lua` registers `'static` mlua closures. `MissionLuaState::with_host`
currently puts `HostPtr(Cell<*mut GameHost>)` in Lua app data, unsafely
dereferences it in every shim, and relies on removing it after the callback.
The pointer is scoped by convention and has an unsafe `Send` implementation.
It also lets Lua bypass the engine-state swap contract via `game_host_mut`.

Both SCB and Lua should call the same `NativeDispatch` implementation. Prefer
mlua's scoped callback facility:

1. create a scoped dispatcher closure borrowing `&mut NativeContext<'_>`;
2. expose the native names in the event environment (or have permanent thin
   shims call a hidden scoped dispatcher);
3. invoke the Lua event synchronously;
4. remove the environment/dispatcher before leaving the scope. A Lua function
   or coroutine which captured it must fail after scope exit rather than retain
   engine access.

If the installed mlua version cannot express this with `Lua::scope`, retain a
single, narrowly audited raw pointer adapter temporarily, but point it at
`dyn NativeDispatch`, attach a nonzero call-generation token, reject nested or
out-of-generation access, and remove app data with an RAII guard on success,
error, and panic. Do not make the borrowed context `'static`, clone engine
state, or add another process TLS slot to satisfy mlua.

Lua remains subject to the repository's deterministic-mode policy. This
refactor must not imply that unsnapshotted Lua state is rollback-safe.

## Historical staged implementation

Each stage compiles, has one owner for every migrated value, and can be
reviewed independently.

The canonical-owner portions of PRs 1–7 have landed, including borrowed native
contexts, engine-owned script domains, removal of GameHost caches/swaps/leases,
and snapshot attachment validation. PR 6's deterministic queue conversion is
still required for correct same-callback semantics. PR 8's Lua/shell cleanup
remains separate follow-up work.

### PR 1: behavior and ownership characterization

Files:

- `crates/robin_engine/src/engine/filter_ai_event_tests.rs`
- `crates/robin_engine/src/natives/tests.rs`
- focused tests under `crates/robin_engine/src/engine/`

Add table-driven same-callback set/get tests for entity active state, location,
posture/AI state, campaign/NPC custom values, patch applied state, door state,
scroll state, building/zone membership, outline state, current animation after
sequence operations, and `ForceCheckVictory`. Add a test-only ordered trace of
native call, immediate sim mutation, nested callback, sequence launch, and
effect emission. Add save/hash tests at legal pre/post-callback boundaries and
assert snapshots cannot be taken during an active script session.

Resolve the two `PrototypeFilterEvent` parity questions separately here. This
PR intentionally changes no ownership.

### PR 2: make native dispatch borrow-friendly

Files:

- `crates/robin_engine/src/interp.rs`
- `crates/robin_engine/src/script_manager.rs`
- `crates/robin_engine/src/natives/mod.rs`
- `crates/robin_engine/src/natives/tests.rs`
- `crates/robin_rs/examples/{run_script,batch_run}.rs`

Remove `Any`, `Send`, downcast helpers, and `clone_dyn` from the production
trait; make `VmWithHost` generic. Introduce `NativeOutcome`, then carry nested
requests out of the VM by value. Preserve IP advancement and return-register
semantics exactly. This PR still uses `GameHost` as the concrete production
dispatcher.

### PR 3: extract true script state and immutable bindings

Files:

- new `crates/robin_engine/src/natives/state.rs` and `context.rs` (or a new
  `script_native/` module if agreed before implementation)
- `crates/robin_engine/src/natives/mod.rs`
- `crates/robin_engine/src/engine/types.rs`
- `crates/robin_engine/src/engine/script.rs`
- `crates/robin_engine/src/engine/level_loading.rs`

Add `MissionScript::state: ScriptState`. Move globals, computed locations, and
the recorder directly—never leave forwarding copies in `GameHost`. Change
static-data native helpers to accept `ScriptBindings` and delete each host
clone immediately after its last use. Replace `SCRIPT_SIGHT_OBSTACLES` TLS with
live static/dynamic/active obstacle borrows in the context. Add reattachment
tests for save/load and two simultaneous engines with different levels.

For save compatibility, version the serde snapshot. Deserialize the legacy
shape into a dedicated compatibility DTO, normalize it once into the new owner,
and fail on contradictory values. Do not retain deprecated runtime fields with
`#[serde(default)]` indefinitely.

### PR 4: move canonical custom values and query mirrors

Files:

- `crates/robin_engine/src/natives/mod.rs`
- `crates/robin_engine/src/campaign.rs`
- `crates/robin_engine/src/element.rs`
- `crates/robin_engine/src/engine/script.rs`
- `crates/robin_engine/src/engine/mod.rs`
- `crates/robin_engine/src/engine/rollback_safe.rs`

Route campaign and NPC custom values to their existing canonical storage and
migrate old saves. Then migrate read-only native groups to `NativeContext`:
entity/type/property queries, selection/PC aggregates, animation queries,
weather, sound source liveness, frame counter, grid and profile queries. Delete
each refresh cache and refresh assignment in the same commit which migrates its
last reader. Finally delete `refresh_game_host_entity_state` and
`refresh_game_host_pc_auth_bits`.

### PR 5: establish engine-owned world-script subsystems

Files depend on the EngineInner split, principally:

- `crates/robin_engine/src/engine/mod.rs`
- `crates/robin_engine/src/engine/types.rs`
- `crates/robin_engine/src/engine/{level_loading,script,movement,input,patch_effects,door_pass,scroll_reveal,titbit_sync}.rs`
- AI/melee/render-facing facade callers identified by `game_host()` searches

Move doors, patches, building/occupancy state, scroll state, zone access, and
deterministic UI controls into the agreed engine substructures. First change
non-native engine consumers to use named `EngineInner` methods; then move the
storage; then point natives at the same methods. Delete `zone_occupants` rather
than moving it. This order avoids a period with two writable stores.

Do not combine all domains in one PR. A sensible sequence is zones, scrolls,
buildings, doors/patches, then mission UI. Every domain PR removes all
`mission_script.game_host` reach-throughs for that domain.

### PR 6: replace deferred deterministic commands

Files:

- `crates/robin_engine/src/natives/commands.rs`
- `crates/robin_engine/src/natives/mod.rs`
- `crates/robin_engine/src/engine/script.rs`
- `crates/robin_engine/src/engine/commands.rs`
- `crates/robin_engine/src/sequence.rs` and affected engine domain modules

Audit every `EngineCommand` and `DeferredCommand` against Original call timing
and the PR 1 traces. Invoke deterministic mutations synchronously through the
context. Retain only typed presentation/external effects. Remove corresponding
arms from `sync_game_host_post_script` as their producers migrate. Sequence and
message variants require special care because they can synchronously invoke
callbacks or change `GetCurrentAction`/selection before the outer script
returns.

At the end of this PR, `sync_game_host_post_script` is either a small transfer
of presentation effects into `pending_side_effects` or gone.

### PR 7: replace all swap sites with `ScriptSession`

Files:

- `crates/robin_engine/src/engine/types.rs`
- `crates/robin_engine/src/engine/script.rs`
- `crates/robin_engine/src/engine/tick.rs`
- all actor/zone/target/scroll/waypoint callback sites

Introduce one entry API for global and object callbacks. It installs
`ScriptCallFrame`, constructs a short-lived `NativeContext` for each VM resume,
handles nested yields, and restores the exact `MissionScript` on errors and
unwinds. Convert all paired swaps and `ScriptContext` users. Then delete
`swap_engine_state`, `ScriptContext`, campaign leasing, `game_host()` and
`game_host_mut()`.

### PR 8: future Lua cleanup and queue-shell rename/removal

Files:

- `crates/robin_lua/src/{natives,state}.rs`
- `crates/robin_lua/tests/natives_smoke.rs`
- `crates/robin_rs/src/{lua_session.rs,game_session/mod.rs}`
- `crates/robin_engine/src/natives/mod.rs`

Run Lua events through `ScriptSession`/`NativeContext`, replace `HostPtr` with
scoped dispatch, and add stale-callback/coroutine/error cleanup tests. Convert
native fixtures to purpose-built owned test contexts. Delete `GameHost` once
`rg 'game_host|GameHost'` contains only legacy save DTOs/provenance text.

## Invariants and validation

Every PR must keep these invariants:

1. At a legal snapshot boundary, campaign, entities, AI global state, grid,
   mission stats, doors/patches/buildings/scrolls, and script state each have
   exactly one serialized owner.
2. A native reads live state, not a pre-callback or pre-frame snapshot.
3. A deterministic mutation which the Original performs inline is visible to
   the next instruction/native and to nested callbacks before the outer VM
   resumes.
4. Presentation effects are ordered, emitted once, and never fed back into the
   same deterministic tick except through explicit later commands.
5. Nested calls restore the outer VM, return register, `ThisActor`, scroll
   context, RNG scope, and call depth on success, error, and panic.
6. Missing required campaign, level binding, entity kind, or handle mapping
   errors/panics with context according to the native contract; migration never
   supplies an empty host/default world.
7. No native access depends on process-global sight data. Two engines on one
   thread and engines on separate threads use their own assets/state.
8. Rollback and multiplayer hashes are unchanged by mechanical ownership PRs.
   A separately approved parity fix must update a provenance-backed ordered
   trace, not silently ride with a field move.

Validation ladder:

- focused `robin_engine` native, nested-call, save, state-hash, and domain tests;
- `cargo test -p robin_engine` and, when touched, `cargo test -p robin_lua`;
- `cargo test` and `cargo build --bin robin` for integration stages;
- `verify_rollback` plus a fixed replay corpus covering mission startup,
  scripted teleports, patches/doors, scrolls, buildings, victory/finalize, and
  nested AI filters;
- two-peer multiplayer replay with a late command crossing a script-second
  boundary;
- a Lua mission test for successful event, native error, coroutine capture,
  and teardown;
- compare per-frame state hashes and the ordered native/sequence trace. Do not
  accept a changed golden merely because final state happens to match.

## Dependencies and conflict handling

### EngineInner logical split

This work should consume, not compete with, the EngineInner split. Agree on the
destination structs for interactables, buildings, scrolls, mission UI, and
script runtime before PR 5. The EngineInner work may introduce those empty or
existing-field-owning structs first. This plan then moves GameHost state into
them domain by domain. Avoid simultaneous broad edits to `engine/mod.rs`,
`engine/types.rs`, `level_loading.rs`, and `script.rs`.

The borrow-friendly trait work, characterization, ScriptState extraction, and
static-binding removal can land before that split. Do not wait for a perfect
EngineInner decomposition to remove `Any` or query caches.

### `run_mission` refactor

The session refactor should treat script execution as one engine API such as
`MissionRuntime::dispatch_script_event`, never access `MissionScript` or native
state directly. It may introduce `MissionRuntime` ownership and teardown first,
but should not build new `game_host_mut` bridges or Lua host swaps. PR 8 should
then change only the implementation behind that API.

### High-conflict files

`natives/mod.rs` is already very large. Extract helper modules by native domain
before or during migration, but do not mix a whole-file mechanical split with
behavior changes. `engine/script.rs` is the other convergence point; delete
sync/refresh code in small domain commits so rebases are reviewable.

## Completed canonical-owner criteria

The canonical-owner migration is complete because:

- every canonical engine value has one serialized owner;
- no canonical field is swapped into `GameHost` or refreshed into a query
  cache;
- immutable mission/script attachments come from validated `LevelAssets`;
- native dispatch operates on borrowed live engine domains;
- engine, renderer, input, tick, and command code have no guard-only or
  canonical-state `game_host()` reach-through; and
- save/load, rollback, multiplayer snapshot adoption, the full workspace test
  suite, the Robin binary build, and a freshly recorded replay pass.

This is not the semantic end state: remaining deterministic queues still need
same-callback visibility audits, even though sight, scroll status, and selection
now use live canonical state.

## Long-term script-adapter end state

The broader adapter cleanup will be complete when:

- `MissionScript` serializes VMs plus the small `ScriptState`, not a world host;
- no engine field is swapped into a native dispatcher or refreshed into a
  query mirror;
- immutable script data is borrowed from reattached level/mod assets;
- SCB and Lua use the same borrowed native-dispatch surface;
- process TLS no longer supplies sight obstacles to natives;
- nested script calls yield and resume without serialized host call state;
- deterministic native mutations have tested same-call visibility;
- engine/render/input code has no `game_host()` reach-through;
- `GameHost`, `ScriptContext`, campaign leases, paired swap calls, and the
  post-script deterministic drain are deleted; and
- save/load, replay, rollback, and multiplayer validation pass with unchanged
  hashes except for separately reviewed Original-parity corrections.
