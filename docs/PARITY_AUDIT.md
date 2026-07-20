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
  Failed-request deadlines remain in the same phase. Seek refresh remains in
  `Entities`; WAIT_TIMER maintenance now runs after Execute at each actor's
  live creation slot, matching `RHElementActor::Hourglass`. The intervening original
  `CheckForCollision` has a live mobile-damage arm: ten shipped missions use
  chariots. Rust runs that containment/damage check between `Paths` and the
  mobile/entity hourglasses, preserving the original previous-tick movement
  test and reverse human traversal.
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

- **NPCs:** The actor-owner envelope now nests the supported Soldier, Human,
  Actor, PC, and NPC Hourglass slices at each live legacy slot, including
  patrol, concussion wake, produced noise, tiredness, detection, and the full
  NPC tail (`original-code/RHelementactornpc.cpp:3495-3659`). `NpcOrders` and
  non-NPC Listen/object discovery remain separate system boundaries, as do the
  movement/combat/ability owners described below.
- **Entity systems:** Movement, special projectile variants, active combat,
  and ability work remain batched by system in Rust, whereas the original
  invoked subtype hourglasses from the creation-ordered element loop. Base entity refresh is
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
| PA-037 | Campaign state moved through optional Engine, session, `ScriptContext`, and `GameHost` holders without proving that the one required allocation was restored on every exit. | Original exposes one required `RHCampaign` singleton across mission and script calls (`RHCampaign.cpp`, `RHgame.cpp`). | `bda4f3ffc` first added required campaign leases and identity checks; the ownership refactor later removed those transitional leases. A live `MissionDomain` now owns one concrete `Campaign`, script dispatch borrows it in place, and consuming mission finish returns it by value. Snapshot migration rejects a missing required campaign instead of fabricating one. |
| PA-038 | AI snapshots inferred archer identity from positive normal-shot range and reconstructed phalanx perception using the leftmost member's radius and stale neighbor entries. | `RHArtificialMalignity::IsArcher` is exactly bow presence, and phalanx merging evaluates each member's current detection inputs (`RHartificialmalignity.h`, `RHartificialmalignity.cpp`). | `55b85a1d9` defines archers by bow presence and snapshots each phalanx member's own radius, sector, elevation, posture, building state, detectable enemies, and current opponent list. Per-member 180/360-degree visibility and occlusion tests cover heterogeneous radii and stale entries. |
| PA-039 | Fatal animated pushes set `Posture::Dead` during translation and omitted the original pushed-flight start and landing state boundaries. | `RHElementActorHuman::ExecuteFallingPushed` applies `Flying`/`WaitingSword` at motion start and `DeadBack` or `Lying` with `WaitingSword` at termination (`RHelementactorhuman.cpp`). | `b8fe27e82` defers animated death/knockout posture changes until the falling order starts and lands, preserves the dead-rider override, updates script sectors before domino effects, and tests fatal, knockout, rider, and already-grounded paths. |
| PA-034 | Spellforge Lua could enter replay, rollback verification, or multiplayer even though its host-owned VM state is absent from Engine snapshots; required construction/startup and event errors were discarded, and startup RNG calls lacked an Engine scope. | Spellforge is a post-Original extension. Original only supplies the placement/failure boundary: SCB startup is structurally required, Initialize runs during engine initialization, PostInitialize follows the first refresh/sound boundary, and failed VM calls assert in debug builds (`GEngineScript.cpp`, `RHengine.cpp`, `RHgame.cpp`). | `cfcff6857` rejects Spellforge before deterministic/network Engine construction, makes required construction and startup dispatch fail with typed mission/event context, and installs the authoritative RNG scope. Ordinary single-player remains available by explicitly omitting its default rollback diagnostic. Lua snapshots and the unwired event surface remain future extension work, not claimed parity. |
| PA-036 | The serialized Rust RNG architecture lacked a complete range/order/provenance inventory and allowed new unlabeled call sites, while `Rand(max <= 0)` fabricated zero and two floating Original ranges/orderings had drifted. | Original owns one process-global C RNG, seeds production from wall time, implements script `Rand(max)` as `rand() % max`, randomizes Sherwood returners before the 100 beam-slot swaps, and uses inclusive `rand()/RAND_MAX` floating ranges (`launcher.cpp`, `RHScript.cpp`, `RHCampaign.cpp`, `RHengine.cpp`). | `c0fae908b` records 84 serialized-stream sites / 127 reviewed source uses plus one state-hashed auxiliary site in `RNG_AUDIT.md`. Typed sites, a public-entry-point allowlist, exact ambient/bootstrap exceptions, macro detection, snapshot/next-draw tests, and source parsing reject unreviewed additions. It also restores Sherwood draw order and inclusive floating jitter, and rejects invalid script bounds without consuming a draw. Bit-identical libc output remains intentionally out of scope. |

## Open Findings

Priority reflects likely gameplay impact, not implementation effort.

| ID | Priority | Status | Finding and evidence |
| --- | --- | --- | --- |
| PA-013 | High | incomplete | `09ec7c5fe` restores a live-size creation-ordered pass for bow release/projectiles, PC auto-heal, purses/coins, wasps, nets, fallback-timed melee completion, and abilities. `4c9b5715a` restores the mixed pre/post target position observed by EYES_FOLLOW. Subsequent slices restore creation-ordered straight and non-straight melee, SEEK target sampling, acoustic and optical detection, full detectable FIFO handling, live per-Think inputs, Royalist/civilian detection, base Actor Execute/ActionChange, owner-local AI state callbacks, and stationary WAIT/Lift/WaitingSword work. This slice fuses the live Soldier prelude and patrol, Human concussion/shoot-list prelude, base Actor Execute/ActionChange, Human produced-noise/tiredness tail, PC auto-heal, and complete NPC detection/AI tail into one append-aware legacy-slot envelope. Each owner closes synchronous recursive work before the next slot; PC noise and mutable detection metadata are sampled at the consuming slot, and RNG-bearing forecasts are not built globally. Movement, active strike/bow/ability ownership, riders (including rider charge), non-NPC Listen/object discovery, and remaining entity-kind Hourglass boundaries remain PA-013 debt (`RHengine.cpp:3715-3724,7909-7944`; `RHelementactorsoldier.cpp:2573-2605`; `RHelementactorhuman.cpp:277-324`; `RHelementactor.cpp:534-728`; `RHelementactorpc.cpp:1904-1947`; `RHelementactornpc.cpp:3495-3659`; `RHartificialintelligence.cpp:7172-7259,7272-7305`). |

Focused PA-013 stationary/idle progress: the live actor-slot coordinator now
applies WAIT_TIMER and WAIT_FREE_LIFT to the just-produced Execute result
before completion effects are observed, including frozen actors with an
installed wait order. Lift authorization is rechecked and reserved in that
owner slot. The WaitingSword Execute arm now owns smalltalk-hint consumption
and swordfight evaluation, preserving synchronous opponent/initiative changes
for later creation slots. Evidence is traced to
`RHelementactor.cpp:606-707` and
`RHelementactorhuman.cpp:3486-3505,7988-8014,8222-8407`.

2026-07-20 owner-envelope fusion: one live `while slot < entities.len()` walk
now closes Soldier patrol/prelude, Human concussion and shoot-list work, base
Actor Execute/ActionChange, PC noise/tiredness/heal, or the complete NPC
detection/tail before advancing. Removed slots remain holes and callback-spawned
later actors run in the same frame. Concussion FIT_AGAIN, patrol-coordinate
Think, and all resulting owner FIFO work drain synchronously. Detection views
are rebuilt at the consuming NPC slot. Earlier NPCs see later actors at the
preserved pre-movement position, later NPCs see already-visited actors live,
and callback-spawned targets absent from the oracle use their never-moved live
position; patrol snapshots follow the same rule. Destination alternatives are
prepared without RNG, while only the exact primary/missed-PC/officer handler
that consumes a forecast resolves its building-exit draw. Optical target
construction reads those same owner-relative snapshots instead of the later
live position. Building exits retain the Original ordered all-gates rejection
loop, including every rejected-entry RNG draw. FIT_AGAIN applies detectable
restoration, resurrection, eye state, and every opposing NPC's `BlinkEnemy`
inline at the waker slot even under FrozenAll; its owner-work FIFO also places
the Enemy timer/eye writes after the SetState callback and the Friendly eye
write before ReturnToDuty's callback. Patrol history has no extra
chief-liveness gate, coordinate Think precedes its instructed direction per
member, and macro `CMD_PATROL_DIRECTION` broadcasts (including waiting-member
`FaceTo`) close synchronously. FrozenAll, static-AI freeze, script/AI locks,
fried PCs, and inactive PC noise metadata retain their separate Original
gates.

Remaining PA-013 debt is deliberately unchanged outside this slice: movement,
active strike and bow execution, ability ownership, rider charge, non-NPC
Listen/object discovery, and remaining entity-kind Hourglass boundaries still
have separate owners. This slice does not claim save/replay shape compatibility
with pre-change snapshots.

2026-07-20 script-native AI-state slice: accepted `SetAIState` calls now yield
at the native instruction to a typed owner-local barrier. `SCRIPT_DRIVEN`
uses the real Enemy/Friendly `SetState` callback-before-assignment path;
`DEFAULT` runs ReturnToDuty synchronously; and `SEEKING`/`FLEEING` run the
complete `StartThink(NULL, NO_EVENT)` prelude/filter/post-gates, effect, causal
Move translation, matching `EndThink`, and recursive owner FIFO before the VM
resumes. Panic classification occurs after the NO_EVENT callback, and speech,
orders, condolences, and self-stimuli stabilize without forecasting unrelated
actors. This matches `RHScript::SetAIState` (`RHScript.cpp:4274-4330`),
`RHArtificialIntelligence::SetAIState`/`Panic`/`StartThink`
(`RHartificialintelligence.cpp:4730-4778,2884-3035,914-1245`), and the Enemy
and Friendly typed `SetState` callbacks (`RHartificialmalignity.cpp:9225-9272,
9404-9406`; `RHartificialbonhomie.cpp:1185-1237`) for this focused entry path.
It does not fuse the remaining movement/combat/NPC envelopes or complete the
broader PA-013 Actor Hourglass ownership work described above.

2026-07-19 ActionChange ordering slice: `RHElementActor::Hourglass`
(`RHelementactor.cpp:686-721`) snapshots `GetAnimation()` and `moldAction`,
calls the actor VM synchronously, then rereads the live animation into
`moldAction`; `GetAnimation()` returns `RHNONANIMATION_END` when no current
order exists (`RHelementactor.h:209`). Rust now applies that contract one
legacy creation slot at a time over the live element-array size. An earlier
callback can therefore change a later actor's callback arguments in the same
pass, while a mutation of an already visited actor remains a next-pass
transition. Real SCB/native regressions use `SetActorPosture` to cover both
directions and self-mutation, including pre-callback arguments versus the
post-callback retained animation. The 2026-07-20 slice below closes the
separation from generic actor animation while retaining this callback contract.

2026-07-20 base-actor animation boundary: Rust now walks the live legacy
element-array size, resolves each slot through `id_at_legacy_slot`, runs that
actor's existing generic animation/Execute helper when eligible, immediately
executes the soldier combat-injury Think for every implemented terminating arm
(including `StandingUpSword`), then applies its `AnimCompletionOutcomes`,
non-interruptable priority lift, and synchronous successor work, releases
borrows, and dispatches `ActionChange`. The injury Think is isolated from the
NPC's older deferred stimulus FIFO and finishes before completion/DoNext, as in
the Original Execute→base-Hourglass nesting. Terminating animation promotes the
next order before same-actor callback arguments and retention; an earlier
callback can replace a later actor's order and the later sprite executes it in
the same pass; and WAKING_UP DONE can install the later target's recovery order
before that target animates. Swapped-order regressions prove all reverse cases
defer to the next pass. Generic animation eligibility does not suppress
ActionChange for inactive, execution-frozen, moving, active-shot, or
active-melee actors. Nonactor patch/FX/object animation remains a separate
exactly-once pass before actor callbacks, preserving the prior patch/door
visibility and deferring callback-spawned nonactors. This intentionally remains
a pre-actor batch, not exact live-slot parity; the old global actor-freeze gate
still suppresses it. Lazy Wait creation and synchronous drain are
isolated to each actor slot, so an earlier actor cannot consume or observe a
later owner's pending Wait work. Execute-arm inputs are sampled live only
after the animation skip gates select an eligible arm, so skipped actors do not
dereference stale opponent, antagonist, or door references. Movement, melee,
bow, abilities, and NPC detection/tails deliberately retain separate subsystem
owners. AI state callbacks and speech now close one ordered owner-local FIFO at
every audited return boundary, but that barrier is not a claim of arbitrary
inline observation between pure-Rust statements. These remaining boundaries
prevent a full Actor Hourglass coordinator, and exact NPC derived-class nesting
around the base actor remains absent.

PA-013 progress note: `b9ba6f4ee` restores the original's two inactive
eligibility gates for the implemented NPC-side blip/acoustic and soldier
Enemy-optical paths. Inactive NPCs with a door pointer or BUILDING sector
still enter `RefreshDetection`; only an inactive soldier occupying an actual
BUILDING sector reaches optical scanning. Living inactive PCs remain Enemy
detectables, stay silent and unable to fight, remain visible through the
same-building short circuit, and otherwise generate an ordered falling edge
instead of being cleaned up. Regressions cover
HEAR(runner)→VIEW(inactive same-building PC)→OUTOFVIEW(inactive outdoor PC),
the outdoor no-op, door-only blip/hearing plus both-maxima reset and optical
return, and Royalist auto-reveal's common 16-frame cadence. Creation-slot NPC
blip integration landed in the follow-up below; the civilian optical and
Lacklandist-to-Royalist Enemy paths are closed by the later mixed-Enemy
follow-up. Broader inactive Hourglass behavior remains open.

2026-07-19 follow-up: `a915046f6` moved NPC-owned `SeesBlip` work into
each NPC's creation-ordered `RefreshDetection` slot, rebuilds Enemy-list
products from the live detectable list at every queued `Think`, preserves
`seen_now` and `seen_last_frame` as separate ordered inputs, and makes stale
NPC/detectable IDs fail with context. It also restores live lift/stairs target
approach data and correct eye-point projection at equal ground elevation.

2026-07-19 non-straight melee follow-up: lateral, push-aside, half-circle,
and full-circle strikes now advance and apply damage synchronously at the
attacker's live creation slot. This follows `RHElementActor::Hourglass`
(`RHelementactor.cpp:534-731`) and the strike execute methods in
`RHelementactorhuman.cpp:9195-10222`: lateral IN_PROGRESS advances its angle
raw before testing victims; circle effects test the existing angle and then
apply the clamped/same-final-sector tail advance, including the angle-only
advance on the DONE initialization call. Push and sweep damage retain
actor-list victim FIFO. Swapped-order regressions prove an earlier lethal
lateral/push interrupts a later attacker before its slot, while reversing
creation order does not retroactively suppress the hit. PA-013 remains
incomplete for the broader actor/NPC Hourglass boundary, animation callbacks,
riders, smalltalk, and the other items listed above.
The old notes above about a batched NPC blip pass and frozen mid-FIFO Enemy
aggregate are therefore closed.

2026-07-19 mixed-Enemy follow-up: `RHelementactornpc.cpp:1371-1675,
1877-1993,2085-2291` proves that every NPC scans one Enemy list, accumulates
one suspect sum across interleaved entries, then walks that same list in order
through `HandleDetection` before draining any queued `Think`. Rust now uses
that single creation-slot walk for civilians and both soldier camps, choosing
the Original PC-versus-NPC cadence per entry, treating life <= 0 as dead,
retaining-but-hiding HollowMan targets from Lacklandists, and preserving the
established Enemy→Body→Object→Friend→MissedFriend→Beggar FIFO. Royalist Enemy
visibility remains strict modulo-16 even while staring, following, or alerted;
only Lacklandist refresh-always states bypass per-entry cadence. Blind eyes,
outdoor blipped Lacklandists, and newly guarded unseen PCs invalidate cached
visibility before a closed cadence can reuse it, while a door-transit pointer
counts as inside for the blip gate without fabricating a same-building handle.
Each PC creation slot also samples its live posture/detection Z and current
beggar order instead of the tick-start snapshot. Swapped
PC/soldier order, non-Stare 2-versus-16 cadence, negative-life cleanup,
HollowMan visibility, detectable mutation between retained FIFO entries, and
Royalist civilian, strict-cadence cache, pre-cadence invalidation, door-transit,
live-posture/order, and civilian VIEW regressions protect the completed boundary. Missing observer
AI/NPC state and invalid or missing Enemy targets now fail with
observer/target context.

2026-07-19 cross-elevation follow-up: normal human/object visibility now
retains both Original world-space points and projected LOS points instead of
using one `MapPoint` for both. This matches `ComputeVisibility` and
`IsDetecting` in `RHelementactornpc.cpp:2318-2575`: world X/Y (with the
Original Y stretch) drives radius, forward/cone, and distance sharpness; the
full stretched 3D vector drives the close human check and object range; and
the projected points remain isolated to the Rust LOS representation. The
same split reaches periodic Lacklandist/Royalist scans, Body/Friend/Beggar
human scans, Objects, synchronous AI `IsDetecting`, script `Sees`, and cached
officer cone checks. Cross-elevation human/object and exact 3D-sharpness
regressions protect the coordinate split; the existing creation/FIFO tests
remain unchanged.

2026-07-19 NPC pre-detection boundary N1: the production coordinator now
follows `RHElementActorHuman::Hourglass` / `RHElementActorNPC::Hourglass`
ordering at `RHelementactorhuman.cpp:277-305,335-405` and
`RHelementactornpc.cpp:3495-3554`. At each NPC creation slot it drains the
existing stimulus FIFO prefix through natural `EVENT_FITAGAIN`, applies that
NPC's resurrection fan-out and eye change, applies wake `BlinkEnemy` inline
to every opposing NPC regardless of observer creation order, consumes that
NPC's `mbInformMyFriends`, refreshes
that NPC's stateful view once, and then runs its synchronous
`RefreshDetection`. The canonical AI enqueue covers both soldier and
civilian NPC controllers while retaining intentional non-NPC no-ops.
Swapped-order regressions prove earlier body informs affect a later observer,
while later body work remains queued for an earlier observer's next slot;
wake blinks instead follow the Original inline all-observer mutation.
Additional regressions cover both wake producers, civilian wake,
FIFO prefix order, simultaneous recovery/inform flags, and earlier/later
LOOKTHERE receivers. The N2 follow-up below closes the then-outstanding
post-detection tail. Natural concussion wake still launches its
`Recover`/`StandingUp` sequence globally before the waker reaches its owner
prelude; restoring that base-human animation/`ActionChange` interleaving is
explicit future PA-013 boundary debt. The eager posture/action/eye writes are
gone, but this scoped slice does not complete PA-013.

2026-07-20 NPC post-detection boundary N2: the production coordinator now
continues each NPC creation slot through the exact
`RHelementactornpc.cpp:3548-3657` tail before the next NPC enters
`RefreshDetection`: ambush refresh, unconditional non-frozen deafness refresh,
BUSY edge, ladder recovery, wrapped phase and civilian `RandomSpeech`, lock
gate, `The16thFrame`, normal and macro timers, real emoticon expiry, and the
retained stimulus FIFO. The lock branch is sampled once before the suffix:
locks or FrozenAll acquired by The16thFrame or EVENT_TIMER do not suppress the
remaining due timers/emoticon work, while the retained FIFO alone rechecks
AI/script locks before every item and preserves its suffix. Locked deadline
increments use unsigned wrapping, elapsed macro timers stop outside
`DefaultInMacro`, and every AI-calling boundary drains that owner's common
soldier/civilian effects, orders, synchronous LOOKTHERE work, condolations, and
self-stimuli before the next phase or owner. Shared focus, view-cone, posture,
and detectable mutations apply through common `NpcData`; civilian macro
completion therefore clears `MissedFriend` immediately instead of losing the
taken effect. Later Charly/panic/seek consumers build live scratch only at
their own synchronous boundary, after earlier drain mutations. EVENT_TIMER
combat stance, target stop, and civilian panic come from that single common
drain rather than a duplicate timer-specific post-pass. Production no longer
repeats the 6c/6d owner drains globally. Regressions cover two-owner whole-tail
order, swapped retained-VIEW/Friend causality, production gate cutoff,
post-gate lock/freeze sampling, lock-acquired FIFO suffix retention, civilian
timer/retained/self/The16thFrame/macro order causality, civilian macro
detectable cleanup, ambush LookSidewards draining,
unlocked/locked emoticons, off-cadence deafness, ladder threshold, wrapped
phase, and timer/deadline overflow. `SetState` script notifications no longer
use the global post-NPC batch: each owner drains immediately after an
engine-entered Enemy/Friendly AI call releases its borrow, and direct
parade/special-strike entry points close the same boundary before timer/order
continuation. The serialized/state-hashed FIFO records outgoing and incoming
state/substate plus the raw source; each callback exposes the outgoing pair,
uses the bound actor VM/frame, ignores the return, then re-resolves the typed
owner and commits the recorded incoming pair. Disabled, absent, unbound, and
non-overriding script cases consume their entries. The temporary outgoing
restore does not undo an Enemy post-`SetState` tail already applied before
callback re-entry; Friendly alert is correctly pre-callback and visible.
Synthetic SCB/native tests
cover state/source/alert visibility, FIFO, callback mutation, owner order, and
all skip gates. `Say` now shares this serialized/state-hashed owner FIFO with
`SetState`: every engine return boundary settles attempts in statement order,
and a rejected MYTALK callback plus recursively produced work finishes before
the outer FIFO tail resumes. Direct sequence Speak/Provoke, nets, wasps,
scrolls, animation remarks, melee warcries and `SayOuch`, panic paths, and
cross-NPC Say calls close that boundary at their source before the following
termination/order/noise/effect. This restores the source-verified
`RHArtificialIntelligence::Say` filter/category/ID-zero/display/forbid order,
shared CYCLE_3 placement, and the lazy ordered forbidden-list scan
(`RHartificialintelligence.cpp:5846-6182,6273-6390`). Exact-ID
`SoundIsFinished` completion is the first mutation of the next
`PerformHourglass` deferred-effects phase, before every deferred simulation
mutation and NPC Hourglass;
emergency replacement removes the old schedule and mismatch callbacks retain
the replacement (`RHelementactornpc.cpp:6408-6511`). Exact arbitrary
pure-Rust reads between queued `Say`/`SetState` statements, fully inline
intra-`Think` placement, and exact nesting with every remaining actor subtype
are still explicit PA-013 debt; this slice does not complete PA-013.

Movement follow-up `3daf2efaf` removed the two warning-and-reseed fallbacks for
same-ID motion orders. A started order now requires its cached goal and map
increment to remain intact, matching Original `PerformMotion`; corruption
panics with the entity/order context. The serialized leading-transition span
is maintained on every front pop and path installation, and startup transition
insertion is reported directly instead of inferred from queue-length deltas.

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
| Movement, animation, ActionChange, scroll Hourglass | actor/object virtual Hourglass and Execute methods | PA-013; EYES_FOLLOW and live SEEK-target mixed pre/post observations are fixed. The supported Soldier/Human/base Actor/PC-or-NPC envelope now shares each live legacy creation slot. Movement, active combat, abilities, riders, scrolls, and other entity kinds remain separate. |
| NPC view, detection, timers, speech, patrol | `RHElementActorNPC::Hourglass` and AI subclasses | The complete supported NPC-derived envelope now nests patrol before Human/base Actor work and inform/view/detection/ambush/deafness plus the busy/ladder/speech/lock-gated suffix afterward. Live per-owner inputs, synchronous FIFO closure, lock/freeze gates, PC-noise ordering, wake effects, and patrol Think-before-direction are regression-covered. Non-NPC discovery and broader entity-owner debt remain open under PA-013. |
| Projectiles, melee, and abilities | per-type virtual Hourglass/Execute methods | live creation order, spawn-frame inclusion, straight/assault causality, non-straight phase timing, and synchronous melee victim ordering are verified by PA-013 regressions; riders and other combat maintenance remain batched |
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
| Melee and damage | in progress, high risk | Retain the straight and non-straight swapped-creation regressions plus lateral/circle phase and victim-FIFO tests; review riders, smalltalk, and every remaining simplification against actor-human combat code. |
| Movement, paths, doors, lifts | in progress, high risk | Retain PA-014/PA-022/PA-030, creation-ordered SEEK, motion-order cache, transition-prefix, and lift-approach regressions; audit remaining door and animation-callback timing. |
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
