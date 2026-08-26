# Original architecture research for domain isolation

Status: source audit and implementation record for architecture item 4. The
path-scheduling pilot described below is implemented; later slices remain
recommendations.

## Executive finding

The Original is not usefully described as either a clean object-oriented
engine or a collection of independent systems. Its behaviorally important
architecture is a **single ordered transaction loop with object-owned
continuations**:

- the host decides whether an engine frame exists;
- the engine performs mission/script work, advances the frame clock, and may
  stop before the simulation body;
- path completion and collision run at fixed barriers;
- live elements run in Original publication/insertion order, with derived and
  base `Hourglass` work interleaved inside each owner slot;
- the sequence manager drains a FIFO after the element walk, but many
  sequence, actor, AI, messenger, and script transitions execute immediately
  and re-entrantly before that FIFO or during its drain;
- selection cleanup and anonymous timers run after sequences; and
- widgets, rendering/director work, external sound, and one-time script
  post-initialization advance outside the engine tick. Director and sound
  phases can synchronously re-enter gameplay even when the engine tick is
  gated.

The Original's singleton pointers, raw back-pointers, static scratch state,
manual subtype arrays, inheritance hierarchy, and direct UI calls are mostly
C++ implementation accidents. They should not be reproduced as Rust
ownership. Creation identity, insertion/FIFO order, callback-stack closure,
ordinary-lifecycle script handles, timer edge behavior, and the host/tick
boundary are observable and must be retained.

The current nine-owner `EngineInner` is therefore the right **persistent
snapshot root**, not a mistake that should be replaced by independently
ticking services or an ECS. The remaining problem is enforcement: most domain
and subdomain fields are `pub(crate)`, most orchestration methods receive
`&mut EngineInner`, and the now-fixed path context was one example of a
nominally narrow API receiving whole owners. Item 4 should continue introducing
exact, ephemeral capability contexts around proven scheduling barriers. It
should not physically split the deterministic root or alter the tick schedule.

The implemented first slice is the existing path-scheduling barrier. It
replaces the broad `MovementContext { world: &mut WorldState, orders: &mut
OrderRuntime }` with an exact `PathScheduleContext` borrowing only the entity
query, spatial grid, pathfinder, pending/failed path queues, and a read-only
sequence view. Result application, hero speech, sequence termination, and
condolation dispatch remain in the root coordinator. This makes the
architectural rule compile-visible without entering the much riskier
entity/AI/sequence re-entrancy graph.

## Scope and source convention

The audit used the canonical files directly under `original-code/`; the
`original-code/tmprestore/` mirror was not treated as a second source. The
canonical sources contain parity instrumentation added during the port, so
some older documentation points a few hundred lines earlier than the current
files. Anchors below refer to the source tree at the time of this audit.

Principal sources:

| Area | Source anchors | What they establish |
| --- | --- | --- |
| Engine ownership | `original-code/RHEngine.h:145-311`, `RHengine.cpp:328-383` | One object owns simulation, presentation, hardware links, spatial state, sequences, entities, and registries; it also installs global access paths. |
| Engine schedule | `RHengine.cpp:3466-3813` | Exact engine-tick order, early returns, insertion-order element walk, sequence barrier, selection cleanup, and timers. |
| Entity publication | `RHengine.cpp:10285-10369`, `RHengine.cpp:10387-10583` | Canonical append order, derived registries, deactivation-before-removal, and intentionally retained object lifetime. |
| Element owner slots | `RHelement.cpp:28-87`, `RHelementactor.cpp:1034-1232`, `RHelementactorhuman.cpp:461-527`, `RHelementactornpc.cpp:4331-4501`, `RHelementactorsoldier.cpp:2578-2610`, `RHelementactorpc.cpp:1915-1958` | Creation identity, cached service locators, inheritance-call order, inline completion/script callbacks, staggered work, and per-NPC queue order. |
| Sequence scheduling | `RHsequencemanager.h:22-79`, `RHsequencemanager.cpp:64-77`, `RHsequencemanager.cpp:1022-1075`, `RHsequence.cpp:236-315`, `RHsequenceelement.cpp:370-478`, `RHsequenceelement.cpp:622-635`, `RHsequenceelement.cpp:918-960` | Manager ownership, FIFO rules, WAIT and immediate dispatch, owner instruction, and terminal callback ordering. |
| Engine sequence commands | `RHengine.cpp:5073-5265` | One sequence command may synchronously touch user state, camera, UI, timers, script, and the next sequence level. |
| Script bridge | `RHscript.h:30-76`, `RHScript.cpp:50-61`, `RHScript.cpp:340-379`, `RHScript.cpp:885-973`, `Profile/GEngineScript.cpp:31-146` | Static facade, append-based script element table with a compacting physical-removal path, one active sequence builder, and synchronous VM calls. |
| AI | `RHelementactornpc.h:100`, `rhelementactorcivilian.h:15`, `rhelementactorsoldier.h:17`, `RHartificialintelligence.h:927-1133`, `RHartificialintelligence.cpp:126-165`, `RHartificialintelligence.cpp:3529-3595`, `RHartificialmalignity.cpp:501-620`, `RHartificialbonhomie.cpp:140-229` | Per-NPC durable controller state is mixed with global durable state and process-local scratch; hostile/friendly controllers are part of the entity hierarchy; initialization scans the engine's ordered NPC registry. |
| Spatial state | `RHfastfindgrid.h:142-227`, `RHpathfinder.cpp:712-911` | Static topology, mutable overlays, indexes, A* scratch, and path queue/status are mixed in C++; path completion has a distinct scheduling barrier. |
| Host and messages | `RHGame.h:73-177`, `RHgame.cpp:1871-1926`, `RHengine.cpp:4172-4188`, `RHengine.cpp:6945-7086`, `RHsound.cpp:2140-2240`, `RHMessenger.cpp:162-174`, `RHMessenger.cpp:650-774`, `RHengine.cpp:12478-12633` | Host tick gating, post-tick director/render/sound/script order, gameplay callbacks outside the engine tick, subscribed delivery order, and recursive synchronous messages. |

## What the Original actually owns

### `RHEngine` is an aggregate root and a service locator

`RHEngine` inherits the script interface, custom VM dispatch, and messenger
receiver (`RHEngine.h:145`). It holds the messenger, UI/input latches,
hardware/resource pointers, `RHFastFindGrid`, sequence manager, pathfinder,
sound, hiking guide, markers, camera state, the canonical element array, typed
actor arrays, and player selection in one class (`RHEngine.h:158-311`). Its
constructor resets process-global counters and installs itself into
`RHScript` (`RHengine.cpp:338-344`), while every element constructor caches
several engine and process singletons (`RHelement.cpp:56-60`).

Two different facts are hidden in that layout:

1. There must be one authoritative deterministic mission root so a frame,
   save, or rollback snapshot cannot combine state from different moments.
2. Every object does **not** need ambient access to every member of that root.

Rust should retain the first and reject the second. `EngineInner` remains the
atomic serialized/hash root. A phase or command handler receives only the
temporary borrows it actually needs.

### The element array is an ordered world journal

When requested by its `bAddToElements` argument, `AddElement` appends first to
`marrayElements`; it may then publish the object to the script table and append
it to subtype indexes (`RHengine.cpp:10285-10369`). Normal removal first marks
the element inactive and often returns without removing or destroying it
because other objects may still refer to it (`RHengine.cpp:10387-10395`).
Physical removal attempts to repair the parallel registries manually
(`RHengine.cpp:10397-10583`), but the non-background FX branch demonstrates
that this repair is not complete for every subtype.

The manual parallel-array maintenance is incidental. These invariants are not:

- canonical element order is insertion order;
- subtype registries have their own stable insertion order;
- script-visible indices remain stable under the ordinary deactivate/replace
  lifecycle; physical `RHScript::RemoveElement` compacts its array without
  repairing shifted elements' cached indices (`RHScript.cpp:355-364`,
  `sblibng/SBArray.cpp:317-324`);
- deactivation and destruction are different lifecycle events; and
- constructor identity may be consumed before publication.

The engine loop tests `marrayElements.Size()` on every iteration
(`RHengine.cpp:3740`), rather than snapshotting the length. An element appended
after the current slot can therefore receive its first `Hourglass` in the same
frame. The Rust world owner must keep this explicit; a system that snapshots
all IDs at phase entry would be a behavior change.

### Object inheritance is incidental; owner-local call order is not

`RHElement` stores creation identity and sprite/activity state, but also raw
pointers to the sequence manager, draw manager, fast grid, frame holder, and
pathfinder (`RHElement.h:166-192`). Those cached service locators should
disappear in Rust. The virtual `Hourglass` contract (`RHElement.h:290-292`) and
the order in which derived classes invoke it are observable.

For example:

- `RHElementActor::Hourglass` applies delayed positions, resolves the current
  order, executes it, handles wait/lift completion, runs line crossings,
  transitions sequence state, and invokes `ActionChange` inline
  (`RHelementactor.cpp:1042-1227`).
- `RHElementActorHuman::Hourglass` heals concussion and processes shots before
  the Actor call, then refreshes noise and conditionally updates tiredness
  afterward (`RHelementactorhuman.cpp:463-527`). Its tiredness phase is keyed
  by both the universal frame and Original creation order
  (`RHelementactorhuman.cpp:491`).
- `RHElementActorNPC::Hourglass` refreshes patrol, then calls Human, then
  performs friend notification, view/detection, busy/ladder maintenance, and
  finally its 16-frame AI, timers, macro, emoticon, and queued stimuli in
  order (`RHelementactornpc.cpp:4359-4501`).
- Soldier work runs before the NPC/Human/Actor chain
  (`RHelementactorsoldier.cpp:2580-2610`); PC healing runs after the Human/Actor
  chain (`RHelementactorpc.cpp:1924-1942`).

Replacing this with global `movement -> human -> NPC -> soldier` passes is not
an equivalent representation. Rust may use composition and functions instead
of inheritance, but each owner slot must preserve the proven before/base/after
order and its inline callback barriers.

### `FastFindGrid` combines three unrelated lifetimes

The C++ grid mixes loading scratch (`marrayLayers`, `marrayGridBlocks`), final
level topology (`mpLayers`, `mpGridBlocks`, typed indexes), runtime activation
and repulsion data, and A* scratch (`RHfastfindgrid.h:179-227`). The singleton
and wide public surface are incidental. Three lifetimes should remain distinct
in Rust:

- immutable authored topology, shared by queries and rollback snapshots;
- deterministic mutable spatial overlays; and
- per-query/per-path scratch that is never persisted as ambient global state.

The current Rust split is already directionally correct. `LevelGrid` is the
immutable level-loaded side behind `Arc` (`fast_find_grid.rs:851-928`), while
`FastFindGrid` carries runtime activation arrays, lift state, and sector-type
overlays (`fast_find_grid.rs:1039-1076`). Item 4 should narrow mutation access
to the runtime overlay; it should not recombine these values to imitate the
C++ class.

### The sequence manager is a synchronous scheduler, not an event bus

`RHSequenceManager` owns all sequences plus a FIFO of elements to start
(`RHsequencemanager.h:34-38`). Launch appends a sequence and calls `Launch`
immediately (`RHsequencemanager.cpp:64-77`). A sequence advances one command
level at a time: WAIT-priority elements call `Go` inline, while other elements
register with the manager in sequence order (`RHsequence.cpp:253-289`). The
manager drains by repeatedly removing the first FIFO entry and calling `Go`;
new entries appended during the drain are eligible in the same drain
(`RHsequencemanager.cpp:1027-1040`).

Registration is also a dispatch point. `ExecutedImmediately` invokes owner or
engine commands on the current stack and omits them from the manager FIFO
(`RHsequencemanager.cpp:1057-1069`, `RHsequenceelement.cpp:918-960`). `Go`
likewise calls `owner->Instruct` or the engine directly
(`RHsequenceelement.cpp:622-635`). On termination, `SetState` calls the owner's
condolation handler, then `RHSequence::Ready`, then starts a postponed element,
all before returning (`RHsequenceelement.cpp:451-478`). `Ready` can immediately
register or execute the next command level (`RHsequence.cpp:305-315`).

This is why a generic deferred-command queue is unsafe. Rust can use internal
queues to break borrow recursion, but it must expose and test named barriers
that close the equivalent callback stack before an older action is observed.
The queue is an implementation mechanism, not permission to defer work to the
end of a phase or frame.

### Script natives run inside the same transaction

`RHScript` is a static facade over the engine, a script-visible element array,
`This`, location/string storage, movement scratch, and one active sequence
builder (`RHscript.h:30-61`, `RHScript.cpp:50-61`). `AddElement` assigns the
script index at publication (`RHScript.cpp:340-352`). `Start` opens the global
builder, `Then` advances its command level, and `Thanx` launches the completed
sequence immediately (`RHScript.cpp:885-973`). The engine VM wrapper invokes
`Hourglass`, victory checking, message processing, finalization, and
post-initialization synchronously (`Profile/GEngineScript.cpp:31-146`).

The static storage and global `RHEngine *` are accidental. These semantics are
not:

- actor/script handles and `This` have defined dynamic scope;
- native calls see earlier mutations from the same callback;
- native registration of a sequence takes effect at that call position; and
- a script callback may re-enter sequence, actor, AI, or message behavior.

The current Rust `with_script_session` is a good model: it disjointly borrows
the VM, `ScriptDomains`, and ephemeral `NativeSessionCapabilities`, then drains
ordered effects before returning (`engine/script.rs:1264-1344`). It should be
narrowed by native family only when a real boundary is touched. It must not be
replaced with a stored `&mut EngineInner`, a cloned query mirror, or an
asynchronous VM effect batch.

### AI has a real per-owner/global split hidden by statics

`RHArtificialIntelligence` contains a durable controller for one NPC (`mpMe`,
state/substate, paths, targets, timers, groups, and stimulus queues), but mixes
it with process-global alert counts/state, tactical arrays, frame pointers,
view/debug state, and random-selection scratch
(`RHartificialintelligence.h:938-1115`). The definitions at
`RHartificialintelligence.cpp:126-165` make that global coupling explicit.
`InitAI` obtains the engine singleton and initializes each NPC in the engine's
typed registry order (`RHartificialintelligence.cpp:3529-3583`).

There is not a separate manager-owned AI update in the Original. NPC virtually
inherits the common AI base, while Civilian and Soldier add the friendly
`RHArtificialBonhomie` and hostile `RHArtificialMalignity` branches
(`RHelementactornpc.h:100`, `rhelementactorcivilian.h:15`,
`rhelementactorsoldier.h:17`). Both concrete `Think` implementations wrap work
with common `StartThink`/`EndThink` behavior and can call another NPC's `Think`
synchronously (`RHartificialbonhomie.cpp:140-229`,
`RHartificialmalignity.cpp:501-620`; representative active cross-owner calls
occur at `RHartificialmalignity.cpp:1001` and
`RHartificialmalignity.cpp:1578`). Rust
composition is preferable to this virtual-inheritance diamond, but separating
the controller from its owner must not turn those calls into an unordered
manager broadcast.

The Rust ownership should be:

- durable per-NPC controller data on the NPC entity;
- deterministic AI-wide state in `AiRuntime`;
- authored tactical/topology data in immutable level assets or spatial
  topology;
- query scratch local to the query/call; and
- debug/display scratch outside deterministic AI state unless later gameplay
  observes it.

NPCs cannot safely be evaluated in parallel or from one frame-global world
snapshot. An earlier owner can move, launch a sequence, change another actor,
or invoke a callback that a later owner observes. `Think` and the stimulus FIFO
also have owner-local recursive semantics. Borrow-safe Rust adaptations may
suspend and resume a controller, but the logical owner transaction must close
at the same boundary.

### The host boundary is tangled, but still visible

`RHGame` owns the input, device, engine, sound, draw manager, UI, portraits,
widgets, and mission operation (`RHGame.h:73-177`). It gates
`PerformHourglass` on console, UI transition, pause, and mission-operation
state. Regardless of whether that tick ran, it then simulates widgets,
refreshes the screen, advances external sound, and may call the script VM's
one-time `PostInitialize` (`RHgame.cpp:1871-1926`). Conversely,
`RHEngine::PerformHourglass` directly disables widgets and blinks portraits
(`RHengine.cpp:3507-3519`), and PC methods directly add/remove portrait widgets
(`RHelementactorpc.cpp:1882-1893`).

Those host phases are not presentation-only. `RHEngine::Draw` runs
`PerformDirectorWork` before pixels (`RHengine.cpp:4172-4188`), and director
completion terminates camera sequence elements or forwards zoom messages on
the current stack (`RHengine.cpp:6945-7001`, `RHengine.cpp:7045-7086`).
`RHSound::Hourglass` likewise calls an actor's `SoundIsFinished`
(`RHsound.cpp:2225-2240`); NPC remark completion can immediately call AI
`Think` (`RHelementactornpc.cpp:7469-7507`,
`RHartificialintelligence.cpp:6315-6332`). Retail sound completion uses SDL
wall time, whereas parity instrumentation substitutes the universal frame
clock (`RHsound.cpp:2140-2223`). That is an explicit determinism policy, not
evidence that sound completion is an output-only concern.

`RHMessenger` is similarly over-broad, mixing input latches and ordered
synchronous delivery. Its own comment explicitly calls out recursive
pre/post-send messages (`RHMessenger.cpp:162-166`), and receivers are called in
subscription order (`RHMessenger.cpp:667-669`, `RHMessenger.cpp:772-774`). For
example, engine user-lock handling recursively sends action and unselect
messages on the current call stack (`RHengine.cpp:12513-12527`).

Rust should remove host/UI object borrows from deterministic system contexts.
It must retain the semantic command order, host tick gate, the ordered
post-tick director/sound/script continuation barriers, and deterministic
director/camera/sound facts that gameplay or rollback observes. The boundary
is `resolved commands/external facts -> EngineInner -> ordered semantic side
effects and named host continuation inputs`, not `simulation -> UI widget`.

## Behaviorally observable schedule

The following is a preservation contract, not a request to recreate every C++
function:

| Boundary | Original evidence | Required Rust property |
| --- | --- | --- |
| Host admits a frame | `RHgame.cpp:1871-1879` | Pause/console/transition/load gates decide whether the engine tick runs. Later director, sound, and post-initialization callbacks still need their own gates because the Original may run them when this tick is skipped. |
| Mission scripts precede clock increment | `RHengine.cpp:3604-3648` | The 25-frame script cadence and 3-second/forced victory check observe the pre-increment frame; the frame counter increments even when the later simulation body is locked. |
| Body early return | `RHengine.cpp:3650-3654` | Zoom/engine locks stop later phases without undoing earlier script/clock work. |
| Loss/reinforcement/cleanup/path/collision order | `RHengine.cpp:3657-3726` | Do not move path completion or collision across the entity walk. Registry scans retain their own stored orders. |
| One live mutable-size element walk | `RHengine.cpp:3738-3756` | Owner slots follow Original publication/insertion order; deactivation does not compact the schedule; eligible appended children can run this frame. Creation identity separately affects staggered owner logic. |
| Per-owner virtual chain | Actor/Human/NPC/Soldier/PC anchors above | Composition must preserve before/base/after work and inline callbacks for each owner, rather than batching by component type. |
| Sequence manager follows elements | `RHengine.cpp:3762-3763` | Ordinary registered actions run after owner slots, in FIFO order. Immediate and WAIT work may already have run inline. |
| Tail order | `RHengine.cpp:3765-3810` | Swordfight drag edge, titbits, reverse selected-dead scan, and timers remain after the manager. A timer terminates only at exactly `1`; unsigned `0` wraps to `ULONG_MAX`. The scan captures its length, so a timer registered re-entrantly while another terminates waits for a later frame. |
| Terminal sequence stack | `RHsequenceelement.cpp:451-478` | Owner condolation precedes sequence `Ready`, which precedes postponed start. Immediate successors close at the matching stack/barrier. |
| Script/message recursion | `RHengine.cpp:5243-5259`, `RHMessenger.cpp:162-174` | Earlier synchronous mutations are visible to nested/later work. Event queues require named same-call drains, not eventual delivery. |
| Director/render continuation | `RHgame.cpp:1896-1903`, `RHengine.cpp:4172-4188`, `RHengine.cpp:6945-7086` | Director work precedes drawing and may synchronously complete sequences or send messages, including when the main tick was gated. Rendering details must not become ambient inputs, but director gameplay state and callback order are authoritative. |
| Sound continuation | `RHgame.cpp:1914-1915`, `RHsound.cpp:2140-2240` | Sound completion is an ordered gameplay input that may synchronously enter actor AI. Retail wall-clock timing and parity frame timing are distinct policies and must be modeled explicitly. |
| One-time post-initialization | `RHgame.cpp:1918-1926` | The script VM callback follows widget, render/director, and sound work and may run even when the tick gate was skipped. Its native effects close at this host barrier. |

## Incidental C++ coupling to discard

These mechanisms do not encode a useful Rust boundary:

- `RHEngine::mpEngine`, `RHGame::mpInstance`, `RHFastFindGrid::mpFastFindGrid`,
  `RHSequenceManager::mpInstance`, and element-cached subsystem pointers;
- friendship and multiple inheritance on `RHEngine`;
- raw owner/target/sector pointers as identity;
- manual deletion and hand-maintained subtype arrays as the only way to keep
  indexes consistent;
- a class hierarchy as the representation of owner-local scheduling;
- renderer, sound-device, UI widget, and input objects stored beside
  deterministic state;
- loading scratch and query scratch stored on persistent spatial singletons;
- `RHScript` as one all-static native namespace;
- AI selection arithmetic, view rectangles, debug display values, and
  recursion counters as process-global durable fields; and
- serialization repair lists stored alongside the runtime sequence model.

Some values that happen to be global in C++ **are** observable state: Original
creation IDs, sequence/order identities, the simulation RNG stream, overall
alert counts/state, frame deadlines, and stable registry orders. Classification
must follow readers and schedule, not the `static` keyword.

## Current Rust architecture

### What is already correct

`EngineInner` is an atomic deterministic root over nine owners
(`crates/robin_engine/src/engine/mod.rs:177-221`):

| Owner | Appropriate responsibility |
| --- | --- |
| `MissionDomain` | mission result, campaign, objectives/statistics, briefings |
| `SimulationControl` | frame, RNG, gates, speed, deterministic configuration |
| `AiRuntime` | AI-wide deterministic state and vision configuration |
| `WorldState` | entities and runtime spatial/path/weather/shield state |
| `ScriptDomains` | canonical script-facing world domains |
| `OrderRuntime` | messenger, sequence state, order identity, path and deferred scheduling queues |
| `ScriptRuntime` | SCB VM and script-persistent state |
| `PlayerRuntime` | selection, seats, locks, macros, allied control |
| `FeedbackRuntime` | deterministic sound/director/marker state and ordered host effects |

The top-level `HourglassPhase` enum records ten coarse barriers
(`engine/tick.rs:1855-1868`), and `perform_hourglass_inner` invokes them in one
visible order (`engine/tick.rs:2543-2620`). Sequence completion and immediate
work have explicit condolation/action drains on both sides of the timer scan
(`engine/tick.rs:2595-2616`, `engine/tick.rs:6326-6387`). This is consistent
with the Original scheduling evidence.

Other strong foundations are:

- immutable `LevelGrid` versus mutable `FastFindGrid` runtime overlay;
- stable Original creation identity stored explicitly in `WorldState`
  (`engine/state/world.rs:48-88`);
- one VM owner and per-resume native capability construction;
- command-family contexts such as `PositionAssertionContext`,
  `LiftWaitCommandContext`, and `NpcAttentionCommandContext`
  (`engine/sequence_runtime/mod.rs:208-340`, `engine/sequence_runtime/mod.rs:1585-1730`);
- path result application kept outside the nominal path context so
  cross-domain effects occur at explicit coordinator barriers; and
- deterministic feedback separated from GPU/device ownership.

### Where ownership is still conventional rather than enforced

Every `EngineInner` owner is `pub(crate)`, and most fields inside
`WorldState`, `OrderRuntime`, `MissionDomain`, `AiRuntime`, `PlayerRuntime`, and
`FeedbackRuntime` are also `pub(crate)` (`engine/state/*.rs`). An arbitrary
engine helper can therefore bypass domain invariants. Grouping improved
serialization and comprehension, but it did not yet make the dependency map a
compiler-checked API.

Before the pilot, the clearest issue was `MovementContext`: its documentation
said it could not reach scripts, campaign, player state, or feedback, but it
received whole `&mut WorldState` and `&mut OrderRuntime` values and could
technically mutate weather, shield, mobile elements, messenger messages,
reinforcement queues, timers, or the sequence manager. The implemented
`PathScheduleContext` now receives only the exact leaves listed below
(`engine/movement.rs`).

The root path phase already has the correct architectural split:

1. the path context completes the entire pathfinder scheduling operation,
   including starting the successor before returning a completed head as the
   Original does;
2. only after that scheduler boundary closes, `EngineInner` applies the result
   and its ordered cross-domain effects;
3. failure timeout speech precedes `element_impossible` and the owner
   condolation barrier; and
4. collision follows path work (`engine/tick.rs:3627-3890`).

The pilot now makes that split exact. Follow-on isolation must not move the
cross-domain consequences into a larger context.

## Proposed dependency and ownership map

```text
Host/frontend
  resolved PlayerCommand + snapshotted external facts
                         |
                         v
             FrameCoordinator (ephemeral)
             owns phase order, no durable state
                         |
        constructs exact per-phase capabilities
                         |
  +----------------------+------------------------------+
  |                      |                              |
  v                      v                              v
SimulationControl   MissionDomain                 PlayerRuntime
clock/RNG/gates     outcome/campaign              selection/input modes

  +----------------------+------------------------------+
  |                      |                              |
  v                      v                              v
WorldState           OrderRuntime                  ScriptRuntime
  EntityStore          sequences/FIFOs/timers        VM/persistent script
  SpatialRuntime       named barrier queues
      |                     ^                              |
      v                     | synchronous ports           v
Arc<LevelGrid>         owner command handlers      ScriptDomains
immutable topology                                 canonical native domains

                         |
                         v
                    AiRuntime
            global durable AI state; per-owner AI remains on entities

                         |
                         v
                  FeedbackRuntime
       deterministic director/sound/markers + ordered SideEffects
                         |
                         v
                 Host/frontend outputs

LevelAssets is immutable input borrowed by the coordinator and focused
systems. Query scratch and contexts are stack-local and are never serialized.
```

Dependency rules:

1. `EngineInner` owns and snapshots domains; it does not expose a general
   mutable service-locator API.
2. The frame coordinator may split the root into disjoint leaves, but contains
   orchestration rather than domain algorithms.
3. A focused context lists exact leaf borrows. `&mut WorldState`, `&mut
   OrderRuntime`, or `&mut EngineInner` is appropriate only when the operation
   genuinely owns the entire aggregate, which normal tick systems do not.
4. Cross-domain changes return typed outcomes to the coordinator or call a
   narrowly typed synchronous port. Whether the call is immediate or queued is
   part of that port's contract.
5. A context is ephemeral. It is not serialized, cached on an entity, stored
   in a manager, or carried across a VM yield.
6. Immutable topology enters as `&LevelAssets`, `&LevelGrid`, or a narrower
   query. Runtime spatial mutation enters through an explicit overlay/editor
   capability.
7. Domain fields become private incrementally, when the corresponding focused
   API exists. A one-shot privacy conversion would create giant escape-hatch
   accessors and obscure scheduling changes.

## What narrow contexts should look like

The type should tell a reviewer what a system can observe or mutate. For the
recommended pilot, the target shape is conceptually:

```rust
struct PathScheduleContext<'a> {
    frame_counter: u32,
    entities: &'a Entities,
    fast_grid: &'a FastFindGrid,
    pathfinder: &'a mut PathFinder,
    pending: &'a mut PendingPathRequestQueue,
    failed: &'a mut Vec<FailedPathRequest>,
    sequences: &'a SequenceManager,
}
```

The exact implementation can replace raw collection borrows with small domain
methods, but it should not broaden this list. In particular, the path scheduler
does not need mutable entities, mutable sequence state, messenger access,
players, AI, scripts, campaign, timers, feedback, or host display state.

Later high-risk owner scheduling needs capabilities with semantic names, not a
single field bag. A future owner-slot executor may need concepts such as:

- `OwnerEditor`, restricted to the current slot and publication lifecycle;
- `WorldQuery`, allowing ordered target and spatial queries;
- `SequencePort`, whose methods state whether they close condolation/`Ready`
  synchronously;
- `ScriptCallbackPort`, preserving `This` and nested call scope;
- `AiGlobalPort`, separating durable AI-wide state from query scratch; and
- `FeedbackSink`, producing deterministic semantic feedback without exposing
  host widgets or devices.

That later context will legitimately be larger than the path context. It must
still expose operations rather than every domain field, and it must be
designed around the live Original owner slot. Merely renaming `&mut
EngineInner` to `OwnerContext` provides no isolation.

## Implemented first slice

### Path-scheduling capability isolation

The first item 4 change is scoped to `engine/movement.rs`, the path portion of
`engine/tick.rs`, and the minimal state-owner APIs required to split those
borrows.

1. `MovementContext` is renamed to `PathScheduleContext`; it does not represent
   general actor movement.
2. Its two aggregate borrows are replaced by the exact leaf borrows shown
   above.
3. `WorldState` and `OrderRuntime` expose split-borrow methods only where
   needed. The pending/failed fields are narrowed to `pub(in crate::engine)`;
   making them fully private remains follow-on work because legitimate engine
   cancellation, posture, swordfight, parity, and teardown sites still mutate
   them directly. The serialized field layout is unchanged.
4. Keep `apply_completed_path_work`, hero speech, mutation of the sequence
   command, `element_impossible`, owner condolation dispatch, and collision in
   the frame coordinator at their present relative positions. They are
   cross-domain consequences, not path calculation.
5. Preserve the queue's one-result observation rule, retained-cancellation
   behavior, synchronous-mode recursion, stable queue order, and 100-frame
   failure deadline. Original path status/queue behavior is visible in
   `RHpathfinder.cpp:724-910`; the engine calls it before collision and elements
   at `RHengine.cpp:3720-3740`.
6. Retain or strengthen focused tests for stale completions, one completion per
   barrier, synchronous successor parking, repeated-owner requests, timeout
   ordering, and the `hero speech -> impossible -> condolation` boundary. Add
   an ordered trace assertion if no existing test covers that complete timeout
   sequence.

Why this first:

- it is a real scheduling seam in the Original rather than a new abstraction;
- the current code already separates local path work from cross-domain
  consequences;
- exact leaf borrows materially reduce authority;
- persistent layout, hashes, RNG order, entity order, and callback order need
  not change; and
- it establishes the context/privacy pattern before applying it to systems
  whose borrow workarounds encode re-entrant behavior.

One policy difference must remain explicit. Retail asynchronous C++ can
reprioritize slow path requests using the camera-derived screen box
(`RHpathfinder.cpp:748-806`, `RHpathfinder.cpp:834-889`). The parity-instrumented
Original already disables that sort during deterministic capture, and Rust
uses stable queue order so a local viewport cannot change authoritative state.
Do not add `HostDisplayState` or a viewport borrow to `PathScheduleContext`.
Reintroducing retail viewport-dependent scheduling would require a separate,
explicit policy with the relevant fact recorded/snapshotted; it is not domain
isolation.

### Follow-on order, after the pilot

1. Isolate anonymous timer scanning behind an exact timer/sequence context,
   while retaining the two condolation/immediate drains around it and the
   `remaining == 1` rule (`engine/tick.rs:6326-6387`). Preserve the Original's
   captured scan length: a timer registered re-entrantly by a terminating
   timer is not decremented in the same scan.
2. Continue command-family sequence contexts and privatize the fields each one
   no longer needs to expose. Preserve `SequencePhase` FIFO and depth-first
   action barriers.
3. Narrow AI query/effect contexts when parity work touches a specific owner
   boundary. Do not attempt a global AI service extraction.
4. Apply world-store privacy as its own reviewed slice. It must preserve
   canonical insertion/creation order, stable handles, deactivation semantics,
   and derived-registry invariants; it should not be smuggled into the path
   pilot.
5. Remove remaining `HostDisplayState` borrows from deterministic command
   contexts by producing/consuming semantic presentation state, coordinated
   with the dedicated host-boundary work rather than mixed into item 4.
6. Model the post-tick director/render, sound-completion, and one-time
   `PostInitialize` callbacks as named host continuation barriers. Preserve
   their order and their ability to run when the engine tick is gated; record
   the chosen sound-timing fact explicitly instead of reading ambient wall
   time inside deterministic replay.

## Explicit anti-recommendations

Do **not**:

- split the nine `EngineInner` owners into independently snapshotted/ticking
  services;
- introduce a mega `TickContext`, `GameContext`, or `OwnerContext` containing
  all nine owners;
- hide unrestricted `&mut EngineInner` behind traits or closures and call that
  domain privacy;
- convert the live owner walk into global ECS-style component passes or
  parallel NPC jobs;
- snapshot the entity IDs at phase entry unless Original evidence proves that
  newly appended elements must wait;
- replace ordered registries/FIFOs with hash iteration, sorting, or rebuilt
  indexes whose first-match order can differ;
- make all sequence, messenger, script, or AI effects eventually consistent;
  named same-call barriers are required;
- move condolation, `Ready`, postponed starts, immediate commands, WAIT
  commands, or anonymous timers merely to satisfy the borrow checker;
- fold immutable `LevelGrid` back into mutable rollback state or allow routine
  systems to mutate authored topology;
- store contexts, borrows, VM call frames, or query scratch in snapshots;
- clone domains to obtain simultaneous read/write access, then reconcile them;
- create mirrors or cached query models of canonical entity, campaign, AI,
  script, selection, or spatial state;
- move deterministic director camera or sound-simulation state to the host
  solely because the Original mixed it with devices; classify by readers;
- copy Original singleton/global boundaries into Rust module statics; or
- combine domain privacy with schema changes, owner-schedule changes, and
  broad module moves in one review.

## Review checklist for each item 4 slice

A domain-isolation change is complete only when:

- the Original scheduling barrier is cited;
- the context's fields are exact leaf borrows or semantic ports;
- the context cannot reach unrelated domains through a nested aggregate;
- persistent state still has one canonical owner;
- all immediate versus deferred effects are named and tested;
- entity, registry, FIFO, callback, RNG, and timer order is unchanged or a
  deliberate parity correction is documented;
- missing required state still fails contextually rather than fabricating a
  default; and
- snapshot/hash layout changes, if any, are isolated and explicitly versioned.

The architectural goal is not the maximum number of structs. It is that a
reader—and the Rust compiler—can see which state a behavior is allowed to
touch, while the Original's observable transaction order remains intact.
