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
| PA-025 | Charly-to-officer handling used incomplete visibility gates and transitioned before learning whether the officer accepted the report. | `RHArtificialMalignity` performs normal `IsDetecting(mpAntagonist)`, calls the officer's `Think` synchronously, enters the seen state only on acceptance, and calls `ReturnToDuty` on refusal (`RHartificialmalignity.cpp:3614-3625`; detection rules in `RHelementactornpc.cpp`). | `64f671e20` restores exact normal-detection inputs, including projected obstacle/light radius and building/door gates, and feeds the officer's real synchronous `Think` result back to Charly. Focused tests cover acceptance, refusal, view radius, outdoor activity, and same-building exclusions. |
| PA-026 | Shoulder-ceiling checks ran for every `CarryingOnShoulders` posture. | `RHElementActorPC::Execute`, `RHelementactorpc.cpp`, gates the check on `WALKING_CARRYING_ON_SHOULDERS`. | `93faf0436` adds the action gate and focused regression coverage. |
| PA-027 | Messenger processing, condolations, and self-stimuli were deferred to global tick tails. | `RHMessenger::ForwardMessage` and `RHSequenceElement::SendCondolationCard` dispatch synchronously and re-entrantly (`RHMessenger.cpp`, `RHsequenceelement.cpp`). | `6f7907eaf` restores recursive messenger ordering and synchronous condolence arbitration, protected by same-frame ordering tests. |
| PA-028 | Script `SendMessage` bypassed the original sequence element and dispatched after the script call. | `RHScript::SendMessage`, `RHScript.cpp`, launches `RHCOMMAND_SEND_MESSAGE`; `RHSequenceElementSendMessage::Go`, `RHsequenceelement.cpp`, performs delivery. | `618df2081` restores sequence launch, priority/arbitration, deferred delivery, and exact callback-order tests. |
| PA-029 | Rust ran mission `PostInitialize` at the end of the first engine tick before the original presentation boundary. | `RHGame::GameLoop`, `RHgame.cpp`, calls it after the first refresh and sound hourglass. | `cdcf5d0fe` restores an explicit host-driven post-refresh/sound stage and mirrors it in rollback replay, with exact ordering tests. |
| PA-030 | Collinear movement-line intersection fabricated impact parameter `t = 0.5`. | `SBGeoSegment2D::operator^` returns the overlap segment, and `RHFastFindGrid::IsReachable` treats it as a collision; the first overlap endpoint reached by the movement is authoritative. | `be3e7e987` restores earliest-overlap ordering and adds collinear geometry regressions. |
| PA-032 | WAIT-priority elements were appended to `elements_to_go`, delaying `Go()` until the next sequence-manager hourglass. | `RHSequence::Launch` calls `NextSequenceElementsGo` before returning (`original-code/RHsequence.cpp:199-221`); that method advances the stable level range, calls `Go()` inline for `RHPRIORITY_WAIT`, and registers every other priority (`RHsequence.cpp:235-289`). `RHSequenceElement::Go` synchronously calls owner `Instruct` or engine `PerformExecuteCommand` (`RHsequenceelement.cpp:440-458`), while non-immediate registration alone appends to the manager FIFO (`RHsequencemanager.cpp:951-970`), drained by `Hourglass` (`RHsequencemanager.cpp:931-944`). | This commit emits WAIT `Go()` actions into the ordered synchronous registration stream at both initial launch and re-entrant `Ready()` advancement. NORMAL non-immediate elements remain on `elements_to_go`; IMMEDIATE registration retains its order in the same synchronous stream. The engine callback loop drains re-entrant WAIT/IMMEDIATE successors ahead of older sibling actions, with a focused launch-return state and ordering regression. |
| PA-033 | Rollback reconstructed a default `HostDisplayState`, losing an active zoom transition even though its flags gate authoritative simulation work. | Original zoom messages enter through `RHGame` (`RHgame.cpp:2070-2071`), and `RHEngine::Serialize` persists the corresponding `mbackgroundTransform` transition state (`RHengine.cpp`). | `3b46c1720` derives the gameplay zoom gate from the serialized engine camera transition whenever a snapshot is restored and after replayed zoom commands. A rewind-during-zoom regression proves restored and uninterrupted simulations take the same gate. |
| PA-031 | Push handling fell back to radial movement for unexpected thrust kinds. | `RHElementActorHuman` handles the three supported thrust kinds and asserts otherwise (`RHelementactorhuman.cpp`). | `954d4ee8d` restores the supported dispatch and rejects invalid thrust kinds with focused tests. |
| PA-035 | A requested replay whose header could not be decoded warned and substituted the multiplayer seed or zero before Engine construction. | Replay is a Rust extension with no Original equivalent; explicit playback is a deterministic contract and cannot invent initial RNG state. | Replay seed resolution now returns a typed, contextual preload error before `Engine::new`; corrupt JSON/header data, invalid compact data, and missing fields are fatal, while valid replay and non-replay multiplayer seeds retain their priority. |
| PA-037 | Campaign state moved through optional Engine, session, `ScriptContext`, and `GameHost` holders without proving that the one required allocation was restored on every exit. | Original exposes one required `RHCampaign` singleton across mission and script calls (`RHCampaign.cpp`, `RHgame.cpp`). | `bda4f3ffc` adds required campaign leases and identity checks. RAII restoration preserves the exact campaign allocation across native success, error, panic unwinding, save/load, and every mission exit; missing or replacement campaigns fail with context. |
| PA-038 | AI snapshots inferred archer identity from positive normal-shot range and reconstructed phalanx perception using the leftmost member's radius and stale neighbor entries. | `RHArtificialMalignity::IsArcher` is exactly bow presence, and phalanx merging evaluates each member's current detection inputs (`RHartificialmalignity.h`, `RHartificialmalignity.cpp`). | `55b85a1d9` defines archers by bow presence and snapshots each phalanx member's own radius, sector, elevation, posture, building state, detectable enemies, and current opponent list. Per-member 180/360-degree visibility and occlusion tests cover heterogeneous radii and stale entries. |
| PA-039 | Fatal animated pushes set `Posture::Dead` during translation and omitted the original pushed-flight start and landing state boundaries. | `RHElementActorHuman::ExecuteFallingPushed` applies `Flying`/`WaitingSword` at motion start and `DeadBack` or `Lying` with `WaitingSword` at termination (`RHelementactorhuman.cpp`). | `b8fe27e82` defers animated death/knockout posture changes until the falling order starts and lands, preserves the dead-rider override, updates script sectors before domino effects, and tests fatal, knockout, rider, and already-grounded paths. |
| PA-034 | Spellforge Lua could enter replay, rollback verification, or multiplayer even though its host-owned VM state is absent from Engine snapshots; required construction/startup and event errors were discarded, and startup RNG calls lacked an Engine scope. | Spellforge is a post-Original extension. Original only supplies the placement/failure boundary: SCB startup is structurally required, Initialize runs during engine initialization, PostInitialize follows the first refresh/sound boundary, and failed VM calls assert in debug builds (`GEngineScript.cpp`, `RHengine.cpp`, `RHgame.cpp`). | `cfcff6857` rejects Spellforge before deterministic/network Engine construction, makes required construction and startup dispatch fail with typed mission/event context, and installs the authoritative RNG scope. Ordinary single-player remains available by explicitly omitting its default rollback diagnostic. Lua snapshots and the unwired event surface remain future extension work, not claimed parity. |
| PA-036 | The serialized Rust RNG architecture lacked a complete range/order/provenance inventory and allowed new unlabeled call sites, while `Rand(max <= 0)` fabricated zero and two floating Original ranges/orderings had drifted. | Original owns one process-global C RNG, seeds production from wall time, implements script `Rand(max)` as `rand() % max`, randomizes Sherwood returners before the 100 beam-slot swaps, and uses inclusive `rand()/RAND_MAX` floating ranges (`launcher.cpp`, `RHScript.cpp`, `RHCampaign.cpp`, `RHengine.cpp`). | `c0fae908b` records 84 serialized-stream sites / 127 reviewed source uses plus one state-hashed auxiliary site in `RNG_AUDIT.md`. Typed sites, a public-entry-point allowlist, exact ambient/bootstrap exceptions, macro detection, snapshot/next-draw tests, and source parsing reject unreviewed additions. It also restores Sherwood draw order and inclusive floating jitter, and rejects invalid script bounds without consuming a draw. Bit-identical libc output remains intentionally out of scope. |

## Open Findings

Priority reflects likely gameplay impact, not implementation effort.

| ID | Priority | Status | Finding and evidence |
| --- | --- | --- | --- |
| PA-013 | High | incomplete | `09ec7c5fe` restores a live-size creation-ordered pass for bow release/projectiles, PC auto-heal, purses/coins, wasps, nets, fallback-timed melee completion, and abilities. `4c9b5715a` restores the mixed pre/post target position observed by EYES_FOLLOW. `1a1901c7c` moves straight/assault strike advancement, synchronous damage, and completion to the attacker's slot, so an earlier lethal strike interrupts a later chained attacker before it can hit. `b36b4e14b` makes SEEK tolerance sample target position, sector, and hotspot at the seeker's slot, observing earlier movers after their step and later movers before it. `44cfa10f4` runs the Lacklandist optical Enemy→Body→Object→Friend→MissedFriend→Beggar scan and its queued sight stimuli at each NPC creation slot; the swapped-order regression proves that a later officer sees an earlier soldier's synchronous EVENT_VIEW transition while an earlier officer sees the soldier's pre-transition state. `63c5e2245` preserves the full detection-built tactical input for queued EVENT_VIEW, restores Body predetection-shadow-before-commit FIFO order, limits live target snapshots to the current NPC, and skips live-context construction for quiet NPCs. `ef8d67720` moves acoustic detection into that same creation-ordered NPC boundary: it walks each Enemy list in order, applies the original 3D subjective-volume/latch rules, and dispatches rising-edge EVENT_HEAR synchronously before that NPC's optical state snapshot. The regression proves HEAR can change Default→Seeking and thereby make the immediately following sub-threshold optical detection instant; a stale dead-PC entry before the audible PC is also covered. `4721f389a` moves every falling-edge EVENT_OUTOFVIEW into the current NPC's Enemy detectable-list FIFO, interleaved with the currently supported selected rising VIEW and ahead of Body/Object/Friend/MissedFriend/Beggar. Its regression proves OUTOFVIEW can enter Seeking before a later visible Body stimulus is handled, matching original `RefreshDetection` causality. Animation/ActionChange, Royalist detection coordination, per-stimulus live context rebuilding, multiple rising Enemy VIEW events in one scan, NPC Hourglass tails, and non-straight melee remain batched or incomplete. Sweeps, pushes, riders, smalltalk, and the remaining NPC boundaries need separate exact evidence. Full parity still requires a per-entity Hourglass API (`RHengine.cpp:3715-3724,7909-7944`; `RHelementactor.cpp:534-709,7314-7380`; `RHelementactornpc.cpp:1371-1675,3495-3614`). |

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
| Movement, animation, ActionChange, scroll Hourglass | actor/object virtual Hourglass and Execute methods | PA-013; EYES_FOLLOW and live SEEK-target mixed pre/post observations are fixed; animation callbacks and broader dispatch remain batched |
| NPC view, detection, timers, speech, patrol | `RHElementActorNPC::Hourglass` and AI subclasses | phase order verified by PA-016; followed-target, FRIEND-versus-synchronous-EVENT_VIEW, synchronous pre-optical EVENT_HEAR, and Enemy OUTOFVIEW-before-later-detectable causality are fixed at the creation boundary; Royalist coordination, multiple rising VIEW events, per-stimulus live contexts, and the broader NPC tail remain batched or incomplete |
| Projectiles, straight melee, and abilities | per-type virtual Hourglass/Execute methods | live creation order, spawn-frame inclusion, and straight/assault strike causality verified by PA-013 regressions; non-straight melee remains batched |
| Titbits, deselection, anonymous timers | tail of `RHEngine::PerformHourglass` | structurally verified; titbit display-order approximation is visual |
| Condolations and self-stimuli | `RHSequenceElement::SetState` to actor `SendCondolationCard` | synchronous ordering verified by PA-027 |
| PostInitialize | mission loop in `RHgame.cpp` | host boundary verified by PA-029 |
| RNG, rollback side effects, minimap/marks/camera | no single original phase | RNG ranges/order/snapshot ownership reviewed under PA-036; audit other gameplay state individually |

## Coverage Matrix

`queued` means the scanner has produced candidates but no systematic
source-to-source pass is complete.

| Subsystem | State | Required evidence |
| --- | --- | --- |
| Main tick and mission state | in progress | Resolve PA-013; retain the phase- and path-ordering tests. |
| Item interaction / pickup | verified for explicit Take | Keep the no-proximity-pickup regression. Audit other interaction shortcuts. |
| Enemy detection and state machine | in progress, high risk | PA-025 fixed; retain synchronous officer acceptance/refusal and exact-detection regressions, then continue state-by-state comparison. |
| Melee and damage | in progress, high risk | Retain the chained straight-strike creation-order regression; review sweeps, pushes, riders, smalltalk, and every remaining simplification against actor-human combat code. |
| Movement, paths, doors, lifts | in progress, high risk | Retain PA-014/PA-022/PA-030 and creation-ordered SEEK regressions; audit remaining lift/door and animation-callback timing. |
| Script natives and callbacks | in progress, high risk | Retain PA-021/PA-023/PA-024/PA-028 regressions and audit remaining natives. |
| Sequence manager and messages | in progress, high risk | PA-032 fixed; retain WAIT launch-return, condolence, and SendMessage ordering tests. |
| Projectiles and abilities | queued | Per-type Hourglass and spawn-frame comparison. |
| Audio-driven AI state | in progress, high risk | Missing-duration parity is fixed; continue auditing completion callbacks. |
| Deterministic snapshots and replay | in progress, high risk | PA-033/PA-035/PA-034 fixed or contained; retain active-zoom rewind, fatal replay preload, seed-priority, and Spellforge mode-rejection regressions. |
| RNG | verified intentional architecture | Retain the typed, snapshotted stream, auxiliary/ambient classifications, structural inventory guard, and exact draw-order restoration tests from PA-036. |
| Spellforge Lua | contained post-original feature | Normal single-player startup is fail-fast and RNG-scoped; deterministic/network modes reject it until a versioned event surface and Lua snapshot policy are implemented. |
| Save/campaign persistence | in progress | PA-037 fixed; retain identity/unwind/mission-exit regressions and compare every persistent native and transition. |
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
