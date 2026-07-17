# Gameplay Parity Audit

## `PerformHourglass` Phase Ordering

The Rust tick is mechanically decomposed in
`crates/robin_engine/src/engine/tick.rs`. The emitted `HourglassPhase` trace is
the ordering contract for the audited Rust pipeline:

1. `DeferredEffectsStart`
2. `MissionAndMessages`
3. `NpcOrders`
4. `Paths`
5. `Entities`
6. `Sequences`
7. `EntitySystems`
8. `Npcs`
9. `GameplaySystems`
10. `DeferredEffectsEnd`

Early mission and lock exits intentionally produce only the prefix that was
reached. Focused tests lock both the complete order and an early mission exit.

### Original provenance

The shipped C++ spine is in `original-code/RHengine.cpp:3446-3778`:

- Mission/script gates and universal-counter advancement are at lines
  3470-3664.
- `ProcessPathRequests`, then `CheckForCollision`, run at lines 3697-3705.
- `marrayElements` is refreshed at lines 3715-3723.
- `mSequenceManager.Hourglass()` follows at lines 3726-3727.
- Swordfight-edge handling, titbits, selection cleanup, and anonymous timers
  follow the sequence manager at lines 3729-3775.

The original element array is sorted by `GetCreationOrder()` in
`original-code/RHengine.cpp:7909-7944`. The sequence-manager hourglass drains
its pending list FIFO and calls `Go()` at
`original-code/RHsequencemanager.cpp:931-943`.

### Resolved ordering parity

- **Core spine:** Prior-tick path retry now runs before `Entities`; base entity
  and actor-hourglass work completes before `Sequences`; and
  `DeferredEffectsEnd` remains last. This restores the provable portion of the
  original coarse `ProcessPathRequests -> element Hourglass ->
  SequenceManager::Hourglass -> post-process` order without crossing the
  profile-dependent batched-system lifecycle described below.
- **Paths:** Direct Move/Seek paths are constructed synchronously, while
  A*-requiring moves enter a deterministic queue. `Paths` advances that queue
  once per frame, returning at most one completed request and beginning at
  most one successor before `Entities`, matching `RHEngine::ProcessPathRequests`.
  Failed-request deadlines remain in the same phase. Seek refresh and
  WAIT_TIMER maintenance live in `Entities`, matching
  `RHElementActor::Hourglass` provenance. The intervening original
  `CheckForCollision` had only a mobile-damage arm, explicitly marked dead in
  `original-code/RHengine.cpp:10790-10855` because shipped missions never
  return true from `IsMobile`; Rust therefore has no invented replacement.
- **Entities:** `EngineInner::add_entity` appends and removal leaves a hole;
  `Entities::occupied_mut()` walks those slots in ascending order. Focused
  tests prove slots are not reused and serde save/load preserves both holes
  and order. This supplies the Rust equivalent of original creation ordering.
- **Deferred effects:** The original swordfight falling-edge check, titbit
  update, dead-selection scan, and anonymous timers retain their exact order
  at the start of `DeferredEffectsEnd`. Rust-only condolation, re-entrant
  self-stimulus, and immediate-action drains follow them, so they cannot affect
  an original post-process earlier than its source order. `PostInitialize` is
  now a separate host-driven boundary; see fixed finding PA-029.

### Remaining architectural limitations

- **NPCs:** Original NPC AI/detection was reached inside each NPC element's
  creation-ordered Hourglass (for example
  `original-code/RHelementactornpc.cpp:3495-3614`). Rust has both an `NpcOrders`
  pre-pass and a batched `Npcs` pass after sequence launch. Fixing that is an AI
  lifecycle/interleaving change, not a tick-only reorder: the NPC pass requires
  complete profile/brain snapshots, and this slice may neither invent missing
  defaults nor change AI ownership. TODO(original-parity): move NPC work into
  creation-ordered entity refresh only when the AI implementation owns
  complete per-NPC profiles/brains at that boundary.
- **Entity systems:** Movement, animation, projectile, melee, and ability work
  is batched by system in Rust, whereas the original invoked subtype
  hourglasses from the creation-ordered element loop. Base entity refresh is
  now correctly placed, but the profile-dependent batched systems remain after
  sequence translation. Exact intra-entity interleaving requires redesign
  outside this tick-only slice. TODO(original-parity): give these systems
  complete per-entity inputs, then map observable cross-entity interleavings
  with original replays before changing subsystem APIs.

## Review Queue

This document is the review queue for gameplay behavior in the Rust port. The
goal is not line-for-line translation. The goal is that every gameplay effect
can be traced to the original game or is explicitly approved as a post-port
feature.

## Evidence Rules

Use one of these statuses:

- `mismatch`: Rust and original behavior were compared and differ.
- `incomplete`: the Rust code explicitly implements only part of the original.
- `unverified`: suspicious or structurally different, but no behavioral
  difference has been proved yet.
- `verified`: compared with exact original source and covered by a focused test
  or deterministic replay where practical.
- `intentional`: no original equivalent and approved in `docs/NEW_FEATURES.md`.
- `host-only`: tooling, rendering backend, networking, replay, or platform work
  that cannot affect authoritative gameplay state.

A source comment is not evidence by itself. A reviewed item must name the
original file and function. If an intentional difference affects gameplay, add
it to `docs/NEW_FEATURES.md`; do not describe it only as a safeguard, fallback,
or simplification.

## Fixed Findings

| ID | Rust behavior | Original evidence | Resolution |
| --- | --- | --- | --- |
| PA-001 | PCs automatically collected nearby bonuses every tick. | `RHElementObject::BuildTakeSequence`, `RHelementobject.cpp`; `RHElementActorPC::Execute` Taking completion, `RHelementactorpc.cpp`. | Removed in commit `682dc1f49`; collection now requires the Take sequence. |
| PA-002 | Unmapped AI stimuli skipped `FilterAIEvent`; comments incorrectly called original stimuli such as `EVENT_ENEMY_NEAR` Rust-only. | `RHArtificialIntelligence::StartThink`, `RHartificialintelligence.cpp`, assigns event code `-2` and still calls the script filter; enum in `RHartificialintelligence.h`. | Rust now calls `FilterAIEvent(source, -2)` through the normal filtered dispatch path and has a regression test. |
| PA-003 | Default mission loss treated any living, conscious PC as playable and ignored `PcData::playable`. | `RHEngine::PerformHourglass`, `RHengine.cpp`; `RHElementActorPC::IsPlayable`, `RHelementactorpc.h`. | The check now uses the explicit playable flag plus guard state, with a focused test. |
| PA-004 | Mission exits could replace a missing required campaign with `Campaign::default()`. | `RHCampaign`, `RHCampaign.cpp`, installs one concrete singleton; `launcher.cpp` owns that campaign across mission runs. | `c106f35b1` funnels mission exits through `restore_required_campaign`, which panics with boundary context instead of inventing campaign state. |
| PA-005 | Restore repaired malformed parallel fast-grid arrays with all-active values absent from the snapshot. | `RHEngine::Serialize`, `RHengine.cpp`, serializes the concrete engine/grid state; it has no all-active corruption-repair path. | `c5e277769` validates lengths before mutation and returns `SnapshotRestoreError`; the compatibility facade rejects corruption loudly. |
| PA-010 | EnemyNear scanned every PC unconditionally and ran generic trouble/battle decisions. | `RHArtificialMalignity::AttackingReactiontimeEnemyNearTest` and `EVENT_ENEMY_NEAR`, `RHartificialmalignity.cpp`. | `15e5ffd6f` restores the trainer/substate/time gates, ordered `mlistThem` scan, exact box/postures, stimulus target assignment, and `BeginSwordfight`. |
| PA-011 | `reinitialize_them_list` retained an unseen saved primary target. | `RHArtificialMalignity::ReinitializeThemList`, `RHartificialmalignity.cpp`. | `4350e5092` rebuilds the list solely from visible, living enemies. |
| PA-012 | FadeToBlack frozen frames advanced the mission clock. | `RHScript::FadeToBlack`, `RHScript.cpp`. | `e3fb1efb0` makes the fade render-only while simulation, RNG, script, display, and sound timers remain frozen. |
| PA-014 | Rust completed every pending A* path request synchronously in one tick. | `RHEngine::ProcessPathRequests`, `RHengine.cpp`, and `RHPathFinder::ProcessPathRequests`, `RHpathfinder.cpp`, expose one scheduling point and at most one ready result per frame. | `7786df3bf` restores the waiting/in-flight queue, original priority ordering, one-call latency, and one completed request per frame, with focused timing tests. |
| PA-015 | Missing sound/exclamation durations fabricated 75-frame completions. | `RHSound::GetSampleLengthMs`, `RHsound.cpp`, returns zero when the sample cannot be resolved; completion follows the sound hourglass. | Missing duration metadata now warns and schedules the original zero-length result at the next deterministic simulation boundary, with focused completion-order tests. |
| PA-016 | NPC tick phases ran in a materially different order. | `RHElementActorNPC::Hourglass`, `RHelementactornpc.cpp`, orders patrol, base human work, broadcasts/view/detection/ambush, busy/ladder, lock gate, periodic work, timers, then queued stimuli. | The Rust phases now follow that exact order once each, protected by an ordering trace test. Per-entity batching remains tracked by PA-013. |
| PA-020 | Arrow-watching AI treated `EVENT_DONE` as a return-to-duty signal. | Arrow-watching substate cases in `RHartificialmalignity.cpp`. | `a8dcaaf01` removes the invented fallback and adds a focused regression. |
| PA-021 | Script native `Sees` omitted ambiance-adjusted view radius and the forest Royalist 180-degree rule. | `RHScript::Sees`, `RHScript.cpp`, delegates to `RHElementActorNPC::IsDetecting` in `RHelementactornpc.cpp`. | `6a9ec3b11` routes the native through matching NPC visibility rules and adds focused coverage. |
| PA-022 | Cached enemy-door seeking omitted building capacity and rider authorization gates. | `RHArtificialMalignity::FindDoorEnemyCouldBeBehind`, `RHartificialmalignity.cpp`, calls `RHGate::IsActorAutorized` in `RHGate.cpp`. | `0fe1bd919` restores every authorization gate and adds focused occupied-building/rider tests. |
| PA-023 | The audit treated `SetExperiences` campaign persistence as a Rust-only mismatch. | `RHElementActorPC` is constructed with the campaign description's `PCStatus` (`RHelementactorpc.cpp`), and `RHElementActorHuman::SetCapacity` writes through that pointer (`RHelementactorhuman.cpp`). | `2fcdaabaf` documents the shared backing state and tests both live capacity changes and campaign serialization; persistence is Original behavior. |
| PA-024 | `SetPersistentProperty` dropped live PC ammo writes when no campaign existed. | `RHScript::SetPersistentProperty`, `RHScript.cpp`, updates the live PC for arrows and PC ammo properties. | `7029583d6` writes through the live PC capacity independently of campaign persistence and adds no-campaign regressions. |
| PA-026 | Shoulder-ceiling checks ran for every `CarryingOnShoulders` posture. | `RHElementActorPC::Execute`, `RHelementactorpc.cpp`, gates the check on `WALKING_CARRYING_ON_SHOULDERS`. | `93faf0436` adds the action gate and focused regression coverage. |
| PA-027 | Messenger processing, condolations, and self-stimuli were deferred to global tick tails. | `RHMessenger::ForwardMessage` and `RHSequenceElement::SendCondolationCard` dispatch synchronously and re-entrantly (`RHMessenger.cpp`, `RHsequenceelement.cpp`). | `6f7907eaf` restores recursive messenger ordering and synchronous condolence arbitration, protected by same-frame ordering tests. |
| PA-028 | Script `SendMessage` bypassed the original sequence element and dispatched after the script call. | `RHScript::SendMessage`, `RHScript.cpp`, launches `RHCOMMAND_SEND_MESSAGE`; `RHSequenceElementSendMessage::Go`, `RHsequenceelement.cpp`, performs delivery. | `618df2081` restores sequence launch, priority/arbitration, deferred delivery, and exact callback-order tests. |
| PA-029 | Rust ran mission `PostInitialize` at the end of the first engine tick before the original presentation boundary. | `RHGame::GameLoop`, `RHgame.cpp`, calls it after the first refresh and sound hourglass. | `cdcf5d0fe` restores an explicit host-driven post-refresh/sound stage and mirrors it in rollback replay, with exact ordering tests. |
| PA-030 | Collinear movement-line intersection fabricated impact parameter `t = 0.5`. | `SBGeoSegment2D::operator^` returns the overlap segment, and `RHFastFindGrid::IsReachable` treats it as a collision; the first overlap endpoint reached by the movement is authoritative. | `be3e7e987` restores earliest-overlap ordering and adds collinear geometry regressions. |
| PA-035 | A requested replay whose header could not be decoded warned and substituted the multiplayer seed or zero before Engine construction. | Replay is a Rust extension with no Original equivalent; explicit playback is a deterministic contract and cannot invent initial RNG state. | Replay seed resolution now returns a typed, contextual preload error before `Engine::new`; corrupt JSON/header data, invalid compact data, and missing fields are fatal, while valid replay and non-replay multiplayer seeds retain their priority. |

## Open Findings

Priority reflects likely gameplay impact, not implementation effort.

| ID | Priority | Status | Finding and evidence |
| --- | --- | --- | --- |
| PA-013 | High | unverified | Rust globally regroups per-entity Hourglass work into movement, animation, script, detection, combat, and ability passes. Original `RHEngine::PerformHourglass` calls each virtual `Element::Hourglass` in entity order before `RHSequenceManager::Hourglass` (`RHengine.cpp`). Cross-entity and same-frame callback ordering needs scenario tests. |
| PA-025 | Medium | mismatch | Charly-to-officer logic substitutes 360-degree detection. Original calls normal `IsDetecting(mpAntagonist)` and therefore respects the view cone (`RHartificialmalignity.cpp`). |
| PA-031 | Low | mismatch | Push handling falls back to radial movement for unexpected thrust kinds. Original push dispatch handles the three supported kinds and asserts otherwise (`RHelementactorhuman.cpp`). |
| PA-032 | High | mismatch | WAIT-priority sequence elements are appended to `elements_to_go` and do not dispatch until the manager hourglass. Original `RHSequence::Launch` calls `Go()` for WAIT elements before launch returns (`RHsequence.cpp`). Add a launch-return ordering test before changing the barrier. |
| PA-033 | High | mismatch | `SimSnapshot` excludes `HostDisplayState` and replay constructs a default display, although `EngineInner::perform_hourglass_inner` reads zoom-transition flags to decide whether gameplay phases run. Original `RHEngine::Serialize` persists `mbackgroundTransform` (`RHengine.cpp`). Move the gate input into deterministic state or prove it is derived, then test replay from a non-default zoom transition. |
| PA-034 | High | incomplete | Spellforge Lua is a post-original feature, not the original SCB VM. Only Initialize/PostInitialize are dispatched; Lua state is absent from snapshots, required startup failure continues without Lua, event errors are discarded, and Initialize can call the sim-RNG shim without an installed Engine RNG scope (`lua_session.rs`, `game_session/mod.rs`, `robin_lua/state.rs`). Reject it in deterministic modes until a versioned Spellforge contract and state policy exist. |
| PA-036 | Medium | intentional | Rust uses one serialized Engine-owned `fastrand` stream; Original uses the process-global C RNG, seeds production from wall time, and implements script `Rand(max)` as `rand() % max` (`launcher.cpp`, `RHScript.cpp`, `RHartificialintelligence.cpp`). Bit-identical rolls are not the target; every gameplay draw still needs a reviewed range, call-site order, and snapshot test. This intentional architecture is recorded in `NEW_FEATURES.md`. |
| PA-037 | Medium | unverified | Required campaign fallback is fixed, but mission ownership still moves an `Option<Campaign>` between Engine/session boundaries and temporarily swaps it into `MissionScript::game_host`. Original exposes one required `RHCampaign` singleton (`RHCampaign.cpp`, `RHgame.cpp`). Prove identity preservation and panic-safe restoration across every script call and mission exit. |

## Tick Provenance

This is the top-level audit spine. A row marked verified means the phase has a
clear upstream owner; extracted helper internals and same-frame ordering still
need their own review.

| Rust phase | Original owner | Status / next check |
| --- | --- | --- |
| Mission notices, quit branches, script Hourglass and victory | `RHEngine::PerformHourglass`, `RHengine.cpp` | verified structurally |
| Frame increment, lock gate, default loss | `RHEngine::PerformHourglass` | playable mismatch fixed; retain regression |
| Reinforcement countdown | `RHEngine::PerformHourglass`; `RHElementActorPC::IsReinforcementTime` | verify bypassing the messenger has no observers |
| Sequence cleanup and path processing | `RHEngine::PerformHourglass`; `RHEngine::ProcessPathRequests` | frame pacing verified by PA-014 |
| Entity refresh and sequence dispatch | virtual `RHElement::Hourglass`; `RHSequenceManager::Hourglass` | PA-013 |
| Movement, animation, ActionChange, scroll Hourglass | actor/object virtual Hourglass and Execute methods | PA-013; test entity-order observations |
| NPC view, detection, timers, speech, patrol | `RHElementActorNPC::Hourglass` and AI subclasses | phase order verified by PA-016; per-entity batching remains PA-013 |
| Arrows, purse/coins, wasps, nets, melee, abilities | per-type virtual Hourglass/Execute methods | verify spawn-frame inclusion and ordering per type |
| Titbits, deselection, anonymous timers | tail of `RHEngine::PerformHourglass` | structurally verified; titbit display-order approximation is visual |
| Condolations and self-stimuli | `RHSequenceElement::SetState` to actor `SendCondolationCard` | synchronous ordering verified by PA-027 |
| PostInitialize | mission loop in `RHgame.cpp` | host boundary verified by PA-029 |
| RNG, rollback side effects, minimap/marks/camera | no single original phase | intentional architecture only where documented; audit gameplay state individually |

## Coverage Matrix

`queued` means the scanner has produced candidates but no systematic
source-to-source pass is complete.

| Subsystem | State | Required evidence |
| --- | --- | --- |
| Main tick and mission state | in progress | Resolve PA-013; retain the phase- and path-ordering tests. |
| Item interaction / pickup | verified for explicit Take | Keep the no-proximity-pickup regression. Audit other interaction shortcuts. |
| Enemy detection and state machine | in progress, high risk | Resolve PA-025 and continue state-by-state comparison. |
| Melee and damage | queued, high risk | Review every remaining simplification against actor-human combat code. |
| Movement, paths, doors, lifts | in progress, high risk | Retain PA-014/PA-022/PA-030 regressions and audit remaining lift/door timing. |
| Script natives and callbacks | in progress, high risk | Retain PA-021/PA-023/PA-024/PA-028 regressions and audit remaining natives. |
| Sequence manager and messages | in progress, high risk | Resolve PA-032; retain condolence and SendMessage ordering tests. |
| Projectiles and abilities | queued | Per-type Hourglass and spawn-frame comparison. |
| Audio-driven AI state | in progress, high risk | Missing-duration parity is fixed; continue auditing completion callbacks. |
| Deterministic snapshots and replay | in progress, high risk | Resolve PA-033; retain fatal replay preload and seed-priority regressions. |
| RNG | intentional architecture, audit in progress | Retain one snapshotted stream; review ranges and call order under PA-036. |
| Spellforge Lua | incomplete post-original feature | Resolve PA-034 before deterministic use; provenance must name versioned Spellforge sources, not Original. |
| Save/campaign persistence | queued | Resolve PA-037 and compare every persistent native and mission transition. |
| Rendering, UI, HTTP, replay, multiplayer | exception review | Gameplay state changes require `NEW_FEATURES.md`; pure host behavior is host-only. |

## Audit Workflow

1. Run the scanner and select one gameplay finding, starting with high-priority
   state mutation, AI, scripts, movement, and combat.
2. Locate the exact original caller and callee. Record file, function, gates,
   ordering, and failure behavior.
3. Classify the ledger item before editing. Do not convert uncertainty into a
   permissive fallback.
4. Reproduce observable behavior with the smallest unit/integration test or a
   recorded replay. For timing issues, assert the exact frame and sequence
   state, not only the eventual result.
5. Make one behavioral correction per commit where practical. Update the
   ledger status and add an `Original:` comment at non-obvious ports.
6. If the difference is desired, document it in `docs/NEW_FEATURES.md` and add
   a test that makes the intentional behavior explicit.

Stale comments must be corrected as part of review. In particular,
`reconsider_enemy_approach` and `tick_enemy_sword_attacks` are no longer the
small simplified implementations their headings describe, while the
`is_detecting_360_degrees` distance check mirrors an original method whose own
name is approximate. Keyword matches for those comments are not evidence of a
current behavior mismatch.
