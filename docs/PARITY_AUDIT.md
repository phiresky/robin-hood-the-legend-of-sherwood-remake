# Gameplay parity audit

## `PerformHourglass` phase ordering

The Rust tick is mechanically decomposed in
`crates/robin_engine/src/engine/tick.rs` without reordering the statements that
were in `perform_hourglass_inner`. The emitted `HourglassPhase` trace is the
ordering contract for that Rust pipeline:

1. `DeferredEffectsStart`
2. `MissionAndMessages`
3. `NpcOrders`
4. `Entities`
5. `Paths`
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

### Unresolved parity

- **Paths:** Rust constructs paths synchronously while dispatching Move/Seek
  sequence elements. Its `Paths` phase contains failed-path retry,
  moving-target seek refresh, and actor wait-timer maintenance, and currently
  follows `Entities`. This does not reproduce the original asynchronous
  `ProcessPathRequests -> CheckForCollision -> entity Hourglass` interleaving.
  TODO: use replay/state-hash evidence before moving any of these calls.
- **Entities:** `Entities::occupied_mut()` walks stable table slots, matching
  creation order for the append-only runtime table. TODO: verify imported save
  games and every level loader preserve creation order in slot order; do not
  add a fallback sort key without an original-data source.
- **NPCs:** Original NPC AI/detection was reached inside each NPC element's
  creation-ordered Hourglass (for example
  `original-code/RHelementactornpc.cpp:3495-3614`). Rust has both an `NpcOrders`
  pre-pass and a batched `Npcs` pass after sequences/entity systems. TODO:
  characterize cross-NPC cases where batching changes same-frame visibility.
- **Entity systems:** Movement, animation, projectile, melee, and ability work
  is batched by system in Rust, whereas the original invoked subtype
  hourglasses from the creation-ordered element loop. TODO: map observable
  cross-entity interleavings with original replays before consolidating or
  moving phase boundaries.
- **Deferred effects:** Condolation, re-entrant self-stimulus,
  `PostInitialize`, and immediate-action drains are Rust determinism/borrowing
  splits without exact one-to-one original calls. They remain in
  `DeferredEffectsEnd`, after all gameplay phases, and are explicitly traced so
  future parity changes cannot move them silently.
