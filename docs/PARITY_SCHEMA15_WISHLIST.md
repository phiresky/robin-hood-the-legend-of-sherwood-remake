# Parity schema 15 follow-up wishlist

Schema 15 adds actor door-pass/sequence-element state and successful route-construction events. Keep these candidates for later additions when a parity investigation demonstrates their value:

Implemented follow-up: successful beggar clicks now emit a
`beggar_dont_talk_stamp` resolved command naming the civilian. This preserves
the post-click cooldown mutation even when `AddInteractionWithSeek` reduces a
non-macro double-click to the otherwise target-free `make_pc_fast` event.

- Failed route-construction attempts, including the exact rejection stage/reason.
- The full current order (ID, action, motion method, goal, completion flags), not only the sequence element's remaining order count.
- Postponed and following sequence-element identities, to expose hand-off timing without dumping entire sequence graphs.
- The AI forecast input and resolved destination before route construction, including target identity and building-exit selection.
- Door-pass phase and authored in/out positions in addition to gate identity and traversal direction.
- Command-specific movement payloads (destination, sector/layer, flags, tolerance, speed) only after documenting which fields are initialized for every constructor of that command; dormant C++ fields must not be read by instrumentation.
- Per-event simulation phase/ordinal when several routes or state transitions occur in one frame.
- `DisplayPopupText` modal phase and nested-refresh diagnostics. `RHMenuPopupScroll::NeededBkgndColorization` can conditionally trigger a nested `Refresh`, while its static same-frame suppression changes later behavior; record the popup decision, colorization result, refresh nesting/phase, and suppression frame for presentation/RNG investigations.
- Alert-formation selection and `CanPut` diagnostics. Record the officer ID; every scanned candidate ID in scan order; inactive and script-locked status; the exact eligibility rejection stage (`rank`, `able_to_help`, `stay_on_post`, `can_call`, `radius`, or `Think`); accepted IDs in order; exact normalized-contribution and running-average float bits; and the final selected sector. Then record each `CanPut` candidate direction and slot destination box, position-authorization result, thick-corridor result, and stable IDs for blocking motion/mobile lines. `CanPut` alone misses omitted candidates, while schema 12's direction goal alone cannot explain either selection or live slot rejection.
- Actor PositionInterface collision state: exact move box, anti-collision enabled state, deviated flag, blocked count, box-blocked state, and radius. Also capture the last `GoTo` destination/flags and its straight/path authorization result when available; a rejected straight move can otherwise surface only as later panic-RNG drift.

Prefer fields with stable engine identities and exact integer/float-bit representations. Do not add pointer values or observational code that advances RNG, queries mutable caches, or changes sequence/pathfinding behavior.

For future capture campaigns, keep schema 14 as the default and opt into the
new recorder explicitly:

```sh
PARITY_TRACE_SCHEMA=15 \
PARITY_RANDOM_REPLAYS=10 \
PARITY_FRAMES=1500 \
original-code/scripts/capture_parity_save_replays.sh
```

This selects `original-code/build/native-full/robin` and the separate
`parity-save-replays/60s-random-input/schema15` output directory. The producer
rejects a trace whose header does not declare schema 15. Use `DRY_RUN=1` to
inspect resolved paths and settings without creating capture directories or
starting the game.
