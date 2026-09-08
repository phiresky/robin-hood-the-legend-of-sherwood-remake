# Replay save/load projection repair

## Evidence and scope

The original native-client capture under
`/tmp/robin-client-validation-HR7Szn` records a save at replay ordinal 141,
timeline 140, engine hash `53574769c85af73f`, and a load-back at ordinal 608
targeting that marker. Playback matches the checked hashes through ordinal
600; the first subsequent check at 625 is expected `63b47a2cc2cc28ec` but
observed `c68d5e9a031e91bc`. These original artifacts are not edited or relabeled.

The save/load boundaries are aligned: ordinal 608 consumes timeline 140 ->
141 and ordinal 609 consumes 141 -> 142. Two restore-path differences matter:

1. Live loading decoded the payload through `serde_json::Value`. Its sorted
   object keys reorder the `IndexMap`-backed sequence manager: original
   QuickSave keys start `1,2,3,...,10`, but loading produces `1,10,100,...,2`.
   Sequence insertion order is authoritative and participates in engine
   hashing. Comparing JSON values alone hides this change.
2. Replay pinned `Engine::clone()` and restored it directly. Serde deliberately
   omits runtime-only state, but a clone preserves it. The new regression
   demonstrated replay's initialized AI scratch flag remained `true`, while a
   real disk load reset it to `false`; the old hash-only check still passed.

A clone is an appropriate rollback checkpoint, but not a representation of a
loaded save. Both the Value decode and skipped AI scratch fields existed at
review base `a413e4c80`; they were not introduced by repository refactoring.

## Correction

Disk loading still validates header/schema through Value first, but now decodes
the typed payload from the original JSON stream, preserving stored insertion
order. `GameRuntimeSnapshot::capture_serialized` captures engine, sound and
persistent game state through the same typed JSON decode. Replay save markers
use it after checking the recorded engine hash. Encoding/decoding failures are
explicit errors, and campaign-history validation remains mandatory. Restore
still uses the existing validated asset attachment, engine fixups, audio reset,
host reset and game post-load synchronization.

Ordinary snapshot capture, rollback cloning, save formats, replay formats,
executable identity checks, engine hashing and tick ordering are unchanged.
No individual AI cache is reset by hand. Debugger rewind must continue to use
raw checkpoints rather than the save projection.

## Regression coverage

- The existing linear save/load replay regression now writes and reads an
  actual QuickSave file. It runs a normal owner pass before capture and checks
  the AI scratch initialization flag after both restore paths, in addition to
  engine hashes, timeline movement and host/game state. Previously it applied
  the in-memory `GameSaveFile` directly and could not detect the discrepancy.
- A focused projection test seeds clone-only AI continuation state and checks
  that the source and raw snapshot preserve it while both disk and replay
  projections discard it identically. It also compares the restored hashes.
- A populated 12-sequence regression explicitly reproduces the old Value
  decoder's hash change, then checks that disk loading and replay projection
  both preserve the original hash and sequence-manager state. A feature-gated
  `Engine::test_launch_sequence` helper seeds this fixture; the exact facade
  capability allowlist includes it without weakening the visitor.
- An opt-in diagnostic reads the original native QuickSave without modifying
  it, compares the typed and old Value decoding hashes against the recorded
  marker, and checks another typed persistence round trip. Set
  `ROBIN_REPLAY_LOAD_SAVE_FIXTURE` and `ROBIN_REPLAY_LOAD_MARKER_HASH`.

## Validation status

- `CARGO_BUILD_JOBS=1 cargo test -p robin_rs --lib save_ -- --nocapture`:
  **66 passed**, one opt-in diagnostic ignored.
- The opt-in diagnostic passed on the untouched original QuickSave with
  `ROBIN_REPLAY_LOAD_MARKER_HASH=53574769c85af73f`: direct typed decoding and
  repaired disk loading both hash to **53574769c85af73f**, exactly the recorded
  marker. Reproducing the old Value decoder produces **3a2f7bd8daeb1193**.
  The serialized values compare equal; the lost insertion order explains why
  a JSON-value diff did not expose this state difference.
- `CARGO_BUILD_JOBS=1 cargo build -p robin_rs --bin robin --features desktop`:
  passed. Existing unrelated compiler warnings remain.
- `cargo fmt --all`, explicit rustfmt for macro-declared modules, and
  `git diff --check`: passed.
- `CARGO_BUILD_JOBS=1 cargo test -p robin_engine --test engine_facade_contract
  engine_public_mutation_surface_is_an_exact_capability_allowlist`: **1 passed**.
- Focused desktop-feature graphical acceptance **passed**: save ordinal 213 /
  timeline 212 / hash `9dae5d9226933600`, actual load ordinal 680 -> 213,
  exact compact replay completed **1,329 records**, with **all 54 checked
  hashes matching**, including **26 after load-back**. No desync occurred.
  Export response SHA-256:
  `cc7a5f654206422676bed40347523d28e1f8f428cc7df67cd6b657e08d2ae3f2`.
- Final committed-build graphical acceptance **passed**: **1,017 records**,
  **all 41 checked hashes match**, including **20 after load-back**, with no
  desync or panic. Its live save
  marker is ordinal 204 / timeline 203 / hash `ec69b54526277f5e`, and actual
  load is ordinal 520 -> 204. Both live and playback closed cleanly with exit
  code 0; neither required the harness's bounded-stop fallback.
  Frozen export response SHA-256:
  `4ac5462ca8cd7fba0d0cee4f125394d57011cf2ef85a4288e7bb9ae0d3bfe3c1`.

Fresh recording evidence is under `/tmp/robin-replay-load-kIBfPV`, with a
dedicated X display `:110` and isolated save/config/cache directories. Live
and playback use the same built executable (SHA-256
`a688796b3cf63f82a6fc846675cda618b8d52296073bed053407ec423caa6959`).
This desktop-feature binary was built as `ecfdfbb10` plus uncommitted changes,
production-source-equivalent to repair commit `4655dba6c`; it is not claimed
to carry that commit's build identity. A separate final run under
`/tmp/robin-replay-final-gmeOfP` uses committed build `185ea5783` (production
content identical to combined `83f87b1d1`), SHA-256
`4a9df31e4ae89c56d6ba965369b514dd34a328d6dc64b9a31a15966a6a110814`.
That variant has native multiplayer support but no audio; validation starts
ordinary single-player gameplay, not a network session.
The earlier desktop-feature binary lacks the subsequently integrated window
exit fix: its explicit post-export window-close cleanup returned code 1 with
the known event-loop shutdown race. This happened after live export / replay
EOF, not during save/load; the final committed live run instead closed with
code 0. Desktop-feature coverage does not claim physical audio output.
The original replay identity is never changed. The fixture, regressions and
completed focused native workflow prove both the original payload order loss
and the clone-only scratch mismatch are corrected.

## Browser queued replay activation follow-up

The final browser acceptance exposed a separate entry-path omission: the direct
mission loop (used by demo auto-start) handled pause-menu Restart using its old
borrowed launch arguments without consuming an admitted RPC replay. The strict
`replay_init` pending-slot guard correctly rejected that cold construction. The
preserved failing browser evidence is `/tmp/robin-browser-validation-ORDfRC`.

The direct loop now owns its launch arguments and mutable profile catalog. At a
completed mission boundary it consumes a pending replay once, releases the old
resolved asset lease, and uses the existing canonical `prepare_replay_launch`.
Campaign, mission index, location, seed, simulation configuration, explicit RPC
pause policy, and replay restart checkpoint all change together. Invalid replay
preparation returns a contextual error; it never falls back to the old mission.
No pending request leaves ordinary restart/checkpoint behavior unchanged. Seven
`main_entry/run.rs` callers transfer ownership, including dropping the original
direct custom launch arguments so they cannot retain a stale archive lease.

The strict replay-init guard, schema checks, exact asset resolver, engine order,
and replay hash logic remain unchanged. New tests cover same/different mission
selection, recorded metadata, single consumption, old lease destruction,
no-pending ordinary arguments, and contextual rejection. Existing direct-restart
checkpoint tests remain part of the required-state suite. Browser acceptance of
this follow-up must run on the newly combined runtime/helper build; the previous
native quick-load acceptance above does not substitute for that test.

Validation: `CARGO_BUILD_JOBS=1 cargo test -p robin_rs --lib
game_session::required_state_tests -- --nocapture` passed all 21 tests (including
the four new pending-direct tests). `cargo fmt --all`, explicit formatting of
the macro-included entry modules, and `git diff --check` passed.
