# Completed architecture: mission runtime and lifecycle

Status: completed on 2026-07-19. `run_mission` and
`run_mission_headless` are construction/finish wrappers around explicit mission
owners and frame methods. This is no longer an implementation plan.

## Result

```text
run_mission / run_mission_headless
  bootstrap a loaded mission
  run the selected frontend owner
  consume the mission and return campaign + next simulation state

MissionRuntime
  MissionWorld       Host, Game, EngineManager, LevelAssets, DevState
  TimelineRuntime    replay, rewind, rollback, MP history and frame clock
  MissionControl     pause/step and mission-local host controls

Interactive frontend
  MissionInput / MissionAudio / MissionResources
  MissionUi / MissionPresentation

HeadlessMission
  MissionRuntime + explicit HeadlessPolicy

MissionFrame
  one iteration's commands, replay/modal inputs and recorder lifecycle
```

The implementation is split across `game_session/bootstrap.rs`, `runtime.rs`,
`interactive.rs`, `headless.rs`, `flow.rs`, `frame_prepare.rs`,
`frame_simulate.rs`, `event_hud.rs`, `live_gameplay.rs`, `debriefing.rs` and
`terminal_debriefing.rs`. `game_session/mod.rs` remains the public session
boundary and owns only the outer construction/finish flow.

## Ownership rules

- The campaign moves by value into the active mission and returns by value on
  every controlled exit.
- `MissionWorld` owns loaded gameplay/host state. GPU, input, UI and native
  audio resources are owned only by the interactive frontend.
- `TimelineRuntime` owns deterministic history and transport policy; it does
  not absorb Engine, renderer or menu responsibilities.
- `MissionFrame` makes recorder begin/end and per-frame command ownership
  explicit and prevents state leaking between iterations.
- Headless execution uses an explicit policy and does not construct fake
  renderer, input, menu or audio objects.

## Lifecycle order that must remain observable

The interactive path preserves:

1. network/snapshot admission and input/menu collection;
2. operation and save/load processing;
3. final command injection and Engine tick;
4. timeline commit, script UI and terminal/debriefing work;
5. recorder, app effects and sound hourglass;
6. render/present and cleanup;
7. one-shot `PostInitialize` after the first refresh/sound boundary;
8. state-hash transmission and pacing.

The Original anchor is `RHGame::GameLoop` in
`original-code/RHgame.cpp:1399-1842`. In particular, `PostInitialize` is host
lifecycle work and must not move into `EngineInner::perform_hourglass`.

The headless path shares deterministic setup, tick, timeline commit and
post-initialize work, but keeps its explicit modal, replay-completion, pacing
and multiplayer-start policy differences.

## Exit and persistence contract

- All controlled exits converge on consuming mission finalization.
- Save/load/restart operations carry exact decoded payload, campaign, mission,
  seed and `SimConfig` data; explicit slots never silently fall back to
  Continue.
- Required state fails contextually. No exit path fabricates a campaign,
  mission ID, snapshot, renderer or frontend.
- Historical save/replay compatibility is intentionally unsupported.

## Remaining local cleanup

No new mission mega-struct or large `run_mission` rewrite is planned. Focused
helpers may be split when their domain changes. The current known local seam is
making `MissionWorld` fields private after the remaining frame operations move
onto their focused owners. True-headless multiplayer start-barrier support
also remains an explicit parity TODO.
