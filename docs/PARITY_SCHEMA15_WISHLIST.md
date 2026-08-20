# Parity schema 16 draft wishlist

Schema 15 added actor door-pass/sequence-element state and successful
route-construction events. Schema 16 is an opt-in draft: its recorder and Rust
consumer have landed together, passed targeted smoke captures, and produced
1,500-frame replacement recordings. The draft adds:

Implemented follow-up: successful beggar clicks now emit a
`beggar_dont_talk_stamp` resolved command naming the civilian. This preserves
the post-click cooldown mutation even when `AddInteractionWithSeek` reduces a
non-macro double-click to the otherwise target-free `make_pc_fast` event.

- Failed route attempts with stable failure phase/reason; successful routes also
  expose safe gate kind/activity/geometry, traversal direction, penalties, and
  A* scores.
- Current-order state, following/postponed sequence identities, and
  command-whitelisted movement payloads. Door movement includes the active
  order identity and authored gate in/out geometry.
- AI-forecast input and resolution, including target and selected building
  exit when one is safely available.
- Per-frame event ordinals and explicit phases for route, forecast, GoTo,
  popup, and alert-formation diagnostics.
- `DisplayPopupText` modal entry/exit, background-colorization decision,
  same-frame suppression state, and popup-scoped nested-refresh entry/exit.
- Full alert-formation candidate scan and short-circuit eligibility results,
  accepted and selected ordering, exact contribution/running-average float
  bits, final sector, and the decomposed position/thick-corridor `CanPut`
  sweep for every slot.
- Actor `PositionInterface` collision snapshots (move/blocked boxes,
  anti-collision, deviation, blocked count, and radius) plus `GoTo` input,
  effective flags, phase/outcome, and straight/path authorization results when
  those checks ran. Straight checks also retain the exact query-local source,
  source layer, and move box passed to `IsStraightMovementAutorized`.

Known safe omissions and future work:

- `RHOrder::bDone` is not recorded: the Original copy constructor can leave it
  uninitialized. The motion method is an ephemeral `PerformMotion` argument,
  not stored in the current order. Recording either would require an authored
  lifecycle hook rather than reading dormant memory.
- The building forecast branch with at most one gate does not assign its output
  direction. Schema 16 omits `resolved.direction` for that branch.
- Existing `RHFastFindGrid` authorization APIs return only booleans, so alert
  `CanPut` events set `blocker_ids_available:false` and leave motion/mobile
  blocker IDs null. A future collision-witness API could expose stable blocker
  IDs without repeating or mutating the query.
- Route failure hooks cover the three `RHSequence` route builders. Other raw
  `FindPathGates` callers are not yet event producers.
- Short smoke fixtures positively exercised the general `GoTo` event envelope,
  but not the straight-authorization query branch. They also did not trigger
  popup, route/forecast, or alert events. Record targeted fixtures for those
  paths before treating their event shapes as stable.

Prefer fields with stable engine identities and exact integer/float-bit representations. Do not add pointer values or observational code that advances RNG, queries mutable caches, or changes sequence/pathfinding behavior.

For future capture campaigns, keep schema 14 as the default. Schema 15 remains
available for compatibility, and the schema 16 draft must be selected
explicitly:

```sh
PARITY_TRACE_SCHEMA=16 \
PARITY_RANDOM_REPLAYS=10 \
PARITY_FRAMES=1500 \
original-code/scripts/capture_parity_save_replays.sh
```

This selects `original-code/build/native-full/robin`, passes the requested
schema explicitly, and uses the separate
`parity-save-replays/60s-random-input/schema16` output directory. The producer
rejects a trace whose header does not declare schema 16. Use `DRY_RUN=1` to
inspect resolved paths and settings without creating capture directories or
starting the game. The recorder and consumer have passed targeted schema 16
smokes and replacement captures; use a small targeted pilot for each newly
instrumented branch before beginning a large capture campaign.
