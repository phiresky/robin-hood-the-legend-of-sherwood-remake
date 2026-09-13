# AI scheduling simplification audit

## Scope and evidence

Read-only source audit of the scheduling architecture, September 2026. No
scheduling changes or parity runs were performed. `original-code/` was absent
from the audit worktree: descriptions of Original behavior below rely on Rust
source annotations and existing regression tests, not independent C++ review.
These are candidates for investigation, not parity-validated deletions.

## Production already runs complete owner slots

`crates/robin_engine/src/engine/tick.rs`, in
`tick_actor_animation_action_change_slots_with_hooks`, walks live entities in
`original_creation_order`. `tick_actor_owner_envelopes_with_owner_hook` runs the
actor's preludes, movement Execute, human tail, and NPC detection/timer tail
before advancing to the next owner. Supported nonactors also execute at their
own slots. The coordinator preserves removal compaction, including the skipped
shifted successor, and callback-created entities joining the tail.

Consequently, comments describing globally batched actor movement in
`engine/tick/frame_systems.rs::hourglass_phase_entity_systems` and
`engine/ai/tick_data.rs::position_at_owner_boundary` are stale. Earlier owners
have already moved; later owners naturally have not. The frame-start geometry
snapshot is a remnant of the older schedule, not evidence that the current
schedule requires historical geometry.

All abbreviated source paths below are under `crates/robin_engine/src/`.

## Candidate removals

| Mechanism | Source | Proposed replacement |
| --- | --- | --- |
| Whole-frame `positions_before_movement` | `engine/tick/frame_systems.rs` | Read authoritative geometry at the existing owner slot. |
| Creation-order / before-actor position rewind | `engine/ai/tick_data.rs::boundary_position` | Live reads at the actual consuming phase. |
| All-entity geometry overlay | `engine/ai/tick_data.rs::build_owner_context_scratch_at_slot_without_forecast` | Targeted reads for the calculation being performed. |
| EYES_FOLLOW rewind | `engine/ai/event_dispatch.rs::refresh_npc_view_for_npc` | Live target world coordinates; also removes its inconsistent use of raw entity indices for ordering. |
| Carried-human position rewind | `engine/ai/snapshots.rs::tick_enemy_ai_build_human_object_targets_for_npc` | Live body coordinates after carrier-owned synchronization, preserving the body's own obstacle/elevation. |
| Forecast-input coordinate correction | `engine/ai/detection.rs::prepare_detection_forecasts_for_owner` | Extract live inputs at the original query point. Retain forecast semantics. |
| Fighter, camp, and target geometry repair | `engine/ai/detection.rs::apply_owner_relative_tick_positions` | Query the particular live records the decision consumes. |
| `owner_boundary_positions` and callback coordinate-diff reconciliation | `engine/script.rs`, ChangeWay continuation | Synchronous caller resumes and reads live state after callback return. Preserve genuine pre-callback locals. |

A concrete risk in the current rewind is an earlier owner's callback moving a
later actor: creation-order projection can replace that legitimate mutation
with frame-start coordinates even though movement is no longer batched.

`PreparedNpcOwnerPass.world` and `PreparedAiEntityViewCache` in
`engine/ai/mod.rs` are Rust extraction/performance caches. Their comments explain
that broad tactical extraction was retained to avoid quadratic rebuilding, and
the tactical cache is invalidated after PC noise refresh. They are not, by
themselves, evidence of Original cached gameplay state. Replace broad projections
incrementally with small engine-owned queries; rebuilding every entity on every
callback would trade complexity for avoidable cost.

## Snapshot presence also controls scheduling

Do **not** simply pass `None` to remove position rewinding.
`engine/ai/detection.rs::tick_enemy_ai_refresh_detection` uses
`positions_before_movement.is_some()` to enable inform/view work and the complete
NPC post-detection tail. Absence identifies a historical detection-only test
seam. Split production owner orchestration from that seam before removing this
argument, or the apparent geometry cleanup will silently skip gameplay.

## Original state and timing that must remain

- NPC view parameters refresh at their original phase. A callback is not a
  reason to refresh the observer's view a second time.
- The viewer/frame radius cache has ground and projection-obstacle entries.
  `tick_enemy_ai_refresh_detection` explicitly preserves this Original cache
  across detectable buckets and commits it before queued Think dispatch.
- Detection memories, cadence, and FIFO survive. The contiguous scan precedes
  queued Think processing; SHADOW, Enemy VIEW/OUTOFVIEW, BODY, OBJECT, FRIEND,
  MISSED_FRIEND, and BEGGAR ordering is significant.
- Pre-acoustic eligibility decisions are latched before synchronous EVENT_HEAR.
  Rechecking after its callback changes the original control flow.
- PC produced-noise records refresh during that PC's human tail. Other owners
  must see the record appropriate to their position in the walk.
- AI Position access through a selected door can differ from literal map/world
  geometry used for detection. Preserve the accessor distinction and exact
  stored world coordinates rather than reconstructing them from map points.
- Door/lift/building destination prediction and its RNG consumption point are
  required behavior. Preparing alternatives must not draw random exit choices
  before the original consumer does.
- Entry-selected orders, recursive completion behavior, and scalar values
  genuinely read before callbacks remain meaningful local state.
- Removal compaction, skipped successors, and callback-created tail owners are
  deliberate behavior of the live Original element walk.

## Suggested sequence

1. Complete the synchronous patrol-arrival refactor independently, retaining
   geometry context still needed by its current callers.
2. Separate production NPC owner orchestration from detection-only test drivers,
   without changing geometry.
3. Replace creation-order rewinds with live reads at existing owner phases;
   delete `positions_before_movement` plumbing after its last consumer is gone.
4. Convert further artificial continuations to synchronous operations and remove
   their frozen world overlays and callback mutation reconciliation.
5. Replace broad tactical/entity projections with targeted live queries while
   retaining Original caches and true pre-callback locals.

Each step needs unchanged `original_parity_replay` comparison through full EOF,
plus focused regressions. No scheduling simplification is established as safe by
this source audit. TODO: verify the cited timing directly against C++ when the
Original tree is available, and run the full replay corpus for each change.

## Regression scenarios

- Moving followed target before versus after the observer, including canonical
  creation order differing from Rust entity indices.
- Earlier callback teleports a later actor or moves the observer before its
  patrol prelude; a callback changes selected door without raw-coordinate change.
- Carrier before/after observer with a body on a different obstacle/elevation.
- EVENT_HEAR changes target position, posture, order, or viewer eligibility before
  optical work.
- PC noise refresh before/after an NPC, and frozen state changed by an earlier
  owner.
- Callback removes a followed target or creates a target absent from frame-start
  snapshots; removal compaction and same-frame spawned tail execution.
- Multiple same-frame detection calls share the proper Original radius cache
  and consume the exact RNG stream.
- Patrol-path callback changes the route and a subsequent statement reads its
  new path or geometry.

Existing starting points include
`engine/tests/world_entity/movement.rs` tests
`owner_boundary_positions_follow_original_creation_order_not_entity_slots` and
`optical_ai_position_uses_carrier_boundary_but_detects_the_target_world_point`,
and `engine/tests/ai_detection/perception.rs` tests for PC noise, swapped creation
order, friend state, hearing-before-optics, and live order/posture observations.
Some focused tests simulate the obsolete batched schedule: retain their asserted
Original behavior by exercising the production owner walk rather than preserving
the old snapshot API as a requirement.
