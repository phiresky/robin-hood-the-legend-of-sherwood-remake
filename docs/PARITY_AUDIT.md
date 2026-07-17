# Gameplay Parity Audit

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
| PA-014 | Rust drained all pending path/move work in one tick. | `RHEngine::ProcessPathRequests`, `RHengine.cpp`, invokes the pathfinder once and resolves at most one queued request per frame. | `7786df3bf` restores frame-paced processing with queue-order, snapshot, and malformed-request tests. |
| PA-015 | Missing sound/exclamation durations silently fabricated 75-frame completions that could shift AI talk events. | `RHSound::GetSampleLengthMs` returns zero when its cache lookup cannot resolve/load the sample; actor talk completion follows that duration. | `f50d86169` uses decoded metadata and the Original zero-length missing-sample result, warns instead of inventing a duration, and adds focused timing tests. |
| PA-016 | Rust's NPC subphases did not match `RHElementActorNPC::Hourglass`. | `RHElementActorNPC::Hourglass`, `RHelementactornpc.cpp`. | `48d6d6aa7` restores the actor-owned patrol/human/detection/busy/timer/stimulus order and adds an exact phase trace. |
| PA-020 | Arrow-watching AI treated `EVENT_DONE` as a return-to-duty signal. | Arrow-watching substate cases in `RHartificialmalignity.cpp`. | `a8dcaaf01` removes the invented fallback and adds a focused regression. |
| PA-021 | Script native `Sees` omitted ambiance radius adjustment and the forest Royalist 180-degree rule. | `RHScript::Sees`, `RHScript.cpp`, delegates to `RHElementActorNPC::IsDetecting` in `RHelementactornpc.cpp`. | `6a9ec3b11` routes the native through matching NPC visibility rules and adds focused coverage. |
| PA-024 | `SetPersistentProperty` dropped live PC ammo writes when no campaign existed. | `RHScript::SetPersistentProperty`, `RHScript.cpp`, updates the live actor for arrows and PC ammo properties. | `7029583d6` updates live ammo independently of campaign persistence and tests the no-campaign path. |
| PA-026 | Shoulder-ceiling checks ran for every `CarryingOnShoulders` posture. | `RHElementActorPC::Execute`, `RHelementactorpc.cpp`, gates the check on `WALKING_CARRYING_ON_SHOULDERS`. | `93f0436` adds the action gate and focused regression coverage. |
| PA-027 | Messenger, condolence-card, and recursive self-stimulus work was deferred across global phases. | `RHMessenger::ForwardMessage`, `RHMessenger.cpp`; `RHSequenceElement::SetState`, `RHsequenceelement.cpp`. | `6f7907eaf` restores depth-first same-frame re-entry and tests recursive messages, self-stimuli, cascade ordering, and cross-owner cards. |
| PA-029 | Mission `PostInitialize` ran inside the first engine tick before the host refresh/sound boundary. | `RHGame::GameLoop`, `RHgame.cpp`, invokes it after refresh and sound. | `cdcf5d0fe` moves it to an explicit post-refresh stage shared by live play and replay, with an exact one-shot frame-boundary test. |

## Open Findings

Priority reflects likely gameplay impact, not implementation effort.

| ID | Priority | Status | Finding and evidence |
| --- | --- | --- | --- |
| PA-013 | High | unverified | Rust globally regroups per-entity Hourglass work into movement, animation, script, detection, combat, and ability passes. Original `RHEngine::PerformHourglass` calls each virtual `Element::Hourglass` in entity order before `RHSequenceManager::Hourglass` (`RHengine.cpp`). Cross-entity and same-frame callback ordering needs scenario tests. |
| PA-022 | Medium | incomplete | Cached door authorization checks only building type, active state, and villain lock. Original `FindDoorEnemyCouldBeBehind` calls `RHGate::IsActorAutorized`, which also checks building capacity and riders (`RHartificialmalignity.cpp`, `RHGate.cpp`). |
| PA-023 | Medium | mismatch | `SetExperiences` writes the campaign description and persists into later missions. Original `RHScript::SetExperiences` changes only the live PC capacities (`RHScript.cpp`). |
| PA-025 | Medium | mismatch | Charly-to-officer logic substitutes 360-degree detection. Original calls normal `IsDetecting(mpAntagonist)` and therefore respects the view cone (`RHartificialmalignity.cpp`). |
| PA-028 | Medium | unverified | Rust dispatches script `SendMessage` after the script call instead of launching the original `RHCOMMAND_SEND_MESSAGE` sequence element. Compare arbitration and same-frame callback order (`RHScript.cpp`, `RHsequenceelement.cpp`). |
| PA-030 | Low | unverified | Collinear movement-line intersection fabricates impact parameter `t = 0.5`. Find and port the original earliest-overlap behavior or add geometry evidence and focused collision tests. |
| PA-031 | Low | mismatch | Push handling falls back to radial movement for unexpected thrust kinds. Original push dispatch handles the three supported kinds and asserts otherwise (`RHelementactorhuman.cpp`). |
| PA-032 | High | mismatch | WAIT-priority sequence elements are appended to `elements_to_go` and do not dispatch until the manager hourglass. Original `RHSequence::Launch` calls `Go()` for WAIT elements before launch returns (`RHsequence.cpp`). Add a launch-return ordering test before changing the barrier. |
| PA-033 | High | mismatch | `SimSnapshot` excludes `HostDisplayState` and replay constructs a default display, although `EngineInner::perform_hourglass_inner` reads zoom-transition flags to decide whether gameplay phases run. Original `RHEngine::Serialize` persists `mbackgroundTransform` (`RHengine.cpp`). Move the gate input into deterministic state or prove it is derived, then test replay from a non-default zoom transition. |
| PA-034 | High | incomplete | Spellforge Lua is a post-original feature, not the original SCB VM. Only Initialize/PostInitialize are dispatched; Lua state is absent from snapshots, required startup failure continues without Lua, event errors are discarded, and Initialize can call the sim-RNG shim without an installed Engine RNG scope (`lua_session.rs`, `game_session/mod.rs`, `robin_lua/state.rs`). Reject it in deterministic modes until a versioned Spellforge contract and state policy exist. |
| PA-035 | High | incomplete | A requested replay whose header cannot be decoded warns and chooses the multiplayer seed or zero before Engine construction (`game_session/setup.rs`). Replay is a Rust extension with no Original equivalent; its deterministic contract requires a fatal preload error, never an invented seed. |
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
| Sequence cleanup and path processing | `RHEngine::PerformHourglass`; `RHEngine::ProcessPathRequests` | frame pacing verified; retain queue-order regression |
| Entity refresh and sequence dispatch | virtual `RHElement::Hourglass`; `RHSequenceManager::Hourglass` | PA-013 |
| Movement, animation, ActionChange, scroll Hourglass | actor/object virtual Hourglass and Execute methods | PA-013; test entity-order observations |
| NPC view, detection, timers, speech, patrol | `RHElementActorNPC::Hourglass` and AI subclasses | phase order verified by exact trace; continue state-specific review |
| Arrows, purse/coins, wasps, nets, melee, abilities | per-type virtual Hourglass/Execute methods | verify spawn-frame inclusion and ordering per type |
| Titbits, deselection, anonymous timers | tail of `RHEngine::PerformHourglass` | structurally verified; titbit display-order approximation is visual |
| Condolations and self-stimuli | `RHSequenceElement::SetState` to actor `SendCondolationCard` | synchronous depth-first ordering verified by focused traces |
| PostInitialize | mission loop in `RHgame.cpp` | post-refresh boundary verified by focused trace |
| RNG, rollback side effects, minimap/marks/camera | no single original phase | intentional architecture only where documented; audit gameplay state individually |

## Coverage Matrix

`queued` means the scanner has produced candidates but no systematic
source-to-source pass is complete.

| Subsystem | State | Required evidence |
| --- | --- | --- |
| Main tick and mission state | in progress | Resolve PA-013; retain path, NPC phase, and PostInitialize ordering tests. |
| Item interaction / pickup | verified for explicit Take | Keep the no-proximity-pickup regression. Audit other interaction shortcuts. |
| Enemy detection and state machine | in progress, high risk | Resolve PA-025; retain the NPC phase trace and continue state-by-state comparison. |
| Melee and damage | queued, high risk | Review every remaining simplification against actor-human combat code. |
| Movement, paths, doors, lifts | in progress, high risk | Resolve PA-022 and PA-030; retain frame-paced path tests. |
| Script natives and callbacks | in progress, high risk | Resolve PA-023 and PA-028; retain `Sees` and live-ammo regressions. |
| Sequence manager and messages | queued, high risk | Resolve PA-032 and add arbitration replay tests for PA-028; retain synchronous re-entry traces. |
| Projectiles and abilities | queued | Per-type Hourglass and spawn-frame comparison. |
| Audio-driven AI state | verified for missing-duration handling | Retain metadata and missing-required-duration tests; audit remaining sound callbacks. |
| Deterministic snapshots and replay | in progress, high risk | Resolve PA-033 and PA-035; compare live/replayed zoom and fatal replay preload behavior. |
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
