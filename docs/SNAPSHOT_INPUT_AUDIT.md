# Snapshot-input ownership audit

Audited 2026-07-19. Scope: `Engine::apply_commands`,
`EngineInner::perform_hourglass`, and simulation-affecting callees reachable
from those boundaries. Original anchors are `RHEngine::PerformHourglass`
(`original-code/RHengine.cpp:3446-3777`), camera/display work
(`RHengine.cpp:4170-4501,6758-6913`), zoom-message dispatch
(`RHengine.cpp:12162-12229`), and engine serialization
(`RHengine.cpp:2408-2514`).

## Classification

| Input/read family | Classification | Result |
| --- | --- | --- |
| `SimulationContext`, frame/clock gates, RNG, entities, orders, scripts, campaign, AI and feedback runtime | Deterministic Engine state | Already owned by the nine serialized `EngineInner` domains. |
| Shared script/director camera transform, zoom-initialized/mechanized flags, follow displacement/countdown and pending zoom anchor | Deterministic Engine state | Fixed: all future-affecting values are serialized and hashed. The write-only `old_view_position` / `old_zoom_factor` presentation copies remain excluded. |
| `PerformHourglass` zoom suspension | Deterministic Engine state | Fixed: reads `feedback.cutscene_camera.display`; the rollback bridge that copied flags into `HostDisplayState` was removed. |
| Minimap drag focus and minimap-click camera target | Deterministic command input where it produces Engine output | Fixed: `MinimapMouse*` records `continuing_drag` / resolved `center_on`; apply no longer derives messenger or camera mutations from host widget scratch. Invalid recorded map targets fail with level-bound context. |
| Profiles, static grid/path graph, authored scripts/bindings, entity attachment tables, sprite scripts, duration tables, hiking paths and sight geometry | Immutable `LevelAssets` | Read-only after mission construction. Snapshot adoption preflights and reattaches the level-dependent runtime data; missing required attachments already return contextual `SnapshotRestoreError`s. |
| Minimap transitions/highlights/geometry, QA shift/blink animation, drag boxes, mouse-button/focus latches | Genuine host presentation/input | May be read or written only to produce host presentation. Minimap profile-position output is explicitly omitted from serialization/hash. Box selection coordinates and held-button decisions that affect gameplay arrive resolved on `PlayerCommand`. |
| `DevState::projectile_cheat_rain` drain | Genuine developer state | The shipped Original's projectile-rain body is unimplemented here; the tick only clears the dev trigger and does not branch Engine state on its value. |
| `web_time::Instant` hourglass profiler | Process-local instrumentation | Timing is logged only; it cannot affect Engine state, command order, RNG or returned game code. |

## Violations fixed

1. Host `background_transform.zoom_to_{up,down}` could suppress a tick. A
   default rollback display therefore advanced gameplay unless a bespoke
   two-boolean mirror ran before every replay frame.
2. `zoom_init_done`, `mechanized_zoom`, follow-camera displacement/countdown,
   and the pending zoom anchor were serde/hash-skipped even though camera work
   consumes them to mutate the snapshotted view or terminate a script sequence.
3. Minimap widget state decided whether `UiHasFocus` entered the Engine
   messenger and whether a click recentered the Engine camera. Rollback starts
   with host display scratch, so the same recorded command could produce a
   different Engine hash.
4. Host-only minimap profile persistence rode inside hashed `SideEffects`, so
   a presentation-only dirty flag could perturb pre-tick multiplayer hashes.

The snapshot shape and `PlayerCommand` wire shape changed. Current-only schema
versions are save 51, replay 8 and network 14; older formats are intentionally
unsupported.

## Remaining uncertain cases

- The Rust port deliberately separates the local `Host::viewport` from the
  shared script/director camera. Minimap clicks currently retain the existing
  behavior of issuing a deterministic shared-camera target plus locker-off;
  whether local multiplayer minimap recentering should instead affect only
  `Host::viewport` needs an explicit split-screen policy and is not an
  ownership leak after the target is command-derived.
- `pending_zoom_mouse_screen` has no live producer today; it is retained as
  Engine state because `InitZoom` consumes it and tests/tools can construct a
  pending request. A future local-view zoom cleanup may remove it from the
  shared camera rather than populate it from ambient host input.
- Host-only minimap and macro animations still execute alongside command/tick
  dispatch for Original ordering. They are permitted only while remaining
  one-way presentation outputs; any future gameplay consumer must move the
  observed value into an Engine owner or a resolved command.
