# Next architecture audit — local, uncommitted

Audited 2026-09-14. Main advanced to `935ac150e` during cleanup; the engine
and audited UI runtime files are unchanged from `5aca6058c`. This is a new
audit, separate from the completed migration checklist. No implementation or
new replay validation was performed for the initial audit.

Implementation is committed as `e8a614573` on `direct-execution`, based on
`43b88b2eb`: all four structural cuts are implemented together. The optimized
parity sample passed all 274 recordings / 339,589 frames with exact EOF and zero
divergences. Engine checks passed 4,268 unit tests, 15 integration tests, and 38
doctests; client checks passed 2,189 unit tests and its integration/doc suites.
Save 85 / network 52 / replay 44 are synchronized.

Accepted and fast-forwarded into main at `e8a614573` on 2026-09-15.
The final release runner also passed all 274 recordings / 339,589 frames,
with exact EOF and zero divergences. Post-merge engine checks passed 4,268
unit tests and 15 integration tests; the client passed 2,170 unit tests plus
integration, example, and documentation checks; the protocol passed 102 tests.
The changed client count reflects the integrated replay-submission cleanup.

Release game, matching admission helper, parity runner, checksums, manifest,
and acceptance results are preserved in `target/direct-execution-release/`.
The merged worktree and branch are removed after preserving these artifacts.
The portrait-widget follow-ups below are preexisting gaps outside these four
architectural cuts.

## Big picture

The central AI snapshot map and script-effects interpreter are gone. The next
large pass should remove the remaining mechanisms that turn synchronous calls
into queued instructions: engine-owned methods execute immediately, the sequence
manager owns the graph and the genuinely deferred FIFO, and presentation events
cross the host boundary as outputs. Do not replace these queues with another
generic effects enum or an all-purpose context wrapper.

## Prioritized work

### 1. Remove four obsolete deferred-operation lists together

`crates/robin_engine/src/engine/state/orders.rs:27` owns
`pending_reinforcements`, `pending_scroll_amulets`, `pending_hero_speeches`,
and `pending_hades_kills`. `engine/tick/frame_systems.rs:141` drains them.
The old explanation was unavailable mutable assets/sprite loading. Relevant
callers now receive assets, and spawn sprites are preloaded.

Concrete source-order findings:

- `engine/scroll_reveal.rs:319` copies placement into `PendingScrollAmulet`,
  marks the scroll Taken, and returns the old scroll. Its caller at `:416`
  highlights that identity. `original-code/RHElementScroll.cpp:456` creates
  and adds the amulet first, marks the scroll Taken, and returns the amulet.
  This is an observable identity/order mismatch, not just extra allocation.
- `engine/mod.rs:2692` terminates an instruction then queues hero speech.
  `original-code/RHelementactorpc.cpp:6557` performs the speech immediately
  after termination.
- `engine/tick/mission.rs:253` queues reinforcement creation on countdown
  expiry. `original-code/RHengine.cpp:3736` and `:13000` forward arrival into
  synchronous creation.
- `engine/console_dispatch.rs:1028` queues Hades victims.
  `original-code/RHconsole.cpp:1613` kills, sets Dead, launches Wait, then
  clears selection.

Replace each producer with the actual operation at its deterministic command or
instruction boundary. Remove the lists, payload schema, drains, persistence/hash
fields, and tests that only assert queue contents. Preserve RNG, entity allocation,
callback, and selection order. Estimated net removal: **150–350 lines**.

### 2. Remove the synchronous sequence outbox

`sequence.rs:2718` defines `PendingSyncEntry`; `:2952` stores its queue.
`sequence/dispatch.rs` is 594 lines, `engine/sequence_runtime/immediate.rs`
545, `phase.rs` 879, and `script_sync.rs` 965. These file sizes are the
inspection surface, not a claim that all their code is removable.

The queue mixes executable actions with parked registration iterations. Engine
wrappers drain, detach, restore, and splice continuations to reconstruct nested
calls. `original-code/RHsequence.cpp:273` instead iterates a captured sibling
boundary, reads current elements, and executes WAIT directly;
`original-code/RHsequencemanager.cpp:1057` executes immediate commands before
deciding whether to append normal work to the real FIFO.

Move registration and advancement orchestration onto `EngineInner`; retain graph
storage and `elements_to_go` in `SequenceManager`. Invoke immediate command bodies
directly with narrow owner borrows. Keep real VM suspension and stack-safe Stop
traversal. Estimated net removal: **600–1,200 lines**, excluding command bodies.

### 3. Make simulation message delivery synchronous

`messenger.rs:318` stores a general `VecDeque<Message>`.
`engine/tick/mission.rs:279` drains into another queue, dispatches handlers,
prepends nested emissions, then re-enqueues downstream output. This can only run
nested messages after the emitting handler returns, rather than at the sending
statement.

`original-code/RHMessenger.cpp:174` performs preprocessing, receiver dispatch,
and postprocessing synchronously. Recursive sends at `:761` and `:804–853`
occur inside that flow. Add one engine-owned forwarding path for simulation
messages; leave ordered presentation output at the host boundary. Preserve the
pre/postprocessing semantics, including macro action restoration and selection
notifications. Roughly 15 engine source files are involved. Estimated net removal:
**200–500 lines**, less certain until every message consumer is mapped.

### 4. Collapse the remaining movement phase shell

`engine/movement_step.rs:57` already borrows `&mut EngineInner`, but
`MovementPrepass`, `MovementStepEntry`, copied `SelectedMovementOrder`, and
`MovementCompletion` still reflect the former long mutable entity borrow.
See `engine/movement.rs:2791–2865,5154` and the stale module explanation in
`movement_step.rs`.

Review locals against actual source statement order and collapse phase transport
into ordinary scoped execution. Some values must be sampled at entry; preserving
them is behavior, not gratuitous snapshotting. Estimated net removal:
**200–500 lines**, lower confidence than the first three cuts.

## Implementation and acceptance checklist

- [x] Delete the four deferred lists in one pass; execute at their producers.
- [x] Move synchronous sequence registration/advancement onto the engine and
  delete parked registration/action transport.
- [x] Map all messenger consumers; dispatch simulation messages at send sites.
- [x] Review and simplify movement phase transport against source order.
- [x] Add focused source-behavior regressions for scroll return/highlight identity,
  inline speech, reinforcement creation, Hades selection order, nested messages,
  WAIT siblings, callback cancellation, postponed restamping, and deep Stop.
- [x] Run affected tests, then the existing 274-recording release manifest and any
  newly targeted captures. A passing sample does not establish coverage of rare
  scroll, console, or reinforcement paths.

Start with item 1, then combine items 2 and 3 as one engine-ownership pass. These
are removal estimates, not measured guarantees; shared scaffolding makes the
ranges overlap. The game is now released in early access: preserve save compatibility from
version 96 onward using backward-compatible fields or explicit migrations.
A save-version bump is allowed only when ABSOLUTELY necessary and with explicit
human confirmation. Do not discard save or replay compatibility as an architectural
cleanup shortcut; replay schema and deterministic playback compatibility must be
assessed separately from JSON disk-save compatibility.

## Keep real state and scheduling

### Follow-up: portrait widget availability

- [ ] Model the portrait's explicit action-widget enabled state and connect it to
  HUD/input availability. `ActionAvailable` forwards `MSG_ENABLE_ACTION` /
  `MSG_DISABLE_ACTION`: `original-code/RHgame.cpp:4089–4111` changes widget
  availability, not the PC's gameplay `disabled_actions` arrays. The removed
  Rust messenger had no downstream consumer, so this was already missing.
- [ ] Apply the same UI boundary to entering a building: the PC's temporary
  action mutation already runs directly; the source's separate
  `MSG_DISABLE_ALL_ACTIONS(actor)` must not be replaced with a second gameplay
  mutation against every selected PC.

Keep sequence `elements_to_go`, path requests (queued priority insertion exists in
`original-code/RHpathfinder.cpp:434`), locked-stimulus ordering, actual VM
activations, deterministic sound deadlines, host output events, and rollback
snapshots. Do not classify a queue as removable merely by its name.

## Cleanup and prior acceptance evidence

Seven stale worktrees and twelve local branches were removed. Main and the separate
unmerged `desperados-integration` worktree remain. Recovery bundle, uncommitted
notes/patches, and old validation evidence are under
`.codex-tmp/worktree-cleanup-2026-09-14/`.

The previously verified release game/helper and parity runner now live in
`target/architecture-release/`. `parity-results.json` records 274 recordings,
339,589 frames, zero divergences, and validated EOF for every recording. Those
binaries are the preserved architecture release, not a rebuild of the subsequent
shipping-codec merge.
