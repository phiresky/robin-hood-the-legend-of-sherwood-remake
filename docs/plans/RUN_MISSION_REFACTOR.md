# `run_mission` ownership and lifecycle refactor

## Decision summary

Refactor the mission loop around composition, not around one larger `impl`.
The target is:

```text
run_mission / run_mission_headless              thin construction + run wrappers
  InteractiveMission / HeadlessMission          frontend-specific orchestration
    MissionRuntime                              one loaded mission's common state
      MissionWorld                              Host, Game, EngineManager, assets, DevState
      TimelineRuntime                           replay, rewind, rollback, MP clock/history
      MissionControl                            pause/step and mission-local control state
    MissionFrame                                one iteration's commands and decisions
    InteractiveFrontend                         GPU/input/audio/UI resources only
```

`MissionRuntime` is the common owner, but it must not become a replacement
4,000-line method namespace. Timeline operations stay on `TimelineRuntime`, UI
operations stay on focused interactive components, and the two frontends retain
their own short orchestration methods. The graphical and headless loops should
share deterministic frame operations while keeping genuinely different UI and
exit policies explicit.

This is primarily an ownership refactor. It must not reorder simulation,
change when commands are recorded, make a blocking modal asynchronous relative
to the tick, or move `PostInitialize` into `perform_hourglass`.

## Why this is worth doing

Before this refactor, `crates/robin_rs/src/game_session/mod.rs` was 4,346 lines
and `run_mission` owned almost every live mission facility as a separate local.
The landed implementation leaves that module as the session boundary and short
build/run/finish wrappers; bootstrap, runtime, interactive frontend, frame
preparation/simulation, and finishing now have separate owners. The refactor
addressed three practical costs:

- an early return had to coordinate campaign return, queued app effects, save
  flushing, recorder completion, and sometimes modal teardown;
- the only reliable description of frame order is the physical order of one
  very long function;
- headless execution independently reimplements part of the same timeline and
  has silently different operation, modal, and exit behavior.

The common seam is now explicit: `MissionRuntime` owns `MissionWorld`,
`TimelineRuntime`, and `MissionControl`; `InteractiveFrontend` separately owns
renderer/input/audio/UI resources. The timeline asserts the coarse
Input -> Simulation -> Bookkeeping -> Presentation progression.

## Landed ownership inventory

The important distinction is lifetime, not whether a value happens to be used
by rendering.

| Lifetime / authority | Values | Landed owner |
| --- | --- | --- |
| Application/session, borrowed for a call | `GameWindow`, `RustCallbacks`, profile manager, CLI/application options | `MissionServices<'_>` passed to methods; never stored in the loaded mission |
| Between missions | the one concrete `Campaign` | caller before/after the mission; moved into and returned by the mission ownership API |
| Common loaded world | `Host`, `Game`, `EngineManager`, `Arc<LevelAssets>`, `DevState` | `MissionWorld` inside `MissionRuntime` |
| Timeline and transport policy | recorder/player, rewind, rollback checker, peer hashes, history, start gates, MP scheduling, frame clock | `TimelineRuntime` inside `MissionRuntime` |
| Mission control | `manual_pause`, step-key repeat deadlines, replay-finished latch, current ambience shadow key | `MissionControl`; input repeat timers may later move into `MissionInput` |
| Load-only scratch | loading renderer, predecoded background/minimap, raw `Engine`, seed, extracted sim sprite metadata | ordered load stages plus `MissionBootstrap`, consumed to construct a loaded runtime/frontend |
| Mission text/resource data | `text_res`, `cursor_res`, level descriptors, pre-resolved short briefing strings, HUD fonts | `MissionResources` in the interactive frontend; headless retains only data required by sim/modal policy |
| GPU presentation | `Renderer`, cursor/selection/titbit/mouse-trail renderers, portrait cache | `MissionPresentation` |
| HUD layout and hover state | Sherwood/zoom/corner/stature sprites, layouts, enable masks, all tooltip trackers, last cursor id | `MissionHud` inside `MissionPresentation` |
| Input | `ThreadedInput`, `InputTranslator`, per-frame event vector and modifier state | mission-lifetime `MissionInput`; events/modifiers in `MissionFrame` |
| Modal/menu UI | pause menu, active modal, campaign-map overlay state, console overlay, menu resources | `MissionUi` |
| Audio | optional backend, sample loader, sound RNG | `MissionAudio`; `Host::sound` remains host state |
| One frame | start time, `FrameCommands`, recorder hash, replay/modal dismissals | `MissionFrame`, finalized at frame end; presentation flags live in `PreparedFrame`/`FramePresentationState` |
| Save/profile callback state | save manager and pending save/load/app effects on `RustCallbacks` | remain session-owned; borrowed only by operation/save methods |

`RenderContext<'_>` should remain a short-lived view. Once its referenced values
are grouped, `MissionPresentation::render_context(...)` (or a closure-based
`with_render_context`) can create it from disjoint field borrows. It should not
be stored because that would make the frontend self-referential.

## Lifecycle contract to preserve

### Original game

`original-code/RHgame.cpp` establishes the relevant contract:

1. `RHGame::GameLoop` initializes input/UI, calls `mpEngine->Initialize()`,
   handles the lost-Sherwood gate, starts play-time recording, and writes the
   restart snapshot (roughly lines 1399--1480).
2. Each iteration processes input and UI, menu/campaign-map work, one-shot sound
   activation, and the current game operation (roughly lines 1500--1800).
3. If allowed, it calls `RHEngine::PerformHourglass`.
4. It simulates persistent widgets and removes delayed portraits.
5. `Refresh(true, true)` draws and flips the frame.
6. It restores mission sound after a menu, then calls `RHSound::Hourglass`.
7. Exactly once, after that first refresh and sound boundary, it marks
   `mbPostInitialized` and calls script `PostInitialize` (lines 1835--1841).
8. It performs capture/pacing and resets transient messenger state.

`mbPostInitialized` is serialized in `RHGame::Serialize`
(`original-code/RHgame.cpp` around line 2473). The Rust port stores the flag
with mission script state and deliberately dispatches it from
`sim_timeline::run_post_initialize_stage`, outside the engine tick.

### Current graphical path

The graphical path has additional deterministic-host adaptations, but its
effective phase order is:

```text
top-of-frame MP drain and snapshot begin
  -> Sherwood overlay + window/live input + pause/menu handling
  -> operation processing, thumbnail, save/load flush
  -> replay and final pre-tick MP command injection/hash sample
  -> engine tick
  -> HTTP drain, timeline commit, stepping
  -> pending script UI/modals and terminal debriefing
  -> recorder end-frame, queued app effects, sound hourglass
  -> render/present and post-render cleanup
  -> one-shot PostInitialize
  -> MP state-hash send and pacing
```

This order, including the two network drains and the delayed recorder
`end_frame`, is behavioral. A refactor may name the boundaries but may not
coalesce them casually.

### Current headless path

`run_mission_headless` shares setup, replay initialization, tick, HTTP stepping,
timeline commit, `PostInitialize`, and pacing, but it is not simply the
graphical loop without draws:

- it does not run the full live-input, save/load, game-operation, sound, or
  final-debriefing flows;
- it auto-dismisses host modal queues and consumes recorded modal dismissals
  differently;
- it treats replay completion as `GameCode::Quit`;
- it intentionally does not wait at the multiplayer start barrier;
- it runs the post-initialize stage before committing frame zero.

These differences must first be captured as policy/tests. Do not make the
paths textually identical by routing headless through fake renderer, input, or
audio objects. The target shares a common deterministic kernel and gives
`HeadlessMission` an explicit `HeadlessPolicy` for auto-dismiss, replay-complete
exit, pacing, and multiplayer support. Unsupported multiplayer start-barrier
behavior remains an explicit error/TODO, not `false` hidden in a constructor.

## Target types and boundaries

The names are proposals; the ownership boundaries are the contract.

```rust
struct MissionServices<'a> {
    window: Option<&'a mut GameWindow>,
    callbacks: &'a mut RustCallbacks,
    profiles: &'a ProfileManager,
    args: &'a CliArgs,
}

struct MissionWorld {
    host: Host,
    game: Game,
    manager: EngineManager,
    assets: Arc<LevelAssets>,
    dev: DevState,
}

struct MissionControl {
    manual_pause: bool,
    step_forward_repeat_at_ms: Option<u32>,
    step_back_repeat_at_ms: Option<u32>,
    last_shadow_color: u16,
}

struct MissionRuntime {
    world: MissionWorld,
    timeline: TimelineRuntime,
    control: MissionControl,
}

struct MissionFrame {
    started_at_ms: u32,
    commands: FrameCommands,
    recorder_hash: Option<u64>,
    replay_modal_dismissals: VecDeque<PlayerCommand>,
    modal_commands: Vec<PlayerCommand>,
    flags: FrameFlags,
    tick_exit: Option<GameCode>,
}

enum FrameControl {
    Continue { sleep_ms: u32 },
    RestartIteration,
    Exit(MissionExit),
}
```

`MissionFrame` prevents half of a frame's state from leaking into the next
iteration and makes recorder ownership obvious: begin-recording happens while
constructing/preparing the frame, and `finish_recording(self, ...)` consumes
the per-frame dismissal queue exactly once.

The common runtime should expose narrow operations, for example:

```rust
impl MissionRuntime {
    fn begin_frame(&mut self, now_ms: u32) -> MissionFrame;
    fn apply_due_network_inputs(&mut self, frame: &mut MissionFrame) -> NetStatus;
    fn process_operation(
        &mut self,
        frame: &mut MissionFrame,
        callbacks: &mut RustCallbacks,
        profiles: &ProfileManager,
        thumbnail: Option<Thumbnail>,
    ) -> Option<MissionExit>;
    fn run_tick(&mut self, frame: &mut MissionFrame, paused: bool);
    fn commit_tick(&mut self, frame: &mut MissionFrame, paused: bool);
    fn run_post_initialize(&mut self) -> bool;
}
```

Where an operation is really timeline work, the implementation belongs on
`TimelineRuntime` and accepts `&mut MissionWorld`; the root method is at most a
delegating façade. Similarly, MP drain/hash/schedule work belongs on a
`NetworkTimeline` substructure rather than adding more public fields to the
root.

The graphical-only side should be composition as well:

```rust
struct InteractiveMission {
    runtime: MissionRuntime,
    frontend: InteractiveFrontend,
}

struct InteractiveFrontend {
    input: MissionInput,
    ui: MissionUi,
    presentation: MissionPresentation,
    audio: MissionAudio,
    resources: MissionResources,
}

impl InteractiveMission {
    async fn run_frame(
        &mut self,
        services: &mut MissionServices<'_>,
    ) -> Result<FrameControl, String>;
}

struct HeadlessMission {
    runtime: MissionRuntime,
    policy: HeadlessPolicy,
}
```

`InteractiveMission::run_frame` should read as a short ordered list of phase
calls. The implementations of `collect_input`, `drive_modals`, `tick_audio`,
and `present` live on the owning components or in their existing focused
modules. `HeadlessMission::run_frame` is separately readable and calls the same
runtime methods at the same deterministic boundaries.

### Borrowing and async constraints

- Do not store `&mut GameWindow`, `&mut RustCallbacks`, profile references, or
  CLI references on `MissionRuntime`. Pass a short-lived `MissionServices` into
  the async frontend method. This keeps app/session services outside mission
  ownership and avoids a borrow lasting beyond mission teardown.
- `Renderer` clones the window's shared GPU/surface handles and has no lifetime
  parameter, so it can be owned by `MissionPresentation`. Modal methods can
  borrow `window` only for the duration of an `.await`.
- Never keep `RenderContext` or another struct borrowing sibling fields across
  an `.await`. Construct it after async modal/input work and drop it before the
  next await.
- Prefer destructuring disjoint component fields over `RefCell`, raw pointers,
  or moving `Host::engine_display` more widely. The existing `mem::take` around
  engine tick/script calls is a known transitional seam that the Engine/GameHost
  plans may remove.
- Do not introduce an async trait merely to unify the two frontends. Two small
  concrete `run_frame` methods are clearer and avoid boxed futures and fake
  capabilities.

### Campaign and teardown

Every controlled exit converges on one ownership boundary. `run_mission` and
`run_mission_headless` retain the constructed mission and call one consuming
`finish` after the run loop returns; frame helpers return `MissionExit` rather
than owning campaign teardown.

The landed API moves the concrete campaign into a mission and returns that
same value from consuming finalization:

```rust
struct MissionOutcome {
    campaign: Campaign,
    result: Result<GameCode, String>,
}
```

The outer session and main-entry call sites also pass campaign ownership by
value. There is no restore lease, placeholder campaign, or take/install pair.
Dropping a cancelled async mission future drops the one owned campaign in
place; controlled cancellation/window-close paths return it in
`MissionOutcome`.

## Staged implementation plan

Each stage should compile and be reviewable independently. Mechanical moves and
behavior changes belong in separate commits/PRs.

Current status: PRs 1--6 have landed. PR7's structural cleanup has reduced
`game_session/mod.rs` to the session boundary and wrappers; further reduction
of specialized modal/input helper signatures is optional cleanup, not an
ownership or lifecycle blocker. The manual/replay validation matrix below
remains the release-level validation checklist.

### PR 1: Characterize the two frame contracts

Status: implemented. Typed graphical/headless traces cover early restart,
pause/rewind, terminal exit, recorder finalization, and the distinct
`PostInitialize` boundaries.

Files:

- `crates/robin_rs/src/game_session/runtime.rs`
- `crates/robin_rs/src/game_session/mod.rs`
- new `crates/robin_rs/src/game_session/tests.rs` if the test table no longer
  fits the inline test module

Work:

- Expand the test-only phase model beyond the current four coarse stages. Use
  typed events such as `Begin`, `Operation`, `PreTickCommands`, `Tick`,
  `TimelineCommit`, `ModalDrain`, `RecorderCommit`, `Audio`, `Present`,
  `PostInitialize`, and `Pace`.
- Record/verify the graphical and headless phase traces without moving work.
- Characterize early-continue points (Sherwood overlay, pause/menu handlers),
  paused/rewind frames, replay completion, and terminal tick exits.
- Add an explicit `HeadlessPolicy` rather than the unexplained `false` passed as
  `wait_for_multiplayer_start`.

Tests/invariants:

- `PostInitialize` follows first graphical present+audio, is one-shot, and
  remains outside `perform_hourglass`.
- Headless frame-zero post-initialize and recorder-commit ordering is locked to
  current behavior until a parity decision changes it deliberately.
- recorder begin/end, rewind snapshot, rollback check, and `sim_frame += 1`
  all refer to the same pre-tick frame.
- the second MP drain remains before the hash/tick boundary.

### PR 2: Establish common owning state, with no phase moves

Status: implemented. `MissionWorld`, `MissionRuntime`, `MissionControl`,
`MissionFrame`, and `TimelineRuntime` own the common mission/timeline state.

Files:

- rename current timeline implementation to
  `crates/robin_rs/src/game_session/timeline.rs`
- new `crates/robin_rs/src/game_session/runtime.rs`
- new `crates/robin_rs/src/game_session/frame.rs`
- `crates/robin_rs/src/game_session/mod.rs`
- `crates/robin_rs/src/game_session/tick.rs`
- `crates/robin_rs/src/game_session/multiplayer.rs`

Work:

- Rename the current `MissionRuntime` to `TimelineRuntime`.
- Introduce `MissionWorld`, the root `MissionRuntime`, `MissionControl`, and
  `MissionFrame` with private fields and narrow accessors.
- Move existing locals into those structs at their present initialization
  points. Keep the loop body in `mod.rs` temporarily so the diff proves this is
  ownership-only.
- Move MP telemetry/maps/scheduling under the timeline component; stop reaching
  into every field directly from `mod.rs`.
- Make `MissionFrame` own frame command/dismissal/hash/flag state.

Tests/invariants:

- existing replay, rewind, rollback, and multiplayer tests pass unchanged;
- compile-time ownership prevents two live frame objects for one runtime;
- paused/rewind frames do not advance the timeline or end a recorder frame;
- no new serde derives/default fallbacks are added to process resources.

### PR 3: Own the interactive frontend

Status: implemented. `InteractiveFrontend` composes focused input, UI, audio,
resources, HUD, and presentation owners without storing borrowed session
services.

Files:

- new `crates/robin_rs/src/game_session/interactive.rs`
- new `crates/robin_rs/src/game_session/presentation.rs`
- new `crates/robin_rs/src/game_session/ui_runtime.rs`
- `crates/robin_rs/src/game_session/render.rs`
- `crates/robin_rs/src/game_session/input_handlers.rs`
- `crates/robin_rs/src/game_session/mouse_input.rs`
- `crates/robin_rs/src/game_session/modal_state.rs`
- `crates/robin_rs/src/game_session/mod.rs`

Work:

- Introduce `MissionInput`, `MissionUi`, `MissionAudio`, `MissionResources`,
  `MissionHud`, and `MissionPresentation` and move the corresponding locals
  into them.
- Give components focused methods: resolution update/input reset on
  `MissionInput`, pause/active-modal transitions on `MissionUi`, shadow-key
  rebind and sound hourglass on `MissionAudio`/presentation, and render-context
  construction/present on `MissionPresentation`.
- Keep existing large specialized functions where they are useful; change
  their parameter lists to accept the owning component or a short-lived view.
- Keep save/profile callbacks outside the frontend.

Tests/invariants:

- logical resize reconfigures renderer, viewport, input clipping/translator,
  minimap, and all four HUD layouts together;
- closing pause/modal resets input and restores mission sound exactly once;
- screenshot/thumbnail/wide-map passes do not advance tooltip state multiple
  times or mutate deterministic engine state;
- the mission-start map path still captures after `Initialize` and before first
  hourglass/`PostInitialize`.

### PR 4: Make bootstrap produce complete owned missions

Status: implemented. Ordered bootstrap stages produce complete interactive or
headless mission owners and return the campaign on every controlled setup
outcome.

Files:

- new `crates/robin_rs/src/game_session/bootstrap.rs`
- `crates/robin_rs/src/game_session/setup.rs`
- `crates/robin_rs/src/game_session/replay_init.rs`
- `crates/robin_rs/src/game_session/interactive.rs`
- new `crates/robin_rs/src/game_session/headless.rs`
- `crates/robin_rs/src/game_session/mod.rs`

Work:

- Introduce `MissionSpec` and `MissionBootstrap` for the ordered setup stages.
- Return a complete `MissionRuntime` plus either an `InteractiveFrontend` or a
  `HeadlessPolicy`; do not return a large tuple.
- Preserve the loading-screen lifetime: close/drop it before creating the game
  renderer even though the renderer currently uses cloned GPU handles.
- Keep Spellforge startup after SCB `Initialize` and before mission audio/
  replay initialization.
- Stop running graphical predecode/HUD work in true headless construction.
  Retain resource decoding proven necessary for engine setup or explicit
  headless modal policy.
- Route every bootstrap failure after campaign installation through the same
  checked campaign-return path.

Tests/invariants:

- both modes receive identical engine seed, sim metadata, campaign, and replay
  header for the same mission;
- required Lua/SCB startup failure returns an error and the original campaign;
- lost-Sherwood and mission-start-map exits return the same campaign allocation;
- missing required resources remain errors/warnings according to existing
  contracts, never fabricated defaults.

### PR 5: Extract deterministic frame methods and the headless driver

Status: implemented. Interactive preparation/simulation/finish phases and the
true-headless driver share timeline seams while retaining explicit frontend
policy differences.

Files:

- `crates/robin_rs/src/game_session/runtime.rs`
- `crates/robin_rs/src/game_session/timeline.rs`
- `crates/robin_rs/src/game_session/frame.rs`
- `crates/robin_rs/src/game_session/headless.rs`
- `crates/robin_rs/src/game_session/interactive.rs`
- `crates/robin_rs/src/game_session/multiplayer.rs`
- `crates/robin_rs/src/game_session/tick.rs`
- `crates/robin_rs/src/game_session/mod.rs`

Work:

- Move common begin-frame, replay injection, final network drain/hash,
  tick, rollback/rewind commit, HTTP stepping, recorder, post-initialize, and
  pacing operations onto the owning runtime/timeline types.
- Implement the two short `run_frame` orchestrators against those operations.
- Model modal behavior and replay-completion exit as frontend policy, not
  `args.headless` branches inside the interactive driver.
- Keep `Game::process_operation`, save/load flushing, terminal
  `ApplyQuitMissionUpdates`, and interactive debriefing in a named flow phase.
  Decide with tests whether headless intentionally skips each item; do not
  inherit the graphical behavior accidentally.
- Remove the graphical function's internal `args.headless` branches once the
  true headless entry point owns that mode.

Tests/invariants:

- fixed replay produces identical per-frame hashes before and after extraction;
- interactive and headless share pre-tick command/timeline boundaries;
- auto-dismissed headless modals and recorded `ModalDismiss` commands cannot
  perturb the sim command stream;
- pause, HTTP forward/back, keyboard forward/back, rewind hold, replay seek, and
  MP late-input rollback all update `sim_frame` through one tested API.

### PR 6: Centralize operation/save/exit and campaign return

Status: implemented. Mission builders accept `Campaign` by value, all
controlled setup/runtime outcomes return it in `MissionOutcome`, and the outer
session returns it in `SessionOutcome` without a borrowed placeholder slot.

Files:

- new `crates/robin_rs/src/game_session/flow.rs`
- `crates/robin_rs/src/game_session/runtime.rs`
- `crates/robin_rs/src/game_session/interactive.rs`
- `crates/robin_rs/src/game_session/headless.rs`
- `crates/robin_rs/src/game_session/mod.rs`
- `crates/robin_rs/src/game.rs`
- `crates/robin_rs/src/main_entry.rs`
- save/campaign files selected by the coordinated campaign-ownership plan

Work:

- Make handlers return `FrameControl`/`MissionExit`; remove campaign-return
  ownership and direct function returns from nested input/modal helpers.
- Execute pending app effects and saves at named exit barriers.
- Make one `finish` consume a loaded mission and return its campaign and result.
- Change outer callers to ownership-by-value only after the Engine/GameHost
  plan establishes the final campaign/script-access API.
- Leave `run_mission` and `run_mission_headless` as small wrappers that build,
  run, finish, and return.

Tests/invariants:

- campaign identity and contents are preserved for window close, setup error,
  quit, win/loss/interruption, restart, cross-mission load, replay completion,
  mission-start capture, and modal emergency exit;
- quit-time continue save and queued app effects run before host/frontend drop;
- cross-mission load returns its pending request exactly once;
- recorder is finalized once on every exit that began a recordable frame.

### PR 7: Cleanup only after the ownership migration

Status: substantially implemented. `game_session/mod.rs` is the session/index
boundary and obsolete ownership tuples/leases are gone. Large specialized
input/modal functions remain in focused modules and may be simplified in later
non-behavioral cleanups.

Files: primarily `game_session/mod.rs` and obsolete helper modules/imports.

Work:

- Move `run_session` and debriefing-only helpers to focused modules if still
  large.
- Delete transitional tuple types, public fields, `#[allow(too_many_arguments)]`
  attributes made obsolete by the owning components, and `args.headless`
  branches unreachable from the interactive frontend.
- Update `docs/REFACTORING.md` ownership status.

Do not mix this cleanup with behavior fixes discovered by replay validation.

## Validation ladder

For every PR:

1. `cargo fmt`
2. focused tests for `game_session`, `game`, timeline, and modified MP/modal
   components
3. `cargo test -p robin_rs`
4. `cargo build --bin robin`

For PRs 4--6, also run manual/replay validation:

- demo Leicester from a fresh start through the first frame; verify the trace
  ends in render/audio then one-shot `PostInitialize`;
- a replay with dialogues/debriefing, normal speed and fast-forward, comparing
  every recorded hash;
- start-paused, `.`, `,`, Backspace hold, Enter resume, and HTTP
  `/step-forward`/`/step-back`;
- quicksave, quickload in the same mission, cross-mission quickload, restart,
  quit continue-save, and resume;
- Sherwood map open/close, mission selection, lost-ARES gate, and production
  transition;
- two peers through the lobby start barrier, current-frame input, late-input
  rollback, and state-hash sampling;
- headless replay to completion, including a replay containing modal dismissal
  commands;
- Lua custom mission startup where supported, confirming SCB
  `PostInitialize` remains at the host boundary.

Use the newest recorded `~/.local/share/robin_hood/replays/*.rhrec.jsonl` as the
default reproduction artifact and retain a small fixed corpus for merge gates.
Do not update expected hashes solely because a refactor changed them.

## Risks and controls

| Risk | Control |
| --- | --- |
| A method extraction subtly moves a command across the recorder/tick boundary | typed phase trace plus per-frame replay hashes; one phase move per PR |
| A root `MissionRuntime` becomes another god object | private component fields; behavior on the smallest owning component; root only orchestrates/delegates |
| Headless inherits UI assumptions or fake resources | concrete `HeadlessMission`, no dummy renderer/audio/input, explicit policy and unsupported-state errors |
| Borrow-checker pressure leads to `RefCell`/raw pointers | short-lived services/views, disjoint component destructuring, no stored `RenderContext` |
| Async modal calls retain sibling borrows | finish/drop frame/render views before `.await`; modal methods borrow only the fields they need |
| Campaign is lost on an early return | `MissionExit` propagation and one consuming `finish`; final by-value campaign API coordinated with mirror removal |
| `PostInitialize` runs too early or twice | retain `sim_timeline::run_post_initialize_stage`, serialized one-shot flag, graphical/headless trace tests |
| Save/load or rewind replaces Engine but leaves presentation caches stale | named post-load/post-rewind hooks; test input reset, resolution resync, shadow cache and rollback checker invalidation |
| Concurrent refactors conflict in `mod.rs` | land ownership shells before method moves; rebase behavior-sensitive Engine/GameHost changes first and preserve their APIs/invariants |

## Dependencies on the other two planned refactors

### EngineInner logical split

This plan should treat `Engine` as one deterministic kernel through PR 5. It
must not reach into new `EngineInner` substructures from the session layer.
The Engine split should preserve or improve the façade methods used here:
`apply_commands`, `perform_hourglass`, `perform_post_initialize`, campaign
access/return, save/load, display order, and read-only presentation queries.

Likely conflicts are `EngineManager`, snapshot schema, `LevelAssets`
reattachment, and post-load hooks. Merge behavior/schema changes first; then
rebase session ownership without changing phase order.

### GameHost mirror removal

Campaign ownership and script re-entry are the direct dependency. The session
plan must adopt the mirror-removal plan's borrowed native/script context and
must not add another `swap_engine_state`, temporary campaign owner, or mirrored
entity/door state to simplify method signatures.

The safest merge order is:

1. this plan's phase characterization and non-semantic owning shells (PRs 1--3);
2. GameHost borrowed-context/mirror removal and any Engine façade changes;
3. bootstrap/common-frame extraction (PRs 4--5) rebased on that API;
4. by-value campaign finish/teardown (PR 6);
5. EngineInner physical field moves, unless its façade-only preparation can
   land earlier without schema changes.

If the EngineInner and GameHost work establishes a different final owner name,
keep the boundaries above and adapt the names; do not retain duplicate root
types.

## Non-goals

- no ECS conversion;
- no renderer/audio ownership inside deterministic Engine snapshots;
- no generic async frontend trait;
- no single `MissionContext` containing app, renderer, engine, callbacks, and
  every UI object behind public fields;
- no tick, entity, sequence, RNG, or script timing changes;
- no fake default campaign/resources on missing required state;
- no save-format migration except one separately reviewed change required by
  the final ownership model;
- no clippy cleanup mixed into these PRs.

## Definition of done

The refactor is complete when:

- both public mission entry points are short build/run/finish wrappers;
- every mission-lifetime value has one named owner and every frame-lifetime
  value dies with `MissionFrame`;
- graphical/headless deterministic phase order is explicit and tested;
- campaign return and recorder/save/app-effect teardown are centralized;
- the interactive path contains no headless renderer/audio/input shims;
- replay hashes and the manual matrix above match the pre-refactor baseline;
- `game_session/mod.rs` is an index/session boundary rather than the mission
  implementation;
- the owning structs expose focused methods without recreating the original
  long function as one mega-`impl`.
